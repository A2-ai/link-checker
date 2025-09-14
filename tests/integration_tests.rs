use url::Url;
use regex::Regex;

// Import the main crate functions
use link_checker::{CrawlState, is_likely_html_content, format_bytes};

// Note: These integration tests would ideally use a mock HTTP server
// For now, they test the core logic integration without network calls

#[test]
fn test_crawl_state_integration_path_prefix() {
    let start_url = Url::parse("https://example.com/products/widgets/").unwrap();
    let mut crawl_state = CrawlState::new(&start_url, false);
    
    let urls_to_test = vec![
        ("https://example.com/products/widgets/item1", true),
        ("https://example.com/products/widgets/specs.html", true),
        ("https://example.com/products/gadgets/item2", false),
        ("https://example.com/about/", false),
    ];
    
    for (url_str, should_visit) in urls_to_test {
        let url = Url::parse(url_str).unwrap();
        
        if crawl_state.should_visit_url(&url) == should_visit {
            // Test mark_visited behavior
            if should_visit {
                assert!(crawl_state.mark_visited(&url), "First visit should return true for {}", url_str);
                assert!(!crawl_state.mark_visited(&url), "Second visit should return false for {}", url_str);
            }
        } else {
            panic!("URL {} should_visit expected: {}, got: {}", url_str, should_visit, crawl_state.should_visit_url(&url));
        }
    }
}

#[test]
fn test_crawl_state_integration_domain_match() {
    let start_url = Url::parse("https://example.com/products/widgets/").unwrap();
    let mut crawl_state = CrawlState::new(&start_url, true);
    
    let urls_to_test = vec![
        ("https://example.com/products/widgets/item1", true),
        ("https://example.com/products/gadgets/item2", true),
        ("https://example.com/about/", true),
        ("https://example.com/", true),
        ("https://other.com/products/widgets/", false),
    ];
    
    for (url_str, should_visit) in urls_to_test {
        let url = Url::parse(url_str).unwrap();
        
        assert_eq!(
            crawl_state.should_visit_url(&url),
            should_visit,
            "URL {} should_visit expected: {}, got: {}",
            url_str,
            should_visit,
            crawl_state.should_visit_url(&url)
        );
        
        if should_visit {
            assert!(crawl_state.mark_visited(&url), "First visit should return true for {}", url_str);
            assert!(!crawl_state.mark_visited(&url), "Second visit should return false for {}", url_str);
        }
    }
}

#[test]
fn test_skip_pattern_integration() {
    let skip_patterns = vec![
        (r"external\.com", vec![
            ("https://example.com/page", false),
            ("https://external.com/page", true),
            ("https://subdomain.external.com/page", true),
        ]),
        (r"\.(pdf|zip|exe)$", vec![
            ("https://example.com/document.pdf", true),
            ("https://example.com/archive.zip", true),
            ("https://example.com/program.exe", true),
            ("https://example.com/page.html", false),
            ("https://example.com/image.png", false),
        ]),
        (r"^https://external\.com|/downloads/", vec![
            ("https://external.com/page", true),
            ("https://example.com/downloads/file.zip", true),
            ("https://example.com/page.html", false),
        ]),
    ];
    
    for (pattern_str, test_cases) in skip_patterns {
        let regex = Regex::new(pattern_str).unwrap();
        
        for (url, should_match) in test_cases {
            assert_eq!(
                regex.is_match(url),
                should_match,
                "Pattern '{}' on URL '{}' - expected: {}, got: {}",
                pattern_str,
                url,
                should_match,
                regex.is_match(url)
            );
        }
    }
}

#[test]
fn test_url_normalization_and_deduplication() {
    let start_url = Url::parse("https://example.com/").unwrap();
    let mut crawl_state = CrawlState::new(&start_url, true);
    
    // Test that URLs with different fragments are treated as the same
    let urls_with_fragments = vec![
        "https://example.com/page.html",
        "https://example.com/page.html#section1",
        "https://example.com/page.html#section2",
        "https://example.com/page.html?param=value#section3",
    ];
    
    let base_url = Url::parse("https://example.com/page.html").unwrap();
    
    // First URL should be new
    assert!(crawl_state.mark_visited(&base_url));
    
    // All other variations should be considered already visited
    for url_str in &urls_with_fragments[1..] {
        let url = Url::parse(url_str).unwrap();
        // Note: URLs with query parameters are different from the base URL
        // Only fragment differences should be normalized away
        let should_be_visited = if url_str.contains("?param=value") {
            true  // Different query params mean different URL
        } else {
            false  // Only fragment differences should be treated as same
        };
        
        assert_eq!(
            crawl_state.mark_visited(&url),
            should_be_visited,
            "URL {} should be considered already visited: {}",
            url_str, should_be_visited
        );
    }
}

#[test]
fn test_content_type_detection_logic() {
    // Test the logic for determining HTML content
    let test_cases = vec![
        // HTML-like URLs
        ("https://example.com/", true),
        ("https://example.com/page", true),
        ("https://example.com/dir/", true),
        ("https://example.com/page.html", true),
        ("https://example.com/page.htm", true),
        ("https://example.com/page.php", true),
        ("https://example.com/page.asp", true),
        ("https://example.com/page.aspx", true),
        ("https://example.com/page.jsp", true),
        
        // Non-HTML URLs
        ("https://example.com/image.jpg", false),
        ("https://example.com/document.pdf", false),
        ("https://example.com/archive.zip", false),
        ("https://example.com/script.js", false),
        ("https://example.com/style.css", false),
        ("https://example.com/data.json", false),
        ("https://example.com/data.xml", false),
    ];
    
    for (url_str, expected_html) in test_cases {
        let url = Url::parse(url_str).unwrap();
        assert_eq!(
            is_likely_html_content(&url),
            expected_html,
            "URL {} - expected HTML: {}, got: {}",
            url_str,
            expected_html,
            is_likely_html_content(&url)
        );
    }
}

#[test]
fn test_bytes_formatting_edge_cases() {
    let test_cases = vec![
        (0, "0 B"),
        (1, "1 B"),
        (1023, "1023 B"),
        (1024, "1.0 KB"),
        (1025, "1.0 KB"),
        (1536, "1.5 KB"),
        (1048575, "1024.0 KB"),
        (1048576, "1.0 MB"),
        (1073741824, "1024.0 MB"),
        (2147483648, "2048.0 MB"),
    ];
    
    for (bytes, expected) in test_cases {
        assert_eq!(
            format_bytes(bytes),
            expected,
            "Bytes {} - expected: {}, got: {}",
            bytes,
            expected,
            format_bytes(bytes)
        );
    }
}

// Test the interaction between path prefix matching and HTML content detection
#[test]
fn test_matching_and_content_detection_interaction() {
    let start_url = Url::parse("https://example.com/api/v1/").unwrap();
    let crawl_state = CrawlState::new(&start_url, false); // path prefix mode
    
    let test_urls = vec![
        // Should visit (same prefix) - HTML detection based on URL structure
        ("https://example.com/api/v1/users", true, true),  // no extension = HTML
        ("https://example.com/api/v1/users.json", true, false),  // .json = not HTML
        ("https://example.com/api/v1/docs/", true, true),  // ends with / = HTML
        ("https://example.com/api/v1/docs/guide.html", true, true),  // .html = HTML
        
        // Should not visit (different prefix) - but HTML detection still works
        ("https://example.com/api/v2/users", false, true),  // no extension = HTML but won't visit
        ("https://example.com/docs/", false, true),  // ends with / = HTML but won't visit
    ];
    
    for (url_str, should_visit, is_html) in test_urls {
        let url = Url::parse(url_str).unwrap();
        
        assert_eq!(
            crawl_state.should_visit_url(&url),
            should_visit,
            "URL {} should_visit - expected: {}, got: {}",
            url_str,
            should_visit,
            crawl_state.should_visit_url(&url)
        );
        
        if should_visit {
            let should_extract = crawl_state.should_extract_links(&url);
            assert_eq!(
                should_extract,
                should_visit, // should_extract_links should match should_visit_url
                "URL {} should_extract_links - expected: {}, got: {}",
                url_str,
                should_visit,
                should_extract
            );
        }
        
        assert_eq!(
            is_likely_html_content(&url),
            is_html,
            "URL {} is_likely_html_content - expected: {}, got: {}",
            url_str,
            is_html,
            is_likely_html_content(&url)
        );
    }
}