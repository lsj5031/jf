use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use reqwest::Client;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

const JINA_READER_PREFIX: &str = "https://r.jina.ai/";
const USER_AGENT: &str = concat!("jinafetch/", env!("CARGO_PKG_VERSION"));

/// Fetch any webpage as clean Markdown via jina.ai Reader
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about,
    long_about = None,
    after_help = "Examples:
  jinafetch https://example.com/article
  jinafetch https://docs.rs/tokio -o tokio.md
  jinafetch batch urls.txt -d ./output
  echo 'https://example.com' | jinafetch --stdin
"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Fetch a single URL and output as Markdown
    #[command(visible_alias("get"))]
    Fetch {
        /// URL to fetch (any webpage)
        url: String,

        /// Output file path (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Show progress and metadata
        #[arg(short, long)]
        verbose: bool,

        /// Wait mode: instruct jina.ai to wait for JS rendering
        #[arg(short = 'w', long)]
        wait_render: bool,

        /// Custom timeout in seconds (default: 30)
        #[arg(short = 't', long, default_value = "30")]
        timeout: u64,
    },

    /// Fetch multiple URLs from a file
    Batch {
        /// File containing URLs (one per line)
        file: PathBuf,

        /// Output directory for saved files
        #[arg(short, long)]
        dir: Option<PathBuf>,

        /// Show progress for each URL
        #[arg(short, long)]
        verbose: bool,

        /// Wait mode: instruct jina.ai to wait for JS rendering
        #[arg(short = 'w', long)]
        wait_render: bool,

        /// Custom timeout in seconds (default: 30)
        #[arg(short = 't', long, default_value = "30")]
        timeout: u64,
    },

    /// Read URL from stdin
    Stdin {
        /// Wait mode: instruct jina.ai to wait for JS rendering
        #[arg(short = 'w', long)]
        wait_render: bool,

        /// Custom timeout in seconds (default: 30)
        #[arg(short = 't', long, default_value = "30")]
        timeout: u64,
    },

    /// Extract main content with a CSS selector (jina.ai supports markdownify)
    #[command(visible_alias("extract"))]
    Selector {
        /// URL to fetch
        url: String,

        /// CSS selector to extract (e.g., "article", "#content", ".main")
        #[arg(short, long)]
        selector: String,

        /// Output file path (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Show progress and metadata
        #[arg(short, long)]
        verbose: bool,

        /// Custom timeout in seconds (default: 30)
        #[arg(short = 't', long, default_value = "30")]
        timeout: u64,
    },
}

#[derive(Debug)]
#[allow(dead_code)]
struct FetchResult {
    markdown: String,
    original_url: String,
    response_time_ms: u64,
}

async fn fetch_url(client: &Client, url: &str, wait_render: bool, verbose: bool) -> Result<FetchResult> {
    let jina_url = format!("{JINA_READER_PREFIX}{url}");
    
    if verbose {
        eprintln!("{} {}", "→".cyan(), jina_url.dimmed());
    }
    
    let start = std::time::Instant::now();
    
    let mut request = client.get(&jina_url);
    
    // jina.ai Reader supports "wait" mode via query parameter
    if wait_render {
        request = request.query(&[("wait", "true")]);
    }
    
    let response = request
        .send()
        .await
        .with_context(|| format!("Failed to connect to jina.ai for URL: {}", url))?;
    
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("jina.ai returned status {} for URL: {}", status, url);
    }
    
    let markdown = response
        .text()
        .await
        .context("Failed to read response body")?;
    
    let response_time_ms = start.elapsed().as_millis() as u64;
    
    if verbose {
        eprintln!(
            "✓ Fetched {} chars in {}ms",
            markdown.len().to_string().green(),
            format!("{}ms", response_time_ms).yellow()
        );
    }
    
    Ok(FetchResult {
        markdown,
        original_url: url.to_string(),
        response_time_ms,
    })
}

fn write_output(content: &str, output: Option<&PathBuf>, verbose: bool) -> Result<()> {
    match output {
        Some(path) => {
            std::fs::write(path, content)
                .with_context(|| format!("Failed to write to: {}", path.display()))?;
            if verbose {
                eprintln!("{} {}", "✓ Written to".green(), path.display().to_string().cyan());
            }
        }
        None => {
            // Write to stdout, ensuring we don't add extra newlines
            io::stdout().write_all(content.as_bytes())?;
            if !content.is_empty() && !content.ends_with('\n') {
                io::stdout().write_all(b"\n")?;
            }
        }
    }
    Ok(())
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    
    // Setup logging (only show with verbose or RUST_LOG set)
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .init();
    }
    
    // Build HTTP client with sensible defaults (unused here, each command builds its own)
    
    match cli.command {
        Commands::Fetch {
            url,
            output,
            verbose,
            wait_render,
            timeout,
        } => {
            let client = Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(timeout))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .context("Failed to create HTTP client")?;
            
            let result = fetch_url(&client, &url, wait_render, verbose).await?;
            write_output(&result.markdown, output.as_ref(), verbose)?;
        }
        
        Commands::Batch {
            file,
            dir,
            verbose,
            wait_render,
            timeout,
        } => {
            let client = Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(timeout))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .context("Failed to create HTTP client")?;
            
            let urls: Vec<String> = std::fs::read_to_string(&file)
                .context("Failed to read URL file")?
                .lines()
                .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
                .map(|s| s.trim().to_string())
                .collect();
            
            if urls.is_empty() {
                anyhow::bail!("No URLs found in file: {}", file.display());
            }
            
            if verbose {
                eprintln!("{} {} URLs", "→".cyan(), urls.len());
            }
            
            for (i, url) in urls.iter().enumerate() {
                if verbose {
                    eprintln!("\n[{} / {}]", i + 1, urls.len());
                }
                
                match fetch_url(&client, url, wait_render, verbose).await {
                    Ok(result) => {
                        if let Some(ref dir_path) = dir {
                            // Generate filename from URL
                            let filename = url_to_filename(url);
                            let output_path = dir_path.join(&filename);
                            std::fs::create_dir_all(dir_path)
                                .with_context(|| format!("Failed to create directory: {}", dir_path.display()))?;
                            write_output(&result.markdown, Some(&output_path), verbose)?;
                        } else {
                            if i > 0 {
                                println!("\n---\n");
                            }
                            println!("<!-- Source: {} -->\n", url);
                            write_output(&result.markdown, None, verbose)?;
                        }
                    }
                    Err(e) => {
                        eprintln!("{} {}: {}", "✗".red(), url, e.to_string().red());
                    }
                }
            }
        }
        
        Commands::Stdin {
            wait_render,
            timeout,
        } => {
            let client = Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(timeout))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .context("Failed to create HTTP client")?;
            
            let mut url = String::new();
            io::stdin().read_line(&mut url)?;
            let url = url.trim();
            
            if url.is_empty() {
                anyhow::bail!("No URL provided on stdin");
            }
            
            // stdin mode is always quiet, just output the markdown
            let result = fetch_url(&client, url, wait_render, false).await?;
            write_output(&result.markdown, None, false)?;
        }
        
        Commands::Selector {
            url,
            selector,
            output,
            verbose,
            timeout,
        } => {
            let client = Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(timeout))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .context("Failed to create HTTP client")?;
            
            // jina.ai Reader supports selector via query parameter
            // Format: https://r.jina.ai/http://example.com?selector=article
            let jina_url = format!("{}{}?selector={}", JINA_READER_PREFIX, url, selector);
            
            if verbose {
                eprintln!("{} {} [selector: {}]", "→".cyan(), jina_url.dimmed(), selector.cyan());
            }
            
            let start = std::time::Instant::now();
            let response = client
                .get(&jina_url)
                .send()
                .await
                .with_context(|| format!("Failed to fetch URL with selector: {}", url))?;
            
            let status = response.status();
            if !status.is_success() {
                anyhow::bail!("jina.ai returned status {} for URL: {}", status, url);
            }
            
            let markdown = response
                .text()
                .await
                .context("Failed to read response body")?;
            
            if verbose {
                eprintln!(
                    "✓ Fetched {} chars in {}ms",
                    markdown.len().to_string().green(),
                    format!("{}ms", start.elapsed().as_millis()).yellow()
                );
            }
            
            write_output(&markdown, output.as_ref(), verbose)?;
        }
    }
    
    Ok(())
}

/// Convert a URL to a safe filename
fn url_to_filename(url: &str) -> String {
    let url = url.trim_end_matches('/');
    
    // Try to extract the last meaningful path segment
    let path_segment = url
        .split("://")
        .nth(1)
        .and_then(|s| s.split('/').next_back())
        .unwrap_or(url);
    
    // Clean up the filename
    let mut filename = path_segment
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>();
    
    // Ensure reasonable length and extension
    if filename.len() > 100 {
        filename = filename[..100].to_string();
    }
    if !filename.ends_with(".md") {
        filename.push_str(".md");
    }
    
    filename
}

fn main() {
    if let Err(e) = tokio::runtime::Runtime::new()
        .expect("Failed to create tokio runtime")
        .block_on(run())
    {
        eprintln!("{} {}", "Error:".red(), e);
        std::process::exit(1);
    }
}
