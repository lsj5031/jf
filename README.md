# jf

**Ultra-fast CLI to fetch any webpage as clean Markdown via [jina.ai Reader](https://r.jina.ai).**

Just prepend `jf` to any URL and get beautiful, LLM-ready Markdown output.

## Installation

### From Source

```bash
git clone https://github.com/lsj5031/jf.git
cd jf
cargo install --path .
```

### Binary Release

Download from [Releases](https://github.com/lsj5031/jf/releases).

## Usage

### Basic Fetch

```bash
jf https://example.com/article
```

### Save to File

```bash
jf https://docs.rs/tokio -o tokio.md
```

### Verbose Mode (with timing)

```bash
jf https://blog.rust-lang.org -v
```

### Wait for JavaScript Rendering

```bash
jf https://spa-website.com -w
```

### Batch Processing

```bash
# Process multiple URLs from a file
jf batch urls.txt -d ./output

# urls.txt format:
# https://example.com/article1
# https://example.com/article2
# # Comments are supported
```

### Pipe Support

```bash
echo 'https://example.com' | jf stdin
curl -s list-of-urls.txt | xargs -I {} sh -c 'echo {} | jf stdin'
```

### CSS Selector Extraction

```bash
jf selector https://news.ycombinator.com -s ".athing"
```

## Options

```
jf fetch <URL> [OPTIONS]

Arguments:
  <URL>  URL to fetch

Options:
  -o, --output <FILE>    Output file path (default: stdout)
  -v, --verbose          Show progress and metadata
  -w, --wait-render      Wait for JS rendering
  -t, --timeout <SECS>   Custom timeout (default: 30)
  -r, --retries <N>      Max retry attempts (default: 3)
      --no-retry         Disable all retries
```

## Features

- **Automatic Retries** — Exponential backoff for transient failures (network, 5xx, rate limits)
- **Smart Error Detection** — Distinguishes between jina.ai errors and target site issues
- **URL Validation** — Catches malformed URLs early with helpful messages
- **Batch Mode** — Process multiple URLs efficiently
- **Pipe Support** — Works in Unix pipelines
- **CSS Selectors** — Extract specific content from pages

## Error Handling

`jf` categorizes errors intelligently:

| Error Type | Retry? | Example |
|------------|--------|---------|
| Network transient | ✅ | DNS timeout, connection reset |
| jina.ai 5xx | ✅ | Server overloaded |
| Rate limit (429) | ✅ | Too many requests |
| Target site error | ❌ | Cloudflare block, 404 |
| Invalid URL | ❌ | Missing scheme |

Retry behavior:
- Exponential backoff: 500ms → 1s → 2s (capped at 30s)
- Only retries transient errors
- Never retries client errors (4xx) or target site issues

## Why `jf`?

The name follows Unix tradition of short, memorable tool names (like `ls`, `cat`, `jq`).

It's also short for "jina fetch" — the core operation.

## How It Works

`jf` uses the [jina.ai Reader API](https://jina.ai/reader/) which:

1. Fetches the target webpage
2. Renders JavaScript (optional)
3. Extracts main content
4. Converts to clean Markdown
5. Returns LLM-ready text

Just prepend `https://r.jina.ai/` to any URL — that's the magic. `jf` wraps this with retry logic, error handling, and a polished CLI.

## License

MIT
