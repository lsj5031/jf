// ============================================================================
// HTTP Client Module
// ============================================================================

use crate::config::RetryConfig;
use crate::error::{FetchError, FetchResult, detect_target_site_error, is_transient_error};
use colored::Colorize;
use reqwest::{Client, StatusCode};

pub const JINA_READER_PREFIX: &str = "https://r.jina.ai/";

/// Build jina.ai reader URL
pub fn build_jina_url(url: &str, wait_render: bool, selector: Option<&str>) -> String {
    let base = format!("{}{}", JINA_READER_PREFIX, url);

    match (wait_render, selector) {
        (true, Some(sel)) => format!("{}?wait=true&selector={}", base, sel),
        (true, None) => format!("{}?wait=true", base),
        (false, Some(sel)) => format!("{}?selector={}", base, sel),
        (false, None) => base,
    }
}

/// Perform a single fetch attempt (no retry)
pub async fn fetch_once(
    client: &Client,
    url: &str,
    wait_render: bool,
    selector: Option<&str>,
) -> Result<String, FetchError> {
    let jina_url = build_jina_url(url, wait_render, selector);

    let request = client.get(&jina_url);

    let response = request.send().await.map_err(|e| {
        let message = e.to_string();
        let is_transient = is_transient_error(&e);
        FetchError::Network {
            message,
            is_transient,
        }
    })?;

    let status = response.status();

    if !status.is_success() {
        let error_body = response.text().await.unwrap_or_default();

        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(FetchError::RateLimited { retry_after: None });
        }

        return Err(FetchError::JinaService {
            status,
            body: error_body,
        });
    }

    let body = response.text().await.map_err(|e| FetchError::Network {
        message: format!("Failed to read response: {}", e),
        is_transient: true,
    })?;

    if let Some(err) = detect_target_site_error(&body, url) {
        return Err(err);
    }

    Ok(body)
}

/// Fetch with automatic retry on transient failures
pub async fn fetch_with_retry(
    client: &Client,
    url: &str,
    wait_render: bool,
    selector: Option<&str>,
    verbose: bool,
    retry_config: &RetryConfig,
) -> Result<FetchResult, anyhow::Error> {
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

        match fetch_once(client, url, wait_render, selector).await {
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
                let should_retry = e.is_retryable() && attempt < retry_config.max_retries;

                if verbose {
                    let icon = if should_retry {
                        "⚠".yellow()
                    } else {
                        "✗".red()
                    };
                    eprintln!("  {} {}", icon, e);
                }

                last_error = Some(e);

                if !should_retry {
                    break;
                }
            }
        }
    }

    let error = last_error.unwrap_or_else(|| FetchError::Network {
        message: "Unknown error".to_string(),
        is_transient: false,
    });

    Err(anyhow::Error::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_jina_url_basic() {
        let url = build_jina_url("https://example.com", false, None);
        assert_eq!(url, "https://r.jina.ai/https://example.com");
    }

    #[test]
    fn test_build_jina_url_with_wait() {
        let url = build_jina_url("https://example.com", true, None);
        assert!(url.contains("wait=true"));
    }

    #[test]
    fn test_build_jina_url_with_selector() {
        let url = build_jina_url("https://example.com", false, Some("article"));
        assert!(url.contains("selector=article"));
    }

    #[test]
    fn test_build_jina_url_with_wait_and_selector() {
        let url = build_jina_url("https://example.com", true, Some("article"));
        assert!(url.contains("wait=true"));
        assert!(url.contains("selector=article"));
    }
}
