use std::collections::HashSet;
use url::Url;

pub fn is_absolute_url(href: &str) -> bool {
    // Check if URL has a scheme (protocol) like http://, https://, ftp://, mailto:, etc.
    href.contains("://") || href.starts_with("mailto:") || href.starts_with("tel:")
}

pub fn is_likely_html_content(url: &Url) -> bool {
    let path = url.path().to_lowercase();
    
    // If no extension or ends with /, assume it's HTML
    if path.is_empty() || path.ends_with('/') {
        return true;
    }
    
    // Check if the path contains a dot (indicating a file extension)
    if let Some(last_segment) = path.split('/').last() {
        if last_segment.contains('.') {
            // Has an extension, check if it's HTML-like
            if let Some(extension) = last_segment.split('.').last() {
                matches!(extension, 
                    "html" | "htm" | "php" | "asp" | "aspx" | "jsp" | "cfm" | "cgi" | "pl" | "py" | "rb"
                )
            } else {
                false
            }
        } else {
            // No extension in the last segment, likely HTML (like /api/v1/users or /page)
            true
        }
    } else {
        // Empty path, likely HTML
        true
    }
}

pub fn format_bytes(bytes: usize) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{} B", bytes)
    }
}

pub struct CrawlState {
    domain: String,
    start_url_path: String,
    domain_match: bool,
    visited_pages: HashSet<String>,
}

impl CrawlState {
    pub fn new(start_url: &Url, domain_match: bool) -> CrawlState {
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
    pub fn normalize_url(url: &Url) -> String {
        let mut normalized = url.clone();
        normalized.set_fragment(None);
        normalized.to_string()
    }

    /// Determine whether a URL should be visited at all.
    /// By default, only visit URLs with the same path prefix.
    /// If domain_match is enabled, visit any URL in the same domain.
    pub fn should_visit_url(&self, url: &Url) -> bool {
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
    pub fn should_extract_links(&self, url: &Url) -> bool {
        // Use the same logic as should_visit_url for now
        self.should_visit_url(url)
    }

    /// Mark the given page as visited, returning false if it had already
    /// been visited. Uses normalized URL (without fragment) for comparison.
    pub fn mark_visited(&mut self, url: &Url) -> bool {
        let normalized_url = Self::normalize_url(url);
        self.visited_pages.insert(normalized_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== URL Matching Logic Tests =====

    #[test]
    fn test_crawl_state_path_prefix_matching() {
        let start_url = Url::parse("https://example.com/products/widgets/").unwrap();
        let crawl_state = CrawlState::new(&start_url, false); // domain_match = false
        
        // URLs that should match (same path prefix)
        let url1 = Url::parse("https://example.com/products/widgets/item1").unwrap();
        let url2 = Url::parse("https://example.com/products/widgets/specs.html").unwrap();
        let url3 = Url::parse("https://example.com/products/widgets/").unwrap();
        
        assert!(crawl_state.should_visit_url(&url1));
        assert!(crawl_state.should_visit_url(&url2));
        assert!(crawl_state.should_visit_url(&url3));
        assert!(crawl_state.should_extract_links(&url1));
        assert!(crawl_state.should_extract_links(&url2));
        assert!(crawl_state.should_extract_links(&url3));
        
        // URLs that should not match (different path prefix)
        let url4 = Url::parse("https://example.com/products/gadgets/item2").unwrap();
        let url5 = Url::parse("https://example.com/about/").unwrap();
        let url6 = Url::parse("https://example.com/").unwrap();
        
        assert!(!crawl_state.should_visit_url(&url4));
        assert!(!crawl_state.should_visit_url(&url5));
        assert!(!crawl_state.should_visit_url(&url6));
        assert!(!crawl_state.should_extract_links(&url4));
        assert!(!crawl_state.should_extract_links(&url5));
        assert!(!crawl_state.should_extract_links(&url6));
    }

    #[test]
    fn test_crawl_state_domain_matching() {
        let start_url = Url::parse("https://example.com/products/widgets/").unwrap();
        let crawl_state = CrawlState::new(&start_url, true); // domain_match = true
        
        // URLs in same domain should match regardless of path
        let url1 = Url::parse("https://example.com/products/widgets/item1").unwrap();
        let url2 = Url::parse("https://example.com/products/gadgets/item2").unwrap();
        let url3 = Url::parse("https://example.com/about/").unwrap();
        let url4 = Url::parse("https://example.com/").unwrap();
        
        assert!(crawl_state.should_visit_url(&url1));
        assert!(crawl_state.should_visit_url(&url2));
        assert!(crawl_state.should_visit_url(&url3));
        assert!(crawl_state.should_visit_url(&url4));
        assert!(crawl_state.should_extract_links(&url1));
        assert!(crawl_state.should_extract_links(&url2));
        assert!(crawl_state.should_extract_links(&url3));
        assert!(crawl_state.should_extract_links(&url4));
        
        // URLs in different domain should not match
        let url5 = Url::parse("https://other.com/products/widgets/").unwrap();
        
        assert!(!crawl_state.should_visit_url(&url5));
        assert!(!crawl_state.should_extract_links(&url5));
    }

    #[test]
    fn test_crawl_state_different_domains_rejected() {
        let start_url = Url::parse("https://example.com/products/").unwrap();
        let crawl_state_path = CrawlState::new(&start_url, false);
        let crawl_state_domain = CrawlState::new(&start_url, true);
        
        let different_domain_url = Url::parse("https://other.com/products/").unwrap();
        
        // Both path and domain matching should reject different domains
        assert!(!crawl_state_path.should_visit_url(&different_domain_url));
        assert!(!crawl_state_domain.should_visit_url(&different_domain_url));
    }

    #[test]
    fn test_normalize_url_removes_fragments() {
        let url1 = Url::parse("https://example.com/page.html#section1").unwrap();
        let url2 = Url::parse("https://example.com/page.html#section2").unwrap();
        let url3 = Url::parse("https://example.com/page.html").unwrap();
        
        assert_eq!(CrawlState::normalize_url(&url1), "https://example.com/page.html");
        assert_eq!(CrawlState::normalize_url(&url2), "https://example.com/page.html");
        assert_eq!(CrawlState::normalize_url(&url3), "https://example.com/page.html");
    }

    #[test]
    fn test_mark_visited() {
        let start_url = Url::parse("https://example.com/").unwrap();
        let mut crawl_state = CrawlState::new(&start_url, false);
        
        let url = Url::parse("https://example.com/page.html").unwrap();
        
        // First visit should return true
        assert!(crawl_state.mark_visited(&url));
        
        // Second visit should return false
        assert!(!crawl_state.mark_visited(&url));
        
        // URLs with different fragments should be considered the same
        let url_with_fragment = Url::parse("https://example.com/page.html#section").unwrap();
        assert!(!crawl_state.mark_visited(&url_with_fragment));
    }

    // ===== HTML Content Detection Tests =====

    #[test]
    fn test_is_likely_html_content() {
        // URLs that should be detected as HTML
        let html_urls = vec![
            "https://example.com/",
            "https://example.com/dir/",
            "https://example.com/page.html",
            "https://example.com/page.htm",
            "https://example.com/page.php",
            "https://example.com/page.asp",
        ];
        
        for url_str in html_urls {
            let url = Url::parse(url_str).unwrap();
            assert!(is_likely_html_content(&url), "Should detect {} as HTML", url_str);
        }
        
        // URLs that should NOT be detected as HTML
        let non_html_urls = vec![
            "https://example.com/image.jpg",
            "https://example.com/document.pdf",
            "https://example.com/archive.zip",
            "https://example.com/script.js",
            "https://example.com/style.css",
            "https://example.com/data.json",
        ];
        
        for url_str in non_html_urls {
            let url = Url::parse(url_str).unwrap();
            assert!(!is_likely_html_content(&url), "Should NOT detect {} as HTML", url_str);
        }
    }

    #[test]
    fn test_is_absolute_url() {
        // Absolute URLs
        let absolute_urls = vec![
            "https://example.com/page",
            "http://example.com/page",
            "ftp://files.example.com/file",
            "mailto:user@example.com",
            "tel:+1234567890",
        ];
        
        for url in absolute_urls {
            assert!(is_absolute_url(url), "Should detect {} as absolute", url);
        }
        
        // Relative URLs
        let relative_urls = vec![
            "/page.html",
            "page.html",
            "../page.html",
            "./page.html",
            "?query=value",
            "#fragment",
        ];
        
        for url in relative_urls {
            assert!(!is_absolute_url(url), "Should NOT detect {} as absolute", url);
        }
    }

    // ===== Format Bytes Tests =====

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1048576), "1.0 MB");
        assert_eq!(format_bytes(2621440), "2.5 MB");
    }

    #[test]
    fn test_crawl_state_with_root_path() {
        // Test behavior when starting from root path
        let start_url = Url::parse("https://example.com/").unwrap();
        let crawl_state = CrawlState::new(&start_url, false);
        
        // All URLs under the domain should match in path prefix mode starting from root
        let test_urls = vec![
            "https://example.com/",
            "https://example.com/page.html",
            "https://example.com/dir/",
            "https://example.com/dir/subpage.html",
            "https://example.com/products/",
            "https://example.com/about/contact.html",
        ];
        
        for url_str in test_urls {
            let url = Url::parse(url_str).unwrap();
            assert!(
                crawl_state.should_visit_url(&url),
                "URL {} should be visited when starting from root",
                url_str
            );
        }
        
        // Different domains should not match
        let external_url = Url::parse("https://other.com/").unwrap();
        assert!(!crawl_state.should_visit_url(&external_url));
    }

    #[test]
    fn test_normalize_url_edge_cases() {
        let test_cases = vec![
            ("https://example.com/page", "https://example.com/page"),
            ("https://example.com/page#", "https://example.com/page"),
            ("https://example.com/page#section", "https://example.com/page"),
            ("https://example.com/page?query=value#section", "https://example.com/page?query=value"),
            ("https://example.com/page/../other", "https://example.com/other"),
        ];
        
        for (input, expected) in test_cases {
            let url = Url::parse(input).unwrap();
            assert_eq!(
                CrawlState::normalize_url(&url),
                expected,
                "Normalization failed for {}",
                input
            );
        }
    }

    #[test]
    fn test_url_parsing_edge_cases() {
        // Test URLs that might cause parsing issues
        let edge_case_urls = vec![
            "https://example.com/path%20with%20spaces/",
            "https://example.com/path?query=value&other=param",
            "https://example.com/path#fragment",
            "https://example.com/path/../other/",
            "https://example.com:8080/path/",
            "https://subdomain.example.com/path/",
        ];
        
        for url_str in edge_case_urls {
            let url = Url::parse(url_str).unwrap();
            let start_url = Url::parse("https://example.com/path/").unwrap();
            let crawl_state = CrawlState::new(&start_url, false);
            
            // Should not panic and should return boolean
            let _ = crawl_state.should_visit_url(&url);
            let _ = is_likely_html_content(&url);
            let _ = is_absolute_url(url.as_str());
        }
    }
}