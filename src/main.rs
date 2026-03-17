use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use colored::Colorize;
use reqwest::{Client, StatusCode};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

// ============================================================================
// Error Types
// ============================================================================

/// Categorized fetch errors for better handling and messaging
#[derive(Debug)]
enum FetchError {
    /// Network-level failures (DNS, connection refused, timeout)
    Network { message: String, is_transient: bool },
    /// jina.ai service returned an error status
    JinaService { status: StatusCode, body: String },
    /// Target site error (jina.ai forwards site errors in response body)
    TargetSite { url: String, hint: String },
    /// Rate limited
    RateLimited { retry_after: Option<u64> },
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Network { message, is_transient } => {
                if *is_transient {
                    write!(f, "Network error (transient): {} - will retry", message)
                } else {
                    write!(f, "Network error: {}", message)
                }
            }
            FetchError::JinaService { status, body } => {
                write!(f, "jina.ai service error ({}): {}", status, body.chars().take(200).collect::<String>())
            }
            FetchError::TargetSite { url, hint } => {
                write!(f, "Target site error for {}: {}", url, hint)
            }
            FetchError::RateLimited { retry_after } => {
                match retry_after {
                    Some(secs) => write!(f, "Rate limited - retry after {}s", secs),
                    None => write!(f, "Rate limited - please wait before retrying"),
                }
            }
        }
    }
}

impl std::error::Error for FetchError {}

// ============================================================================
// Retry Configuration
// ============================================================================

/// Retry configuration with exponential backoff
#[derive(Debug, Clone)]
struct RetryConfig {
    /// Maximum number of retry attempts
    max_retries: u32,
    /// Initial backoff duration
    initial_backoff: Duration,
    /// Maximum backoff duration
    max_backoff: Duration,
    /// Multiplier for exponential backoff
    backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Calculate backoff duration for a given attempt (0-indexed)
    fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        let backoff_secs = self.initial_backoff.as_secs_f64() 
            * self.backoff_multiplier.powi(attempt as i32);
        let capped = backoff_secs.min(self.max_backoff.as_secs_f64());
        Duration::from_secs_f64(capped)
    }
}

const JINA_READER_PREFIX: &str = "https://r.jina.ai/";
const USER_AGENT: &str = concat!("jf/", env!("CARGO_PKG_VERSION"));

/// Fetch any webpage as clean Markdown via jina.ai Reader
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about,
    long_about = None,
    after_help = "Examples:
  jf https://example.com/article
  jf https://docs.rs/tokio -o tokio.md
  jf batch urls.txt -d ./output
  echo 'https://example.com' | jf stdin
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

        /// Maximum retry attempts for transient failures (default: 3)
        #[arg(short = 'r', long, default_value = "3")]
        retries: u32,

        /// Don't retry on failures
        #[arg(long)]
        no_retry: bool,
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

        /// Maximum retry attempts for transient failures (default: 3)
        #[arg(short = 'r', long, default_value = "3")]
        retries: u32,

        /// Don't retry on failures
        #[arg(long)]
        no_retry: bool,

        /// Continue processing URLs even if some fail
        #[arg(long, default_value = "true")]
        continue_on_error: bool,
    },

    /// Read URL from stdin
    Stdin {
        /// Wait mode: instruct jina.ai to wait for JS rendering
        #[arg(short = 'w', long)]
        wait_render: bool,

        /// Custom timeout in seconds (default: 30)
        #[arg(short = 't', long, default_value = "30")]
        timeout: u64,

        /// Maximum retry attempts for transient failures (default: 3)
        #[arg(short = 'r', long, default_value = "3")]
        retries: u32,
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

        /// Maximum retry attempts for transient failures (default: 3)
        #[arg(short = 'r', long, default_value = "3")]
        retries: u32,
    },
}

#[derive(Debug)]
#[allow(dead_code)]
struct FetchResult {
    markdown: String,
    original_url: String,
    response_time_ms: u64,
    attempts: u32,
}

// ============================================================================
// Error Detection and Classification
// ============================================================================

/// Patterns that indicate target site errors in jina.ai response body
const TARGET_ERROR_PATTERNS: &[&str] = &[
    "Application error",
    "Error: Failed to fetch",
    "502 Bad Gateway",
    "503 Service Unavailable",
    "504 Gateway Timeout",
    "404 Not Found",
    "Access Denied",
    "Forbidden",
    "This site can't be reached",
    "connection refused",
    "ERR_CONNECTION_",
    "cloudflare",
    "CAPTCHA",
    "too many requests",
];

/// Check if an error from reqwest is transient (worth retrying)
fn is_transient_error(error: &reqwest::Error) -> bool {
    // Timeout is transient
    if error.is_timeout() {
        return true;
    }
    
    // Connection errors are usually transient
    if error.is_connect() {
        return true;
    }
    
    // Request construction errors are NOT transient
    if error.is_builder() {
        return false;
    }
    
    // Body/decode errors might be transient (partial transfer)
    if error.is_body() || error.is_decode() {
        return true;
    }
    
    // Redirect errors are usually not transient
    if error.is_redirect() {
        return false;
    }
    
    // Status errors depend on the status code
    if error.is_status() {
        if let Some(status) = error.status() {
            return matches!(status.as_u16(), 
                429 |           // Rate limit
                500 | 502 | 503 | 504  // Server errors
            );
        }
    }
    
    // Default: assume transient for unknown error types
    true
}

/// Detect if response body contains a target site error
fn detect_target_site_error(body: &str, original_url: &str) -> Option<FetchError> {
    let body_lower = body.to_lowercase();
    
    for pattern in TARGET_ERROR_PATTERNS {
        if body_lower.contains(&pattern.to_lowercase()) {
            // Try to extract more context from the error
            let hint = extract_error_hint(body, pattern);
            return Some(FetchError::TargetSite {
                url: original_url.to_string(),
                hint,
            });
        }
    }
    
    // Check if response looks like an error page (very short with error keywords)
    if body.len() < 500 && body_lower.contains("error") {
        return Some(FetchError::TargetSite {
            url: original_url.to_string(),
            hint: "Response looks like an error page".to_string(),
        });
    }
    
    None
}

/// Extract a helpful hint from an error response
fn extract_error_hint(body: &str, matched_pattern: &str) -> String {
    // First, look for lines containing the matched pattern
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.len() > 10 && trimmed.len() < 200 {
            // Skip markdown headers
            if !trimmed.starts_with('#') && !trimmed.is_empty() {
                // Prioritize lines containing the error pattern
                if trimmed.to_lowercase().contains(&matched_pattern.to_lowercase()) {
                    return trimmed.to_string();
                }
            }
        }
    }
    
    // Fallback: look for lines containing "error" or "failed"
    for line in body.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        if trimmed.len() > 10 && trimmed.len() < 200 
            && !trimmed.starts_with('#') 
            && (lower.contains("error") || lower.contains("failed") || lower.contains("unavailable")) {
            return trimmed.to_string();
        }
    }
    
    matched_pattern.to_string()
}

// ============================================================================
// URL Validation
// ============================================================================

/// Validate and normalize a URL
fn validate_url(url: &str) -> Result<String> {
    let url = url.trim();
    
    // Basic scheme check
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!(
            "Invalid URL: must start with http:// or https://\n\
             Provided: '{}'\n\
             Tip: jinafetch expects full URLs including the scheme",
            url
        );
    }
    
    // Check for obviously malformed URLs
    if url.contains(' ') || url.contains('"') || url.contains('\'') {
        bail!("Invalid URL: contains invalid characters");
    }
    
    // Try to parse to validate structure
    let parsed = url::Url::parse(url)
        .map_err(|e| anyhow::anyhow!("Invalid URL format: {}", e))?;
    
    // Ensure there's a host
    if parsed.host_str().is_none() {
        bail!("Invalid URL: missing host");
    }
    
    Ok(url.to_string())
}

// ============================================================================
// Fetch Implementation with Retry
// ============================================================================

/// Perform a single fetch attempt (no retry)
async fn fetch_once(
    client: &Client,
    url: &str,
    wait_render: bool,
) -> std::result::Result<String, FetchError> {
    let jina_url = format!("{JINA_READER_PREFIX}{url}");
    
    let mut request = client.get(&jina_url);
    
    if wait_render {
        request = request.query(&[("wait", "true")]);
    }
    
    let response = request
        .send()
        .await
        .map_err(|e| {
            let message = e.to_string();
            let is_transient = is_transient_error(&e);
            FetchError::Network { message, is_transient }
        })?;
    
    let status = response.status();
    
    // Check for error status codes
    if !status.is_success() {
        // Try to get error body for more context
        let error_body = response.text().await.unwrap_or_default();
        
        // Check for rate limiting
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(FetchError::RateLimited { retry_after: None });
        }
        
        return Err(FetchError::JinaService {
            status,
            body: error_body,
        });
    }
    
    // Get response body
    let body = response.text().await.map_err(|e| FetchError::Network {
        message: format!("Failed to read response: {}", e),
        is_transient: true,
    })?;
    
    // Check if body contains a target site error
    if let Some(err) = detect_target_site_error(&body, url) {
        return Err(err);
    }
    
    Ok(body)
}

/// Fetch with automatic retry on transient failures
async fn fetch_with_retry(
    client: &Client,
    url: &str,
    wait_render: bool,
    verbose: bool,
    retry_config: &RetryConfig,
) -> Result<FetchResult> {
    let start = std::time::Instant::now();
    let mut last_error: Option<FetchError> = None;
    let mut attempts;
    
    if verbose {
        eprintln!("{} {}", "→".cyan(), url.dimmed());
    }
    
    for attempt in 0..=retry_config.max_retries {
        attempts = attempt + 1;
        
        if attempt > 0 {
            let backoff = retry_config.backoff_for_attempt(attempt - 1);
            if verbose {
                eprintln!(
                    "  {} Retrying in {}ms (attempt {}/{})",
                    "↻".yellow(),
                    backoff.as_millis(),
                    attempt + 1,
                    retry_config.max_retries + 1
                );
            }
            tokio::time::sleep(backoff).await;
        }
        
        match fetch_once(client, url, wait_render).await {
            Ok(body) => {
                let response_time_ms = start.elapsed().as_millis() as u64;
                
                if verbose {
                    eprintln!(
                        "{} Fetched {} chars in {}ms (attempt {})",
                        "✓".green(),
                        body.len().to_string().green(),
                        format!("{}ms", response_time_ms).yellow(),
                        attempts.to_string().cyan()
                    );
                }
                
                return Ok(FetchResult {
                    markdown: body,
                    original_url: url.to_string(),
                    response_time_ms,
                    attempts,
                });
            }
            Err(e) => {
                let should_retry = match &e {
                    FetchError::Network { is_transient, .. } => *is_transient && attempt < retry_config.max_retries,
                    FetchError::JinaService { status, .. } => {
                        // Retry server errors
                        let is_server_error = status.as_u16() >= 500;
                        is_server_error && attempt < retry_config.max_retries
                    }
                    FetchError::RateLimited { .. } => attempt < retry_config.max_retries,
                    FetchError::TargetSite { .. } => false, // Never retry target site errors
                };
                
                if verbose {
                    let icon = if should_retry { "⚠".yellow() } else { "✗".red() };
                    eprintln!("  {} {}", icon, e);
                }
                
                last_error = Some(e);
                
                if !should_retry {
                    break;
                }
            }
        }
    }
    
    // All retries exhausted
    let error = last_error.unwrap_or_else(|| FetchError::Network {
        message: "Unknown error".to_string(),
        is_transient: false,
    });
    
    bail!("{}", error)
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
    
    match cli.command {
        Commands::Fetch {
            url,
            output,
            verbose,
            wait_render,
            timeout,
            retries,
            no_retry,
        } => {
            let client = Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(timeout))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .context("Failed to create HTTP client")?;
            
            let url = validate_url(&url)?;
            
            let retry_config = if no_retry {
                RetryConfig { max_retries: 0, ..Default::default() }
            } else {
                RetryConfig { max_retries: retries, ..Default::default() }
            };
            
            let result = fetch_with_retry(&client, &url, wait_render, verbose, &retry_config).await?;
            write_output(&result.markdown, output.as_ref(), verbose)?;
        }
        
        Commands::Batch {
            file,
            dir,
            verbose,
            wait_render,
            timeout,
            retries,
            no_retry,
            continue_on_error,
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
                bail!("No URLs found in file: {}", file.display());
            }
            
            if verbose {
                eprintln!("{} {} URLs", "→".cyan(), urls.len());
            }
            
            let retry_config = if no_retry {
                RetryConfig { max_retries: 0, ..Default::default() }
            } else {
                RetryConfig { max_retries: retries, ..Default::default() }
            };
            
            let mut success_count = 0usize;
            let mut failure_count = 0usize;
            
            for (i, url) in urls.iter().enumerate() {
                if verbose {
                    eprintln!("\n[{} / {}]", i + 1, urls.len());
                }
                
                // Validate URL
                let url = match validate_url(url) {
                    Ok(u) => u,
                    Err(e) => {
                        eprintln!("{} {}: {}", "✗".red(), url, e.to_string().red());
                        failure_count += 1;
                        if !continue_on_error {
                            return Err(e);
                        }
                        continue;
                    }
                };
                
                match fetch_with_retry(&client, &url, wait_render, verbose, &retry_config).await {
                    Ok(result) => {
                        success_count += 1;
                        if let Some(ref dir_path) = dir {
                            // Generate filename from URL
                            let filename = url_to_filename(&url);
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
                        failure_count += 1;
                        eprintln!("{} {}: {}", "✗".red(), url, e.to_string().red());
                        if !continue_on_error {
                            return Err(e);
                        }
                    }
                }
            }
            
            if verbose {
                eprintln!(
                    "\n{} Batch complete: {} succeeded, {} failed",
                    "✓".green(),
                    success_count.to_string().green(),
                    if failure_count > 0 { failure_count.to_string().red() } else { "0".green() }
                );
            }
            
            if failure_count > 0 && !continue_on_error {
                bail!("Batch processing failed: {} URLs failed", failure_count);
            }
        }
        
        Commands::Stdin {
            wait_render,
            timeout,
            retries,
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
                bail!("No URL provided on stdin");
            }
            
            let url = validate_url(url)?;
            
            let retry_config = RetryConfig { max_retries: retries, ..Default::default() };
            
            // stdin mode is always quiet, just output the markdown
            let result = fetch_with_retry(&client, &url, wait_render, false, &retry_config).await?;
            write_output(&result.markdown, None, false)?;
        }
        
        Commands::Selector {
            url,
            selector,
            output,
            verbose,
            timeout,
            retries,
        } => {
            let client = Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(timeout))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .context("Failed to create HTTP client")?;
            
            let url = validate_url(&url)?;
            
            let retry_config = RetryConfig { max_retries: retries, ..Default::default() };
            
            // jina.ai Reader supports selector via query parameter
            // Format: https://r.jina.ai/http://example.com?selector=article
            let jina_url = format!("{}{}?selector={}", JINA_READER_PREFIX, url, selector);
            
            if verbose {
                eprintln!("{} {} [selector: {}]", "→".cyan(), jina_url.dimmed(), selector.cyan());
            }
            
            let start = std::time::Instant::now();
            
            // Use retry logic for selector mode too
            let mut last_error: Option<anyhow::Error> = None;
            
            for attempt in 0..=retry_config.max_retries {
                if attempt > 0 {
                    let backoff = retry_config.backoff_for_attempt(attempt - 1);
                    if verbose {
                        eprintln!(
                            "  {} Retrying in {}ms",
                            "↻".yellow(),
                            backoff.as_millis()
                        );
                    }
                    tokio::time::sleep(backoff).await;
                }
                
                match client.get(&jina_url).send().await {
                    Ok(response) => {
                        let status = response.status();
                        if !status.is_success() {
                            let error_body = response.text().await.unwrap_or_default();
                            let e = anyhow::anyhow!("jina.ai returned status {}: {}", status, error_body.chars().take(200).collect::<String>());
                            
                            // Retry server errors
                            if status.as_u16() >= 500 && attempt < retry_config.max_retries {
                                last_error = Some(e);
                                continue;
                            }
                            return Err(e);
                        }
                        
                        match response.text().await {
                            Ok(markdown) => {
                                // Check for target site errors
                                if let Some(err) = detect_target_site_error(&markdown, &url) {
                                    // Target site errors are not retriable
                                    bail!("{}", err);
                                }
                                
                                if verbose {
                                    eprintln!(
                                        "{} Fetched {} chars in {}ms",
                                        "✓".green(),
                                        markdown.len().to_string().green(),
                                        format!("{}ms", start.elapsed().as_millis()).yellow()
                                    );
                                }
                                
                                write_output(&markdown, output.as_ref(), verbose)?;
                                return Ok(());
                            }
                            Err(e) => {
                                last_error = Some(anyhow::anyhow!("Failed to read response body: {}", e));
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        let is_transient = e.is_timeout() || e.is_connect();
                        let err = anyhow::anyhow!("Failed to fetch URL with selector: {}", e);
                        
                        if is_transient && attempt < retry_config.max_retries {
                            last_error = Some(err);
                            continue;
                        }
                        return Err(err);
                    }
                }
            }
            
            return Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error")));
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
