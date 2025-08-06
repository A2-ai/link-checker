use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Instant;

use clap::{Arg, Command};
use cookie::Cookie;
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::cookie::Jar;
use reqwest::Url;
use scraper::{Html, Selector};
use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug)]
enum Error {
    #[error("request error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("bad http response: {0}")]
    BadResponse(String),
}

#[derive(Debug)]
struct CrawlCommand {
    url: Url,
    extract_links: bool,
}

fn create_secure_http_only_cookie(
    name: &str,
    value: &str,
    domain: &str,
) -> cookie::CookieBuilder<'static> {
    Cookie::build((name.to_owned(), value.to_owned()))
        .http_only(true)
        .secure(true)
        .path("/")
        .domain(domain.to_owned())
}

fn visit_page(client: &Client, command: &CrawlCommand) -> Result<Vec<Url>, Error> {
    println!("Checking {:#}", command.url);
    let response = client.get(command.url.clone()).send()?;
    if !response.status().is_success() {
        return Err(Error::BadResponse(response.status().to_string()));
    }

    let mut link_urls = Vec::new();
    if !command.extract_links {
        return Ok(link_urls);
    }

    let base_url = response.url().to_owned();
    let body_text = response.text()?;
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
    Ok(link_urls)
}

struct CrawlState {
    domain: String,
    visited_pages: HashSet<String>,
}

impl CrawlState {
    fn new(start_url: &Url) -> CrawlState {
        let mut visited_pages = HashSet::new();
        visited_pages.insert(start_url.as_str().to_string());
        CrawlState {
            domain: start_url.domain().unwrap().to_string(),
            visited_pages,
        }
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
    /// been visited.
    fn mark_visited(&mut self, url: &Url) -> bool {
        self.visited_pages.insert(url.to_string())
    }
}

#[derive(Debug)]
struct FoundUrls {
    url: Url,
    links: Vec<Url>,
}

type CrawlResult = Result<FoundUrls, (Url, Error)>;
fn spawn_crawler_threads(
    command_receiver: mpsc::Receiver<CrawlCommand>,
    result_sender: mpsc::Sender<CrawlResult>,
    thread_count: u32,
    domain: String,
    cookie_value: String,
) {
    let command_receiver = Arc::new(Mutex::new(command_receiver));

    for _ in 0..thread_count {
        let result_sender = result_sender.clone();
        let command_receiver = command_receiver.clone();
        let domain = domain.clone();
        let cookie_value = cookie_value.clone();
        thread::spawn(move || {
            let jar = Jar::default();
            let cookie =
                create_secure_http_only_cookie("AWSELBAuthSessionCookie-0", &cookie_value, &domain);
            let url = format!("https://{}", domain).parse::<Url>().unwrap();
            jar.add_cookie_str(&cookie.to_string(), &url);
            let client = ClientBuilder::new()
                .cookie_provider(Arc::new(jar))
                .build()
                .unwrap();
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
                        url: crawl_command.url,
                        links: link_urls,

                    }),
                    Err(error) => Err((crawl_command.url, error)),
                };
                result_sender.send(crawl_result).unwrap();
            }
        });
    }
}

#[derive(Serialize)]
struct UrlResults {
    bad_urls: Vec<String>,
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
                        let crawl_command = CrawlCommand { url, extract_links };
                        command_sender.send(crawl_command).unwrap();
                        pending_urls += 1;
                    }
                }
            }
            Err((url, error)) => {
                bad_urls.push(url.clone().to_string());
                println!("Got crawling error: {:#} for URL {:#}", error, &url);
                continue;
            }
        }
    }
    UrlResults {
        bad_urls,
        url_map: url_map,
    }
}

fn check_links(start_url: Url, domain: String, cookie_value: String) -> UrlResults {
    let (result_sender, result_receiver) = mpsc::channel::<CrawlResult>();
    let (command_sender, command_receiver) = mpsc::channel::<CrawlCommand>();
    spawn_crawler_threads(command_receiver, result_sender, 8, domain, cookie_value);
    control_crawl(start_url, command_sender, result_receiver)
}

fn main() {
    dotenvy::dotenv().ok(); 
    let matches = Command::new("crawler")
        .arg(
            Arg::new("domain")
                .long("domain")
                .value_name("DOMAIN")
                .required(true)
                .env("RBQM_LINK_CHECKER_DOMAIN")
                .help("The domain to crawl"),
        )
        .arg(
            Arg::new("cookie_value")
                .long("cookie-value")
                .value_name("COOKIE_VALUE")
                .required(true)
                .env("RBQM_LINK_CHECKER_COOKIE_VALUE")
                .help("The value of the AWSELBAuthSessionCookie-0 cookie"),
        )
        .get_matches();

    let domain = matches.get_one::<String>("domain").unwrap().to_string();
    let cookie_value = matches
        .get_one::<String>("cookie_value")
        .unwrap()
        .to_string();

    let start_url = Url::parse(&format!("https://{}", domain)).unwrap();
    let start_time = Instant::now();
    let url_results = check_links(start_url, domain, cookie_value);

    let bad_urls_file = File::create("bad_urls.json").unwrap();
    serde_json::to_writer_pretty(bad_urls_file, &url_results.bad_urls).unwrap();

    let url_map_file = File::create("url_map.json").unwrap();
    serde_json::to_writer_pretty(url_map_file, &url_results.url_map).unwrap();

    println!("Bad URLs: {:#?}", url_results.bad_urls);
    dbg!(url_results.url_map);
    println!("Crawling took {:#?}", start_time.elapsed());
}
