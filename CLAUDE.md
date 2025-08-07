# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a generic Rust-based link checker tool that crawls public websites to validate all links within a domain. The tool starts from a given URL and incrementally discovers and checks all linked pages, building a comprehensive map of the site's link structure and identifying broken links.

## Development Commands

### Running the application
```bash
# Run with a URL argument
cargo run -- --url https://example.com

# Using short form
cargo run -- -u https://example.com
```

### Building
```bash
cargo build          # Debug build
cargo build --release # Release build
```

### Testing
```bash
cargo test
```

### Other useful commands
```bash
cargo check          # Fast compilation check without producing binary
cargo clippy          # Linting
cargo fmt             # Code formatting
```

## Configuration

The application uses command-line arguments for configuration:
- `--url` or `-u`: The starting URL to crawl (required)

## Architecture

### Core Components

**Main Flow (`main.rs`)**:
1. **Argument Parsing**: Uses `clap` derive macros for simple command-line interface
2. **Multi-threaded Crawling**: Spawns 8 worker threads for concurrent page fetching
3. **Link Extraction**: Uses `scraper` to parse HTML and extract all `<a>` tags
4. **State Management**: Tracks visited pages and maintains crawl queue
5. **Result Output**: Generates JSON files with bad URLs and complete URL mapping

**Key Data Structures**:
- `Args`: Command-line argument structure using clap derive
- `CrawlState`: Manages visited pages and domain filtering logic
- `CrawlCommand`: Work unit containing URL and whether to extract links
- `FoundUrls`: Result containing page URL and all discovered links
- `UrlResults`: Final output with bad URLs and complete URL mapping

**Threading Model**:
- Control thread manages crawl queue and coordinates work
- 8 worker threads process pages concurrently using shared channels
- Each worker maintains its own HTTP client for making requests

### Link Discovery Logic

- Only extracts links from pages within the target domain
- Follows all discovered links to build complete site map
- Handles relative and absolute URLs correctly using base URL resolution
- Deduplicates visited pages to avoid infinite loops

## Output Files

The crawler generates two JSON files:
- `bad_urls.json`: List of URLs that returned non-success HTTP status codes
- `url_map.json`: Complete mapping of each page to all links found on that page

## Dependencies

- `clap`: Command-line argument parsing with derive macros
- `reqwest`: HTTP client for making web requests
- `scraper`: HTML parsing and CSS selector support
- `thiserror`: Error handling macros
- `serde`/`serde_json`: JSON serialization for output files