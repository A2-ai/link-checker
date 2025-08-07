link-checker
============

A generic web link checker tool that crawls a website and validates all links within the same domain. The tool starts from a given URL and incrementally discovers and checks all linked pages, building a comprehensive map of the site's link structure and identifying any broken links.

## Features

- Multi-threaded crawling for efficient link checking
- Domain-scoped crawling (only follows links within the same domain)
- Comprehensive link validation with HTTP status checking
- JSON output with detailed results
- Simple command-line interface

## Usage

To run the link checker:

```bash
cargo run -- --url https://example.com
```

Or using the short form:

```bash
cargo run -- -u https://example.com
```

## Output

The tool generates two JSON files:
- `bad_urls.json`: List of URLs that returned error status codes
- `url_map.json`: Complete mapping of each page to all links found on that page

## Building

Build the release binary:

```bash
cargo build --release
```
