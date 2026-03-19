// ============================================================================
// CLI Commands Module
// ============================================================================

use crate::config::RetryConfig;
use crate::error::FetchError;
use crate::http;
use crate::utils::{parse_url_file, url_to_filename, validate_url, write_output};
use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use colored::Colorize;
use reqwest::Client;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

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
  jf check
"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
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

        /// Maximum concurrent requests (default: unlimited)
        #[arg(short = 'c', long)]
        concurrency: Option<usize>,
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

        /// Don't retry on failures
        #[arg(long)]
        no_retry: bool,
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

    /// Check API connectivity
    Check {
        /// Custom timeout in seconds (default: 10)
        #[arg(short = 't', long, default_value = "10")]
        timeout: u64,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,

        /// Output file (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

/// Create HTTP client helper
fn create_http_client(timeout: u64) -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(timeout))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .context("Failed to create HTTP client")
}

/// Execute the CLI
pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    // Setup logging
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
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
            let client = create_http_client(timeout)?;
            let url = validate_url(&url).map_err(anyhow::Error::new)?;

            let retry_config = if no_retry {
                RetryConfig {
                    max_retries: 0,
                    ..Default::default()
                }
            } else {
                RetryConfig {
                    max_retries: retries,
                    ..Default::default()
                }
            };

            let result =
                http::fetch_with_retry(&client, &url, wait_render, None, verbose, &retry_config)
                    .await?;
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
            concurrency,
        } => {
            let client = create_http_client(timeout)?;

            let contents = std::fs::read_to_string(&file).context("Failed to read URL file")?;
            let urls = parse_url_file(&contents);

            if urls.is_empty() {
                bail!("No URLs found in file: {}", file.display());
            }

            if verbose {
                eprintln!("{} {} URLs", "→".cyan(), urls.len());
                if let Some(limit) = concurrency {
                    eprintln!("  {} Concurrency limit: {}", "ℹ".blue(), limit);
                }
            }

            let retry_config = if no_retry {
                RetryConfig {
                    max_retries: 0,
                    ..Default::default()
                }
            } else {
                RetryConfig {
                    max_retries: retries,
                    ..Default::default()
                }
            };

            // Use concurrent processing with semaphore
            let client = Arc::new(client);
            let semaphore = Arc::new(Semaphore::new(concurrency.unwrap_or(usize::MAX)));

            let urls_len = urls.len();
            let mut success_count = 0usize;
            let mut failure_count = 0usize;
            let mut first = true;
            let mut has_failure = false;

            // Pre-spawn all tasks but collect results immediately to check continue_on_error
            let mut handles = Vec::new();
            for (i, url_str) in urls.iter().enumerate() {
                let permit = semaphore.clone().acquire_owned().await?;
                let client = client.clone();
                let url_str = url_str.clone();
                let retry_config = retry_config.clone();

                let handle = tokio::spawn(async move {
                    let _permit = permit;

                    if verbose {
                        eprintln!("\n[{} / {}] {}", i + 1, urls_len, url_str.dimmed());
                    }

                    let url = match validate_url(&url_str) {
                        Ok(u) => u,
                        Err(e) => {
                            return Err(anyhow::Error::new(e));
                        }
                    };

                    http::fetch_with_retry(&client, &url, wait_render, None, verbose, &retry_config)
                        .await
                        .map(|result| (url_str, result))
                });

                handles.push(handle);
            }

            // Collect results and process immediately to check continue_on_error after each result
            for handle in handles {
                match handle.await {
                    Ok(Ok((url_str, result))) => {
                        success_count += 1;
                        if let Some(ref dir_path) = dir {
                            let filename = url_to_filename(&url_str);
                            let output_path = dir_path.join(&filename);
                            std::fs::create_dir_all(dir_path).with_context(|| {
                                format!("Failed to create directory: {}", dir_path.display())
                            })?;
                            write_output(&result.markdown, Some(&output_path), verbose)?;
                        } else {
                            if !first {
                                println!("\n---\n");
                            }
                            println!("<!-- Source: {} -->", url_str);
                            write_output(&result.markdown, None, verbose)?;
                            first = false;
                        }
                    }
                    Ok(Err(e)) => {
                        failure_count += 1;
                        eprintln!("{} URL: {}", "✗".red(), e.to_string().red());
                        has_failure = true;
                    }
                    Err(e) => {
                        failure_count += 1;
                        eprintln!("{} Task error: {}", "✗".red(), e);
                        has_failure = true;
                    }
                }

                // Check continue_on_error after each result - stop if we have a failure and flag is false
                if has_failure && !continue_on_error {
                    break;
                }
            }

            if verbose {
                eprintln!(
                    "\n{} Batch complete: {} succeeded, {} failed",
                    "✓".green(),
                    success_count.to_string().green(),
                    if failure_count > 0 {
                        failure_count.to_string().red()
                    } else {
                        "0".green()
                    }
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
            no_retry,
        } => {
            let client = create_http_client(timeout)?;

            let mut url = String::new();
            std::io::stdin().read_line(&mut url)?;
            let url = url.trim();

            if url.is_empty() {
                bail!("No URL provided on stdin");
            }

            let url = validate_url(url).map_err(anyhow::Error::new)?;

            let retry_config = if no_retry {
                RetryConfig {
                    max_retries: 0,
                    ..Default::default()
                }
            } else {
                RetryConfig {
                    max_retries: retries,
                    ..Default::default()
                }
            };

            let result =
                http::fetch_with_retry(&client, &url, wait_render, None, false, &retry_config)
                    .await?;
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
            let client = create_http_client(timeout)?;
            let url = validate_url(&url).map_err(anyhow::Error::new)?;

            let retry_config = RetryConfig {
                max_retries: retries,
                ..Default::default()
            };

            if verbose {
                eprintln!(
                    "{} {} [selector: {}]",
                    "→".cyan(),
                    url.dimmed(),
                    selector.cyan()
                );
            }

            let result = http::fetch_with_retry(
                &client,
                &url,
                false,
                Some(&selector),
                verbose,
                &retry_config,
            )
            .await?;
            write_output(&result.markdown, output.as_ref(), verbose)?;
        }

        Commands::Check { timeout } => {
            let client = create_http_client(timeout)?;

            eprintln!("{} Checking jina.ai API connectivity...", "→".cyan());

            match http::fetch_once(&client, "http://example.com", false, None).await {
                Ok(_) => {
                    eprintln!("{} API is reachable", "✓".green());
                }
                Err(e) => {
                    eprintln!("{} API check failed: {}", "✗".red(), e);
                    bail!("Health check failed");
                }
            }
        }

        Commands::Completions { shell, output } => {
            let mut cmd = Cli::command();
            let mut buf = Vec::new();

            clap_complete::generate(shell, &mut cmd, "jf", &mut buf);

            match output {
                Some(path) => {
                    std::fs::write(&path, &buf).with_context(|| {
                        format!("Failed to write completions to: {}", path.display())
                    })?;
                    eprintln!(
                        "{} Generated {} completions to {}",
                        "✓".green(),
                        shell,
                        path.display()
                    );
                }
                None => {
                    std::io::stdout().write_all(&buf)?;
                }
            }
        }
    }

    Ok(())
}

/// Get exit code from error
pub fn get_exit_code(error: &anyhow::Error) -> i32 {
    // Try to find FetchError in the error chain
    for cause in error.chain() {
        if let Some(fetch_error) = cause.downcast_ref::<FetchError>() {
            return fetch_error.exit_code();
        }
    }
    1 // Default exit code
}
