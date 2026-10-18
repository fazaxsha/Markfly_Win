//! Error type for Markfly core. Messages mirror the original Electron
//! implementation so the UI copy stays identical.
use std::fmt;

#[derive(Debug)]
pub enum MarkflyError {
    Io(std::io::Error),
    NotMarkdown(String),
    TooLarge { size: u64, limit: u64 },
    Unauthorized,
    InvalidUrl(String),
    NotHttps,
    HasCredentials,
    PrivateAddress(String),
    DnsLookup(String),
    TooManyRedirects,
    HttpStatus(u16),
    Http(String),
    Watcher(String),
}

impl fmt::Display for MarkflyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MarkflyError::Io(err) => write!(f, "{err}"),
            MarkflyError::NotMarkdown(path) => write!(f, "Select a Markdown file. ({path})"),
            MarkflyError::TooLarge { size, limit } => write!(
                f,
                "Markdown file is larger than {} MB. (size {size} bytes, limit {limit} bytes)",
                limit / (1024 * 1024)
            ),
            MarkflyError::Unauthorized => write!(f, "File access was not authorized."),
            MarkflyError::InvalidUrl(value) => write!(f, "Enter a valid HTTPS URL. ({value})"),
            MarkflyError::NotHttps => write!(f, "Only HTTPS URLs are supported."),
            MarkflyError::HasCredentials => {
                write!(f, "URLs containing credentials are not supported.")
            }
            MarkflyError::PrivateAddress(host) => {
                write!(f, "URL must resolve to a public internet address. ({host})")
            }
            MarkflyError::DnsLookup(host) => write!(f, "Could not resolve host: {host}."),
            MarkflyError::TooManyRedirects => write!(f, "URL redirected too many times."),
            MarkflyError::HttpStatus(status) => write!(f, "Request failed: HTTP {status}."),
            MarkflyError::Http(message) => write!(f, "{message}"),
            MarkflyError::Watcher(message) => write!(f, "Could not watch file: {message}"),
        }
    }
}

impl std::error::Error for MarkflyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MarkflyError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for MarkflyError {
    fn from(err: std::io::Error) -> Self {
        MarkflyError::Io(err)
    }
}

impl From<reqwest::Error> for MarkflyError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            MarkflyError::Http("Request timed out. Try again.".to_string())
        } else {
            MarkflyError::Http(err.to_string())
        }
    }
}

impl From<serde_json::Error> for MarkflyError {
    fn from(err: serde_json::Error) -> Self {
        MarkflyError::Http(err.to_string())
    }
}

impl From<url::ParseError> for MarkflyError {
    fn from(_: url::ParseError) -> Self {
        MarkflyError::InvalidUrl("malformed URL".to_string())
    }
}
