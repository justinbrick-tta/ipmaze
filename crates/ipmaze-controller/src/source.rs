use reqwest::redirect::Policy;
use serde_json::Value;
use std::net::IpAddr;
use std::str::FromStr;
use thiserror::Error;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteAddress {
    Url(Url),
    Hostname(String),
    Ip(IpAddr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedRemoteAddress {
    pub original: String,
    pub remote_address: RemoteAddress,
    pub request_url: Url,
}

#[derive(Debug, Error)]
pub enum SourceAddressError {
    #[error("remote address must not be empty")]
    Empty,
    #[error("unsupported URL scheme `{0}`; only http and https are allowed")]
    UnsupportedScheme(String),
    #[error("remote address `{0}` is neither a valid HTTP(S) URL, hostname, nor IP address")]
    InvalidAddress(String),
    #[error("generated request URL is invalid: {0}")]
    InvalidUrl(#[from] url::ParseError),
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("response body is not valid JSON")]
    InvalidJson(#[source] serde_json::Error),
}

pub fn normalize_source_address(
    address: &str,
) -> Result<NormalizedRemoteAddress, SourceAddressError> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return Err(SourceAddressError::Empty);
    }

    if let Ok(url) = Url::parse(trimmed) {
        return match url.scheme() {
            "http" | "https" => Ok(NormalizedRemoteAddress {
                original: trimmed.to_owned(),
                remote_address: RemoteAddress::Url(url.clone()),
                request_url: url,
            }),
            other => Err(SourceAddressError::UnsupportedScheme(other.to_owned())),
        };
    }

    if let Ok(ip) = IpAddr::from_str(trimmed) {
        return Ok(NormalizedRemoteAddress {
            original: trimmed.to_owned(),
            remote_address: RemoteAddress::Ip(ip),
            request_url: Url::parse(&format!("http://{trimmed}/"))?,
        });
    }

    if is_valid_hostname(trimmed) {
        return Ok(NormalizedRemoteAddress {
            original: trimmed.to_owned(),
            remote_address: RemoteAddress::Hostname(trimmed.to_owned()),
            request_url: Url::parse(&format!("https://{trimmed}/"))?,
        });
    }

    Err(SourceAddressError::InvalidAddress(trimmed.to_owned()))
}

pub fn build_http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .redirect(Policy::limited(10))
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
}

pub async fn fetch_json(
    client: &reqwest::Client,
    address: &NormalizedRemoteAddress,
) -> Result<Value, FetchError> {
    let response = client
        .get(address.request_url.clone())
        .send()
        .await?
        .error_for_status()?;
    let body = response.bytes().await?;
    parse_json_bytes(body.as_ref())
}

pub fn parse_json_bytes(bytes: &[u8]) -> Result<Value, FetchError> {
    serde_json::from_slice(bytes).map_err(FetchError::InvalidJson)
}

fn is_valid_hostname(value: &str) -> bool {
    if value.is_empty() || value.len() > 253 || value.starts_with('.') || value.ends_with('.') {
        return false;
    }

    value.split('.').all(is_valid_hostname_label)
}

fn is_valid_hostname_label(label: &str) -> bool {
    if label.is_empty() || label.len() > 63 {
        return false;
    }

    let bytes = label.as_bytes();
    if bytes.first() == Some(&b'-') || bytes.last() == Some(&b'-') {
        return false;
    }

    label
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_dns_names_default_to_https() {
        let normalized = normalize_source_address("example.invalid").unwrap();
        assert_eq!(normalized.request_url.as_str(), "https://example.invalid/");
        assert!(matches!(
            normalized.remote_address,
            RemoteAddress::Hostname(_)
        ));
    }

    #[test]
    fn bare_ip_addresses_default_to_http() {
        let normalized = normalize_source_address("203.0.113.10").unwrap();
        assert_eq!(normalized.request_url.as_str(), "http://203.0.113.10/");
        assert!(matches!(normalized.remote_address, RemoteAddress::Ip(_)));
    }

    #[test]
    fn explicit_urls_are_preserved() {
        let normalized = normalize_source_address("https://example.invalid/prefixes.json").unwrap();
        assert_eq!(
            normalized.request_url.as_str(),
            "https://example.invalid/prefixes.json"
        );
    }

    #[test]
    fn invalid_addresses_are_rejected() {
        let err = normalize_source_address("ftp://example.invalid").unwrap_err();
        assert!(matches!(err, SourceAddressError::UnsupportedScheme(_)));
    }

    #[test]
    fn invalid_json_is_rejected() {
        let err = parse_json_bytes(br#"not-json"#).unwrap_err();
        assert!(matches!(err, FetchError::InvalidJson(_)));
    }
}
