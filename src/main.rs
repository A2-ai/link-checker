use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use ureq::{Agent, ResponseExt};
use url::Url;
use scraper::{Html, Selector};
use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug)]
enum Error {
    #[error("request error: {0}")]
    UreqError(#[from] ureq::Error),
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("bad http response: {0}")]
    BadResponse(String),
}

#[derive(Debug)]
struct CrawlCommand {
    url: Url,
    extract_links: bool,
    source_page: Option<Url>,
}


fn visit_page(client: &Agent, command: &CrawlCommand) -> Result<Vec<Url>, Error> {
    println!("Checking {:#}", command.url);
    
    // Retry logic for 5xx errors with exponential backoff
    let mut attempts = 0;
    let max_retries = 3;
    loop {
        let result = client.get(command.url.as_str()).call();
        match result {
            Ok(mut response) => {
                let mut link_urls = Vec::new();
                if !command.extract_links {
                    return Ok(link_urls);
                }

                // Use the final URL after redirects
                let base_url_str = response.get_uri().to_string();
                let base_url = Url::parse(&base_url_str).unwrap_or_else(|_| command.url.clone());
                let body_text = response.body_mut().read_to_string()?;
                let start_time = Instant::now();
                let document = Html::parse_document(&body_text);

                let selector = Selector::parse("a").unwrap();
                let href_values = document
                    .select(&selector)
                    .filter_map(|element| element.value().attr("href"));
                for href in href_values {
                    match base_url.join(href) {
                        Ok(link_url) => {
                            link_urls.push(link_url);
                        }
                        Err(err) => {
                            println!("On {base_url:#}: ignored unparsable {href:?}: {err}");
                        }
                    }
                }
                println!(
                    "Parsed {:#?} and found {:#?} URLs in {:#?}",
                    command.url.to_string(),
                    link_urls.len(),
                    start_time.elapsed()
                );
                return Ok(link_urls);
            }
            Err(e) => {
                match e {
                    ureq::Error::StatusCode(status) => {
                        if (500..=599).contains(&status) && attempts < max_retries {
                            attempts += 1;
                            let delay = Duration::from_millis(100 * (2_u64.pow(attempts - 1)));
                            println!(
                                "Got 5xx error for {:#}, retrying in {:#?} (attempt {}/{})",
                                command.url, delay, attempts, max_retries
                            );
                            thread::sleep(delay);
                            continue;
                        }
                        return Err(Error::BadResponse(status.to_string()));
                    }
                    other => return Err(Error::UreqError(other)),
                }
            }
        }
    }

}

struct CrawlState {
    domain: String,
    visited_pages: HashSet<String>,
}

impl CrawlState {
    fn new(start_url: &Url) -> CrawlState {
        let mut visited_pages = HashSet::new();
        let normalized_url = Self::normalize_url(start_url);
        visited_pages.insert(normalized_url);
        CrawlState {
            domain: start_url.domain().unwrap().to_string(),
            visited_pages,
        }
    }

    /// Remove the fragment (hash) part of a URL to avoid treating
    /// page.html#section1 and page.html#section2 as different pages
    fn normalize_url(url: &Url) -> String {
        let mut normalized = url.clone();
        normalized.set_fragment(None);
        normalized.to_string()
    }

    /// Determine whether links within the given page should be extracted.
    /// we only want to continue extracting links for pages within the same
    /// domain.
    fn should_extract_links(&self, url: &Url) -> bool {
        let Some(url_domain) = url.domain() else {
            return false;
        };
        url_domain == self.domain
    }

    /// Mark the given page as visited, returning false if it had already
    /// been visited. Uses normalized URL (without fragment) for comparison.
    fn mark_visited(&mut self, url: &Url) -> bool {
        let normalized_url = Self::normalize_url(url);
        self.visited_pages.insert(normalized_url)
    }
}

#[derive(Debug)]
struct FoundUrls {
    url: Url,
    links: Vec<Url>,
}

type CrawlResult = Result<FoundUrls, (CrawlCommand, Error)>;
fn spawn_crawler_threads(
    command_receiver: mpsc::Receiver<CrawlCommand>,
    result_sender: mpsc::Sender<CrawlResult>,
    thread_count: u32,
) {
    let command_receiver = Arc::new(Mutex::new(command_receiver));

    for _ in 0..thread_count {
        let result_sender = result_sender.clone();
        let command_receiver = command_receiver.clone();
        thread::spawn(move || {
            let client = Agent::new_with_defaults();
            loop {
                let command_result = {
                    let receiver_guard = command_receiver.lock().unwrap();
                    receiver_guard.recv()
                };
                let Ok(crawl_command) = command_result else {
                    // The sender got dropped. No more commands coming in.
                    break;
                };
                let crawl_result = match visit_page(&client, &crawl_command) {
                    Ok(link_urls) => Ok(FoundUrls {
                        url: crawl_command.url.clone(),
                        links: link_urls,

                    }),
                    Err(error) => Err((crawl_command, error)),
                };
                result_sender.send(crawl_result).unwrap();
            }
        });
    }
}

#[derive(Serialize)]
struct BadUrl {
    url: String,
    found_on: Option<String>,
}

#[derive(Serialize)]
struct UrlResults {
    bad_urls: Vec<BadUrl>,
    url_map: HashMap<String, Vec<String>>,
}
fn control_crawl(
    start_url: Url,
    command_sender: mpsc::Sender<CrawlCommand>,
    result_receiver: mpsc::Receiver<CrawlResult>,
) -> UrlResults {
    let mut crawl_state = CrawlState::new(&start_url);
    let start_command = CrawlCommand {
        url: start_url,
        extract_links: true,
        source_page: None,
    };
    command_sender.send(start_command).unwrap();
    let mut pending_urls = 1;

    let mut bad_urls = Vec::new();
    let mut url_map = HashMap::new();
    while pending_urls > 0 {
        let crawl_result = result_receiver.recv().unwrap();
        pending_urls -= 1;

        match crawl_result {
            Ok(found_urls) => {
                url_map.insert(found_urls.url.clone().to_string(), found_urls.links.clone().iter().map(|url| url.to_string()).collect());
                for url in found_urls.links {
                    if crawl_state.mark_visited(&url) {
                        let extract_links = crawl_state.should_extract_links(&url);
                        let crawl_command = CrawlCommand { 
                            url, 
                            extract_links,
                            source_page: Some(found_urls.url.clone()),
                        };
                        command_sender.send(crawl_command).unwrap();
                        pending_urls += 1;
                    }
                }
            }
            Err((crawl_command, error)) => {
                let bad_url = BadUrl {
                    url: crawl_command.url.to_string(),
                    found_on: crawl_command.source_page.map(|u| u.to_string()),
                };
                bad_urls.push(bad_url);
                println!("Got crawling error: {:#} for URL {:#}", error, &crawl_command.url);
                continue;
            }
        }
    }
    UrlResults {
        bad_urls,
        url_map: url_map,
    }
}

fn check_links(start_url: Url) -> UrlResults {
    let (result_sender, result_receiver) = mpsc::channel::<CrawlResult>();
    let (command_sender, command_receiver) = mpsc::channel::<CrawlCommand>();
    spawn_crawler_threads(command_receiver, result_sender, 8);
    control_crawl(start_url, command_sender, result_receiver)
}

#[derive(Parser)]
#[command(name = "link-checker")]
#[command(about = "A tool to check the validity of links on a website")]
#[command(version)]
struct Args {
    /// The URL to start crawling from
    #[arg(long, short)]
    url: String,
}

fn main() {
    let args = Args::parse();
    let start_url = Url::parse(&args.url).expect("Invalid URL provided");
    let start_time = Instant::now();
    let url_results = check_links(start_url);

    let bad_urls_file = File::create("bad_urls.json").unwrap();
    serde_json::to_writer_pretty(bad_urls_file, &url_results.bad_urls).unwrap();

    let url_map_file = File::create("url_map.json").unwrap();
    serde_json::to_writer_pretty(url_map_file, &url_results.url_map).unwrap();

    // Calculate summary statistics
    let pages_crawled = url_results.url_map.len();
    let mut all_unique_urls = std::collections::HashSet::new();
    
    // Add all pages that were crawled
    for page_url in url_results.url_map.keys() {
        all_unique_urls.insert(page_url.clone());
    }
    
    // Add all links found on those pages
    for links in url_results.url_map.values() {
        for link in links {
            all_unique_urls.insert(link.clone());
        }
    }
    
    let total_unique_urls = all_unique_urls.len();
    let broken_links_count = url_results.bad_urls.len();

    // Print summary
    print!("Crawled {} pages, checked {} unique URLs", pages_crawled, total_unique_urls);
    if broken_links_count == 0 {
        println!(", found no broken links.");
    } else if broken_links_count == 1 {
        println!(", found 1 broken link.");
    } else {
        println!(", found {} broken links.", broken_links_count);
    }

    // Show broken links if 20 or fewer, otherwise refer to file
    if broken_links_count > 0 {
        if broken_links_count <= 20 {
            println!("\nBroken links:");
            for bad_url in &url_results.bad_urls {
                if let Some(source) = &bad_url.found_on {
                    println!("  - {} (found on: {})", bad_url.url, source);
                } else {
                    println!("  - {} (starting URL)", bad_url.url);
                }
            }
        } else {
            println!("\nSee bad_urls.json for the complete list of broken links.");
        }
    }

    println!("\nCrawling completed in {:#?}", start_time.elapsed());
}
