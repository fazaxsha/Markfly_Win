//! URL import with the same SSRF protections as `electron/security.cjs`:
//! HTTPS only, no credentials, public IP addresses only, bounded redirects,
//! 5 MiB size cap, 15 s timeout.
//!
//! Beyond the original, DNS is re-validated at connection time through a
//! custom reqwest resolver, so a request cannot be re-pointed between the
//! validation and the connection (classic TOCTOU / DNS-rebinding hardening).
use std::error::Error as StdError;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use url::{Host, Url};

use crate::config::{MAX_REDIRECTS, REQUEST_TIMEOUT_MS};
use crate::error::MarkflyError;
use crate::fsops::assert_markdown_size;

#[derive(Debug, Clone)]
pub struct FetchedMarkdown {
    pub content: String,
    pub url: String,
}

/// Fetches a markdown document from an HTTPS URL.
pub async fn fetch_markdown_url(value: &str) -> Result<FetchedMarkdown, MarkflyError> {
    let mut current = normalize_input(value)?;

    for redirect_count in 0..=MAX_REDIRECTS {
        // Pre-flight validation with clear error messages.
        let addresses = resolve_public_addresses(&current).await?;
        let _ = addresses; // connection is re-validated by the resolver below

        let response = request_once(&current).await?;

        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.to_string());
            if redirect_count == MAX_REDIRECTS {
                return Err(MarkflyError::TooManyRedirects);
            }
            match location {
                Some(location) => {
                    current = validate_url(&current.join(&location)?)?;
                    continue;
                }
                Option::None => {
                    return Err(MarkflyError::HttpStatus(
                        response.status().as_u16(),
                    ))
                }
            }
        }

        if !response.status().is_success() {
            return Err(MarkflyError::HttpStatus(response.status().as_u16()));
        }

        if let Some(length) = response.content_length() {
            assert_markdown_size(length)?;
        }

        let bytes = response.bytes().await?;
        assert_markdown_size(bytes.len() as u64)?;
        return Ok(FetchedMarkdown {
            content: String::from_utf8_lossy(&bytes).into_owned(),
            url: current.to_string(),
        });
    }

    Err(MarkflyError::TooManyRedirects)
}

/// Converts GitHub blob URLs to raw.githubusercontent.com URLs, like
/// `fetch-markdown-url` in the original main process.
fn normalize_input(value: &str) -> Result<Url, MarkflyError> {
    let url = validate_url(
        &Url::parse(value).map_err(|_| MarkflyError::InvalidUrl(value.to_string()))?,
    )?;
    if url.host_str() == Some("github.com") {
        if let Some(raw) = github_blob_to_raw(&url) {
            return Ok(raw);
        }
    }
    Ok(url)
}

/// `https://github.com/{owner}/{repo}/blob/{ref}/{path}` ->
/// `https://raw.githubusercontent.com/{owner}/{repo}/{ref}/{path}`
pub fn github_blob_to_raw(url: &Url) -> Option<Url> {
    if url.host_str() != Some("github.com") {
        return Option::None;
    }
    let parts: Vec<&str> = url.path().split('/').filter(|part| !part.is_empty()).collect();
    let blob_index = parts.iter().position(|part| *part == "blob")?;
    if blob_index != 2 || parts.len() <= blob_index + 1 {
        return Option::None;
    }
    let mut raw = Url::parse("https://raw.githubusercontent.com/").ok()?;
    raw.set_path(&format!(
        "/{}/{}",
        parts[0..2].join("/"),
        parts[3..].join("/")
    ));
    raw.set_query(url.query());
    validate_url(&raw).ok()
}

/// URL validation mirroring `assertSafeRemoteUrl` (minus DNS, resolved later).
fn validate_url(url: &Url) -> Result<Url, MarkflyError> {
    if url.scheme() != "https" {
        return Err(MarkflyError::NotHttps);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(MarkflyError::HasCredentials);
    }
    Ok(url.clone())
}

/// Resolves the hostname and requires every resolved address to be public.
async fn resolve_public_addresses(url: &Url) -> Result<Vec<SocketAddr>, MarkflyError> {
    let host = url
        .host()
        .ok_or_else(|| MarkflyError::InvalidUrl(url.to_string()))?;
    let hostname = host_display(&host);

    let addresses: Vec<SocketAddr> = match host {
        Host::Ipv4(addr) => vec![SocketAddr::new(IpAddr::V4(addr), port_or_default(url))],
        Host::Ipv6(addr) => vec![SocketAddr::new(IpAddr::V6(addr), port_or_default(url))],
        Host::Domain(domain) => {
            let domain: &str = domain;
            let resolved: Vec<SocketAddr> =
                tokio::net::lookup_host((domain, port_or_default(url)))
                    .await
                    .map_err(|_| MarkflyError::DnsLookup(hostname.clone()))?
                    .collect();
            if resolved.is_empty() {
                return Err(MarkflyError::DnsLookup(hostname));
            }
            resolved
        }
    };

    if addresses.is_empty() || addresses.iter().any(|addr| !is_public_ip(addr.ip())) {
        return Err(MarkflyError::PrivateAddress(hostname));
    }

    Ok(addresses)
}

fn port_or_default(url: &Url) -> u16 {
    url.port_or_known_default().unwrap_or(443)
}

/// reqwest DNS resolver that rejects private / non-routable addresses.
/// This closes the DNS-rebinding window between validation and connection.
struct ValidatingResolver;

impl reqwest::dns::Resolve for ValidatingResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|_| -> Box<dyn StdError + Send + Sync> {
                    Box::new(MarkflyError::DnsLookup(host.clone()))
                })?
                .collect();
            if addrs.is_empty() || addrs.iter().any(|addr| !is_public_ip(addr.ip())) {
                return Err(Box::new(MarkflyError::PrivateAddress(host))
                    as Box<dyn StdError + Send + Sync>);
            }
            let boxed: Box<dyn Iterator<Item = SocketAddr> + Send> = Box::new(addrs.into_iter());
            Ok(boxed)
        })
    }
}

async fn request_once(url: &Url) -> Result<reqwest::Response, MarkflyError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(REQUEST_TIMEOUT_MS))
        .dns_resolver(Arc::new(ValidatingResolver))
        .build()?;

    client
        .get(url.clone())
        .header(
            reqwest::header::ACCEPT,
            "text/markdown, text/plain, text/*;q=0.9, */*;q=0.1",
        )
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::USER_AGENT, "Markfly/0.4")
        .send()
        .await
        .map_err(MarkflyError::from)
}

fn host_display(host: &Host<&str>) -> String {
    match host {
        Host::Domain(domain) => domain.to_string(),
        Host::Ipv4(addr) => addr.to_string(),
        Host::Ipv6(addr) => format!("[{addr}]"),
    }
}

/// True when the IP address is routable on the public internet.
/// Ranges mirror the block-list in `electron/security.cjs`.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_ipv4(v4),
        IpAddr::V6(v6) => is_public_ipv6(v6),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let value = u32::from(ip);
    fn cidr(base: [u8; 4], prefix: u32) -> (u32, u32) {
        let base = u32::from_be_bytes(base);
        let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
        (base & mask, mask)
    }
    let blocked = [
        cidr([0, 0, 0, 0], 8),
        cidr([10, 0, 0, 0], 8),
        cidr([100, 64, 0, 0], 10),
        cidr([127, 0, 0, 0], 8),
        cidr([169, 254, 0, 0], 16),
        cidr([172, 16, 0, 0], 12),
        cidr([192, 0, 0, 0], 24),
        cidr([192, 0, 2, 0], 24),
        cidr([192, 168, 0, 0], 16),
        cidr([198, 18, 0, 0], 15),
        cidr([198, 51, 100, 0], 24),
        cidr([203, 0, 113, 0], 24),
        cidr([224, 0, 0, 0], 4),
        cidr([240, 0, 0, 0], 4),
    ];
    !blocked
        .iter()
        .any(|&(network, mask)| value & mask == network)
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    let value = u128::from(ip);
    fn cidr6(base: [u16; 8], prefix: u32) -> (u128, u128) {
        let mut bytes = [0u8; 16];
        for (index, part) in base.iter().enumerate() {
            bytes[index * 2..index * 2 + 2].copy_from_slice(&part.to_be_bytes());
        }
        let base = u128::from_be_bytes(bytes);
        let mask = if prefix == 0 { 0 } else { u128::MAX << (128 - prefix) };
        (base & mask, mask)
    }
    let blocked = [
        cidr6([0; 8], 128),
        cidr6([0, 0, 0, 0, 0, 0, 0, 1], 128),
        cidr6([0, 0, 0, 0, 0, 0xffff, 0, 0], 96),
        cidr6([0x0100, 0, 0, 0, 0, 0, 0, 0], 64),
        cidr6([0x2001, 0x0db8, 0, 0, 0, 0, 0, 0], 32),
        cidr6([0xfc00, 0, 0, 0, 0, 0, 0, 0], 7),
        cidr6([0xfe80, 0, 0, 0, 0, 0, 0, 0], 10),
        cidr6([0xff00, 0, 0, 0, 0, 0, 0, 0], 8),
    ];
    !blocked
        .iter()
        .any(|&(network, mask)| value & mask == network)
}

/// True when an external URL is safe to hand to the OS handler.
pub fn is_safe_external_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host() else {
        return false;
    };
    match host {
        Host::Ipv4(_) | Host::Ipv6(_) => true,
        Host::Domain(domain) => {
            let domain = domain.to_ascii_lowercase();
            !domain.is_empty() && domain != "localhost" && !domain.ends_with(".localhost")
        }
    }
}
