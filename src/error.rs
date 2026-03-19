// ============================================================================
// Error Types
// ============================================================================

use reqwest::StatusCode;
use std::fmt;
use std::fmt::Formatter;

/// Categorized fetch errors for better handling and messaging
#[derive(Debug, Clone)]
pub enum FetchError {
    /// Network-level failures (DNS, connection refused, timeout)
    Network { message: String, is_transient: bool },
    /// jina.ai service returned an error status
    JinaService { status: StatusCode, body: String },
    /// Target site error (jina.ai forwards site errors in response body)
    TargetSite { url: String, hint: String },
    /// Rate limited
    RateLimited { retry_after: Option<u64> },
    /// Validation error
    Validation { message: String },
}

impl FetchError {
    /// Check if this error should be retried
    pub fn is_retryable(&self) -> bool {
        match self {
            FetchError::Network { is_transient, .. } => *is_transient,
            FetchError::JinaService { status, .. } => status.as_u16() >= 500,
            FetchError::RateLimited { .. } => true,
            FetchError::TargetSite { .. } => false,
            FetchError::Validation { .. } => false,
        }
    }

    /// Get exit code for this error (1=general, 2=validation, 3=service, 4=rate limit)
    pub fn exit_code(&self) -> i32 {
        match self {
            FetchError::Network { .. } => 1,
            FetchError::Validation { .. } => 2,
            FetchError::JinaService { .. } => 3,
            FetchError::RateLimited { .. } => 4,
            FetchError::TargetSite { .. } => 5,
        }
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Network {
                message,
                is_transient,
            } => {
                if *is_transient {
                    write!(f, "Network error (transient): {} - will retry", message)
                } else {
                    write!(f, "Network error: {}", message)
                }
            }
            FetchError::JinaService { status, body } => {
                write!(
                    f,
                    "jina.ai service error ({}): {}",
                    status,
                    body.chars().take(200).collect::<String>()
                )
            }
            FetchError::TargetSite { url, hint } => {
                write!(f, "Target site error for {}: {}", url, hint)
            }
            FetchError::RateLimited { retry_after } => match retry_after {
                Some(secs) => write!(f, "Rate limited - retry after {}s", secs),
                None => write!(f, "Rate limited - please wait before retrying"),
            },
            FetchError::Validation { message } => {
                write!(f, "Validation error: {}", message)
            }
        }
    }
}

impl std::error::Error for FetchError {}

/// Fetch result containing markdown and metadata
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub markdown: String,
    pub original_url: String,
    pub response_time_ms: u64,
    pub attempts: u32,
}

// ============================================================================
// Error Detection
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
pub fn is_transient_error(error: &reqwest::Error) -> bool {
    if error.is_timeout() {
        return true;
    }
    if error.is_connect() {
        return true;
    }
    if error.is_builder() {
        return false;
    }
    if error.is_body() || error.is_decode() {
        return true;
    }
    if error.is_redirect() {
        return false;
    }
    if let Some(status) = error.status() {
        return matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504);
    }
    true
}

/// Detect if response body contains a target site error
pub fn detect_target_site_error(body: &str, original_url: &str) -> Option<FetchError> {
    let body_lower = body.to_lowercase();

    for pattern in TARGET_ERROR_PATTERNS {
        if body_lower.contains(&pattern.to_lowercase()) {
            let hint = extract_error_hint(body, pattern);
            return Some(FetchError::TargetSite {
                url: original_url.to_string(),
                hint,
            });
        }
    }

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
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.len() > 10
            && trimmed.len() < 200
            && !trimmed.starts_with('#')
            && !trimmed.is_empty()
            && trimmed
                .to_lowercase()
                .contains(&matched_pattern.to_lowercase())
        {
            return trimmed.to_string();
        }
    }

    for line in body.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        if trimmed.len() > 10
            && trimmed.len() < 200
            && !trimmed.starts_with('#')
            && (lower.contains("error")
                || lower.contains("failed")
                || lower.contains("unavailable"))
        {
            return trimmed.to_string();
        }
    }

    matched_pattern.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_error_network_is_retryable() {
        let err = FetchError::Network {
            message: "timeout".to_string(),
            is_transient: true,
        };
        assert!(err.is_retryable());
    }

    #[test]
    fn test_fetch_error_network_not_retryable() {
        let err = FetchError::Network {
            message: "invalid".to_string(),
            is_transient: false,
        };
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_fetch_error_500_is_retryable() {
        let err = FetchError::JinaService {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: "error".to_string(),
        };
        assert!(err.is_retryable());
    }

    #[test]
    fn test_fetch_error_404_is_not_retryable() {
        let err = FetchError::JinaService {
            status: StatusCode::NOT_FOUND,
            body: "not found".to_string(),
        };
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_fetch_error_target_site_not_retryable() {
        let err = FetchError::TargetSite {
            url: "https://example.com".to_string(),
            hint: "Cloudflare block".to_string(),
        };
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_fetch_error_validation_not_retryable() {
        let err = FetchError::Validation {
            message: "missing scheme".to_string(),
        };
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_exit_codes() {
        assert_eq!(
            FetchError::Network {
                message: "".to_string(),
                is_transient: false
            }
            .exit_code(),
            1
        );
        assert_eq!(
            FetchError::Validation {
                message: "".to_string()
            }
            .exit_code(),
            2
        );
        assert_eq!(
            FetchError::JinaService {
                status: StatusCode::BAD_GATEWAY,
                body: "".to_string()
            }
            .exit_code(),
            3
        );
        assert_eq!(FetchError::RateLimited { retry_after: None }.exit_code(), 4);
        assert_eq!(
            FetchError::TargetSite {
                url: "".to_string(),
                hint: "".to_string()
            }
            .exit_code(),
            5
        );
    }

    #[test]
    fn test_detect_target_site_error_cloudflare() {
        let body = "Error: Cloudflare block detected";
        let result = detect_target_site_error(body, "https://example.com");
        assert!(result.is_some());
        if let Some(FetchError::TargetSite { hint, .. }) = result {
            assert!(hint.to_lowercase().contains("cloudflare"));
        }
    }

    #[test]
    fn test_detect_target_site_error_none() {
        let body = "# Article Title\n\nThis is a normal article content.";
        let result = detect_target_site_error(body, "https://example.com");
        assert!(result.is_none());
    }

    #[test]
    fn test_detect_target_site_error_short_error() {
        let body = "Error 404 not found";
        let result = detect_target_site_error(body, "https://example.com");
        assert!(result.is_some());
    }
}
