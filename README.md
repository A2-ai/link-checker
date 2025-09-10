link-checker
============

A generic web link checker tool that crawls a website and validates all links. The tool starts from a given URL and incrementally discovers and checks linked pages, building a comprehensive map of the site's link structure and identifying any broken links.

## installation

```
curl -sSL https://raw.githubusercontent.com/A2-ai/link-checker/refs/heads/main/scripts/install.sh | bash
```

or for mac can also:

```
brew tap a2-ai/homebrew-tap
brew install link-checker
```

## Features

- Multi-threaded crawling for efficient link checking
- **Path prefix crawling by default** - only follows links within the same path prefix as the starting URL
- **Domain-wide crawling option** - with `--domain-match` flag, crawls entire domain
- Comprehensive link validation with HTTP status checking
- JSON output with detailed results
- Simple command-line interface

## Crawling Behavior

### Default: Path Prefix Matching

By default, the crawler only follows links that share the same path prefix as the starting URL:

```bash
# This will only crawl URLs under /products/widgets/*
link-checker --url https://example.com/products/widgets/
```

For example:
- **Crawled**: `https://example.com/products/widgets/item1`
- **Crawled**: `https://example.com/products/widgets/specs.html` 
- **Skipped**: `https://example.com/products/gadgets/item2`
- **Skipped**: `https://example.com/about/`

### Domain-Wide Crawling

Use the `--domain-match` flag to crawl all URLs within the same domain:

```bash
# This will crawl ALL pages on example.com
link-checker --url https://example.com/products/widgets/ --domain-match
```

## Usage

Basic path-prefix crawling:

```bash
link-checker --url https://example.com/products/widgets/
```

Domain-wide crawling:

```bash
link-checker --url https://example.com --domain-match
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
