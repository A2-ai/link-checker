use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use ureq::{Agent, ResponseExt};
use url::Url;
use scraper::{Html, Selector};
use serde::Serialize;
use thiserror::Error;
use regex::Regex;

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


fn is_absolute_url(href: &str) -> bool {
    // Check if URL has a scheme (protocol) like http://, https://, ftp://, mailto:, etc.
    href.contains("://") || href.starts_with("mailto:") || href.starts_with("tel:")
}

fn is_likely_html_content(url: &Url) -> bool {
    let path = url.path().to_lowercase();
    
    // If no extension or ends with /, assume it's HTML
    if path.is_empty() || path.ends_with('/') {
        return true;
    }
    
    // Check for HTML-like extensions
    if let Some(extension) = path.split('.').last() {
        matches!(extension, 
            "html" | "htm" | "php" | "asp" | "aspx" | "jsp" | "cfm" | "cgi" | "pl" | "py" | "rb"
        )
    } else {
        // No extension, likely HTML
        true
    }
}

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn visit_page(client: &Agent, command: &CrawlCommand, total_bytes: &Arc<AtomicUsize>) -> Result<Vec<Url>, Error> {
    println!("Checking {:#}", command.url);
    
    // For non-HTML content, just do a HEAD request to check availability
    let use_head_request = !command.extract_links && !is_likely_html_content(&command.url);
    
    // Retry logic for 5xx errors with exponential backoff
    let mut attempts = 0;
    let max_retries = 3;
    let mut should_upgrade_to_get = false;
    
    loop {
        let result = if use_head_request && !should_upgrade_to_get {
            client.head(command.url.as_str()).call()
        } else {
            client.get(command.url.as_str()).call()
        };
        match result {
            Ok(mut response) => {
                let mut link_urls = Vec::new();
                
                if use_head_request && !should_upgrade_to_get {
                    // For HEAD requests, check if the content type indicates HTML
                    let headers = response.headers();
                    let content_type = headers.get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("unknown");
                    let content_length = headers.get("content-length")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<usize>().ok());
                    
                    // If this is HTML content, we need to make a GET request to extract links
                    if content_type.starts_with("text/html") {
                        if let Some(size) = content_length {
                            println!("  → HEAD request: {} bytes ({}, content-type: {}) - upgrading to GET for HTML parsing", 
                                    size, format_bytes(size), content_type);
                        } else {
                            println!("  → HEAD request successful (content-type: {}) - upgrading to GET for HTML parsing", content_type);
                        }
                        // Make a GET request instead
                        should_upgrade_to_get = true;
                        continue;
                    } else {
                        // Non-HTML content, just report and return
                        if let Some(size) = content_length {
                            println!("  → HEAD request: {} bytes ({}, content-type: {})", 
                                    size, format_bytes(size), content_type);
                        } else {
                            println!("  → HEAD request successful (content-type: {})", content_type);
                        }
                        return Ok(link_urls);
                    }
                }
                
                // If this wasn't an upgrade from HEAD to GET, and we're not supposed to extract links, return early
                if !command.extract_links && !should_upgrade_to_get {
                    return Ok(link_urls);
                }

                // Use the final URL after redirects
                let page_url_str = response.get_uri().to_string();
                let page_url = Url::parse(&page_url_str).unwrap_or_else(|_| command.url.clone());
                
                // Get headers before reading body to avoid borrow checker issues
                let content_type = {
                    let headers = response.headers();
                    headers.get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("unknown")
                        .to_string()
                };
                let content_length = {
                    let headers = response.headers();
                    headers.get("content-length")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<usize>().ok())
                };
                
                let body_text = response.body_mut().read_to_string()?;
                let actual_size = body_text.len();
                
                // Track total bytes downloaded
                total_bytes.fetch_add(actual_size, Ordering::Relaxed);
                
                // Report document size for bandwidth tracking
                if let Some(declared_size) = content_length {
                    if declared_size == actual_size {
                        println!("  → Downloaded {} bytes ({})", actual_size, format_bytes(actual_size));
                    } else {
                        println!("  → Downloaded {} bytes ({}, declared: {})", 
                                actual_size, format_bytes(actual_size), format_bytes(declared_size));
                    }
                } else {
                    println!("  → Downloaded {} bytes ({})", actual_size, format_bytes(actual_size));
                }
                
                // Only parse HTML content for links
                if !content_type.starts_with("text/html") && !is_likely_html_content(&page_url) {
                    println!("  → Skipping link extraction (content-type: {})", content_type);
                    return Ok(link_urls);
                }
                let start_time = Instant::now();
                let document = Html::parse_document(&body_text);

                // Check for <base href="..."> element to determine the base URL for relative links
                let base_url = {
                    let base_selector = Selector::parse("base[href]").unwrap();
                    if let Some(base_element) = document.select(&base_selector).next() {
                        if let Some(base_href) = base_element.value().attr("href") {
                            match page_url.join(base_href) {
                                Ok(resolved_base) => resolved_base,
                                Err(err) => {
                                    println!("On {page_url:#}: ignored invalid base href {base_href:?}: {err}");
                                    page_url.clone()
                                }
                            }
                        } else {
                            page_url.clone()
                        }
                    } else {
                        page_url.clone()
                    }
                };

                let selector = Selector::parse("a").unwrap();
                let href_values = document
                    .select(&selector)
                    .filter_map(|element| element.value().attr("href"));
                for href in href_values {
                    // Only follow relative URLs - skip absolute URLs
                    if is_absolute_url(href) {
                        continue;
                    }
                    
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
    start_url_path: String,
    domain_match: bool,
    visited_pages: HashSet<String>,
}

impl CrawlState {
    fn new(start_url: &Url, domain_match: bool) -> CrawlState {
        let mut visited_pages = HashSet::new();
        let normalized_url = Self::normalize_url(start_url);
        visited_pages.insert(normalized_url);
        CrawlState {
            domain: start_url.domain().unwrap().to_string(),
            start_url_path: start_url.path().to_string(),
            domain_match,
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

    /// Determine whether a URL should be visited at all.
    /// By default, only visit URLs with the same path prefix.
    /// If domain_match is enabled, visit any URL in the same domain.
    fn should_visit_url(&self, url: &Url) -> bool {
        let Some(url_domain) = url.domain() else {
            return false;
        };
        
        // Must be in the same domain
        if url_domain != self.domain {
            return false;
        }
        
        // If domain_match is enabled, any URL in the domain is allowed
        if self.domain_match {
            return true;
        }
        
        // Otherwise, check if the URL path starts with the same prefix as the start URL
        url.path().starts_with(&self.start_url_path)
    }

    /// Determine whether links within the given page should be extracted.
    /// By default, only extract links from pages with the same path prefix.
    /// If domain_match is enabled, extract from any page in the same domain.
    fn should_extract_links(&self, url: &Url) -> bool {
        // Use the same logic as should_visit_url for now
        self.should_visit_url(url)
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

fn print_summary_and_save(url_results: &UrlResults, start_time: Instant, interrupted: bool, total_bytes_downloaded: usize) {
    // Save the results to files
    if let Ok(bad_urls_file) = File::create("bad_urls.json") {
        let _ = serde_json::to_writer_pretty(bad_urls_file, &url_results.bad_urls);
    }

    if let Ok(url_map_file) = File::create("url_map.json") {
        let _ = serde_json::to_writer_pretty(url_map_file, &url_results.url_map);
    }

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
    if interrupted {
        print!("\n\nCrawl interrupted! ");
    }
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

    println!("Total data downloaded: {} bytes ({})", total_bytes_downloaded, format_bytes(total_bytes_downloaded));

    if interrupted {
        println!("\nResults saved to bad_urls.json and url_map.json");
        println!("Crawling interrupted after {:#?}", start_time.elapsed());
    } else {
        println!("\nCrawling completed in {:#?}", start_time.elapsed());
    }
}

fn spawn_crawler_threads(
    command_receiver: mpsc::Receiver<CrawlCommand>,
    result_sender: mpsc::Sender<CrawlResult>,
    thread_count: u32,
    shutdown_flag: Arc<AtomicBool>,
    total_bytes: Arc<AtomicUsize>,
) {
    let command_receiver = Arc::new(Mutex::new(command_receiver));

    for _ in 0..thread_count {
        let result_sender = result_sender.clone();
        let command_receiver = command_receiver.clone();
        let shutdown_flag = shutdown_flag.clone();
        let total_bytes = total_bytes.clone();
        thread::spawn(move || {
            let client = Agent::new_with_defaults();
            loop {
                // Check if we should shutdown
                if shutdown_flag.load(Ordering::Relaxed) {
                    break;
                }
                
                let command_result = {
                    let receiver_guard = command_receiver.lock().unwrap();
                    receiver_guard.recv_timeout(Duration::from_millis(100))
                };
                let Ok(crawl_command) = command_result else {
                    // Either timeout or sender got dropped
                    if shutdown_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    continue;
                };
                
                // Check again before processing
                if shutdown_flag.load(Ordering::Relaxed) {
                    break;
                }
                
                let crawl_result = match visit_page(&client, &crawl_command, &total_bytes) {
                    Ok(link_urls) => Ok(FoundUrls {
                        url: crawl_command.url.clone(),
                        links: link_urls,

                    }),
                    Err(error) => Err((crawl_command, error)),
                };
                if !shutdown_flag.load(Ordering::Relaxed) {
                    let _ = result_sender.send(crawl_result);
                }
            }
        });
    }
}

fn control_crawl(
    start_url: Url,
    command_sender: mpsc::Sender<CrawlCommand>,
    result_receiver: mpsc::Receiver<CrawlResult>,
    shutdown_flag: Arc<AtomicBool>,
    total_bytes: Arc<AtomicUsize>,
    domain_match: bool,
    skip_pattern: Option<&Regex>,
) -> (UrlResults, usize) {
    let mut crawl_state = CrawlState::new(&start_url, domain_match);
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
        // Check if we should shutdown
        if shutdown_flag.load(Ordering::Relaxed) {
            println!("\nShutdown requested, finishing current requests...");
            break;
        }
        
        let crawl_result = match result_receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => result,
            Err(_) => {
                // Timeout, check shutdown flag again
                continue;
            }
        };
        pending_urls -= 1;

        match crawl_result {
            Ok(found_urls) => {
                url_map.insert(found_urls.url.clone().to_string(), found_urls.links.clone().iter().map(|url| url.to_string()).collect());
                for url in found_urls.links {
                    if shutdown_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    // First check if we should visit this URL at all
                    if !crawl_state.should_visit_url(&url) {
                        continue;
                    }
                    
                    if crawl_state.mark_visited(&url) {
                        let should_extract = crawl_state.should_extract_links(&url);
                        // Only extract links from HTML content if we're in the same domain
                        let extract_links = should_extract && is_likely_html_content(&url);
                        
                        let crawl_command = CrawlCommand { 
                            url, 
                            extract_links,
                            source_page: Some(found_urls.url.clone()),
                        };
                        if command_sender.send(crawl_command).is_ok() {
                            pending_urls += 1;
                        }
                    }
                }
            }
            Err((crawl_command, error)) => {
                let url_string = crawl_command.url.to_string();
                
                // Check if this URL should be skipped
                if let Some(regex) = skip_pattern {
                    if regex.is_match(&url_string) {
                        println!("Skipping broken link (matches skip pattern): {:#}", url_string);
                        continue;
                    }
                }
                
                let bad_url = BadUrl {
                    url: url_string,
                    found_on: crawl_command.source_page.map(|u| u.to_string()),
                };
                bad_urls.push(bad_url);
                println!("Got crawling error: {:#} for URL {:#}", error, &crawl_command.url);
                continue;
            }
        }
    }
    let bytes_downloaded = total_bytes.load(Ordering::Relaxed);
    (UrlResults {
        bad_urls,
        url_map: url_map,
    }, bytes_downloaded)
}

fn check_links(start_url: Url, shutdown_flag: Arc<AtomicBool>, domain_match: bool, skip_pattern: Option<&Regex>) -> (UrlResults, usize) {
    let (result_sender, result_receiver) = mpsc::channel::<CrawlResult>();
    let (command_sender, command_receiver) = mpsc::channel::<CrawlCommand>();
    let total_bytes = Arc::new(AtomicUsize::new(0));
    spawn_crawler_threads(command_receiver, result_sender, 8, shutdown_flag.clone(), total_bytes.clone());
    control_crawl(start_url, command_sender, result_receiver, shutdown_flag, total_bytes, domain_match, skip_pattern)
}

#[derive(Parser)]
#[command(name = "link-checker")]
#[command(about = "A tool to check the validity of links on a website")]
#[command(version)]
struct Args {
    /// The URL to start crawling from
    #[arg(long, short)]
    url: String,
    
    /// Enable domain-wide crawling (default: only crawl URLs with the same path prefix)
    #[arg(long, help = "Crawl all URLs within the same domain, not just those with matching path prefix")]
    domain_match: bool,
    
    /// Skip broken links matching this regex pattern
    #[arg(long)]
    skip: Option<String>,
}

fn main() {
    let args = Args::parse();
    let start_url = Url::parse(&args.url).expect("Invalid URL provided");
    
    let skip_regex = args.skip.as_ref().map(|pattern| {
        Regex::new(pattern).expect("Invalid regex pattern provided")
    });
    
    let start_time = Instant::now();
    
    // Set up shutdown flag for graceful interruption handling
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = shutdown_flag.clone();
    
    // Set up Ctrl+C handler
    ctrlc::set_handler(move || {
        println!("\nReceived interrupt signal (Ctrl+C)...");
        shutdown_flag_clone.store(true, Ordering::Relaxed);
    }).expect("Error setting Ctrl+C handler");
    
    let (url_results, total_bytes_downloaded) = check_links(start_url, shutdown_flag.clone(), args.domain_match, skip_regex.as_ref());
    let interrupted = shutdown_flag.load(Ordering::Relaxed);
    
    print_summary_and_save(&url_results, start_time, interrupted, total_bytes_downloaded);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_html_with_base_href(base_href: &str, links: &[&str]) -> String {
        let link_elements: String = links
            .iter()
            .map(|link| format!(r#"<a href="{}">{}</a>"#, link, link))
            .collect::<Vec<_>>()
            .join("\n");
        
        format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <base href="{}">
</head>
<body>
    {}
</body>
</html>"#,
            base_href, link_elements
        )
    }

    fn create_test_html_without_base_href(links: &[&str]) -> String {
        let link_elements: String = links
            .iter()
            .map(|link| format!(r#"<a href="{}">{}</a>"#, link, link))
            .collect::<Vec<_>>()
            .join("\n");
        
        format!(
            r#"<!DOCTYPE html>
<html>
<body>
    {}
</body>
</html>"#,
            link_elements
        )
    }

    fn extract_links_from_html(html: &str, page_url: &str) -> Vec<Url> {
        let document = Html::parse_document(html);
        let page_url = Url::parse(page_url).unwrap();
        
        // Extract base href logic from visit_page function
        let base_url = {
            let base_selector = Selector::parse("base[href]").unwrap();
            if let Some(base_element) = document.select(&base_selector).next() {
                if let Some(base_href) = base_element.value().attr("href") {
                    match page_url.join(base_href) {
                        Ok(resolved_base) => resolved_base,
                        Err(_) => page_url.clone(),
                    }
                } else {
                    page_url.clone()
                }
            } else {
                page_url.clone()
            }
        };

        let selector = Selector::parse("a").unwrap();
        let mut link_urls = Vec::new();
        let href_values = document
            .select(&selector)
            .filter_map(|element| element.value().attr("href"));
        
        for href in href_values {
            // Only follow relative URLs - skip absolute URLs
            if is_absolute_url(href) {
                continue;
            }
            
            if let Ok(link_url) = base_url.join(href) {
                link_urls.push(link_url);
            }
        }
        
        link_urls
    }

    #[test]
    fn test_base_href_absolute_url() {
        let html = create_test_html_with_base_href(
            "https://example.com/subdir/",
            &["page1.html", "page2.html", "/absolute.html"]
        );
        
        let links = extract_links_from_html(&html, "https://original.com/");
        
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].as_str(), "https://example.com/subdir/page1.html");
        assert_eq!(links[1].as_str(), "https://example.com/subdir/page2.html");
        assert_eq!(links[2].as_str(), "https://example.com/absolute.html");
    }

    #[test]
    fn test_base_href_relative_to_page() {
        let html = create_test_html_with_base_href(
            "subdir/",
            &["page1.html", "../other.html"]
        );
        
        let links = extract_links_from_html(&html, "https://example.com/current/");
        
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].as_str(), "https://example.com/current/subdir/page1.html");
        assert_eq!(links[1].as_str(), "https://example.com/current/other.html");
    }

    #[test]
    fn test_no_base_href_uses_page_url() {
        let html = create_test_html_without_base_href(&["page1.html", "/absolute.html"]);
        
        let links = extract_links_from_html(&html, "https://example.com/current/");
        
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].as_str(), "https://example.com/current/page1.html");
        assert_eq!(links[1].as_str(), "https://example.com/absolute.html");
    }

    #[test]
    fn test_base_href_with_different_protocol() {
        let html = create_test_html_with_base_href(
            "ftp://files.example.com/",
            &["file1.txt", "dir/file2.txt"]
        );
        
        let links = extract_links_from_html(&html, "https://example.com/");
        
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].as_str(), "ftp://files.example.com/file1.txt");
        assert_eq!(links[1].as_str(), "ftp://files.example.com/dir/file2.txt");
    }

    #[test]
    fn test_base_href_with_absolute_links() {
        let html = create_test_html_with_base_href(
            "https://base.example.com/",
            &["relative.html", "https://external.com/absolute.html", "mailto:test@example.com"]
        );
        
        let links = extract_links_from_html(&html, "https://original.example.com/");
        
        // Only relative URLs should be followed, absolute URLs are filtered out
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].as_str(), "https://base.example.com/relative.html");
    }

    #[test]
    fn test_invalid_base_href_falls_back_to_page_url() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <base href=":::invalid-url:::">
</head>
<body>
    <a href="page1.html">Page 1</a>
</body>
</html>"#;
        
        let links = extract_links_from_html(html, "https://example.com/current/");
        
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].as_str(), "https://example.com/current/page1.html");
    }

    #[test]
    fn test_base_href_with_fragment_and_query() {
        let html = create_test_html_with_base_href(
            "https://example.com/subdir/?param=value#section",
            &["page1.html", "?other=param"]
        );
        
        let links = extract_links_from_html(&html, "https://original.com/");
        
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].as_str(), "https://example.com/subdir/page1.html");
        assert_eq!(links[1].as_str(), "https://example.com/subdir/?other=param");
    }

    #[test]
    fn test_multiple_base_href_elements_uses_first() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <base href="https://first.example.com/">
    <base href="https://second.example.com/">
</head>
<body>
    <a href="page.html">Page</a>
</body>
</html>"#;
        
        let links = extract_links_from_html(html, "https://original.com/");
        
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].as_str(), "https://first.example.com/page.html");
    }
}
