//! LiteLLM base-URL policy and management-route URLs.
//!
//! Upstream `litellm.ts` declares the `LITELLM_BASE_URL` endpoint with the
//! `https-or-private-network-http` policy: HTTPS anywhere, plain HTTP only for
//! loopback, RFC 1918, link-local, IPv6 unique-local, and `.local` hosts.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use reqwest::Url;

use crate::core::ProviderError;

const INVALID_BASE: &str = "LiteLLM base URL must use HTTPS, or HTTP on a loopback or private-network address, without embedded credentials.";

/// Validate a LiteLLM base URL. A scheme-less value is treated as HTTPS.
pub(crate) fn validated_base_url(raw: &str) -> Result<Url, ProviderError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ProviderError::Other("LiteLLM base URL is empty".into()));
    }
    let lower = trimmed.to_ascii_lowercase();
    if ["%2f", "%5c", "%3f", "%23", "%40", "%3a"]
        .iter()
        .any(|encoded| lower.contains(encoded))
    {
        return Err(ProviderError::Other(
            "LiteLLM base URL must not contain encoded host delimiters".into(),
        ));
    }
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = Url::parse(&candidate)
        .map_err(|e| ProviderError::Other(format!("Invalid LiteLLM base URL: {e}")))?;
    let host = url
        .host_str()
        .ok_or_else(|| ProviderError::Other("LiteLLM base URL must include a host".into()))?;
    let scheme_ok = match url.scheme() {
        "https" => true,
        "http" => is_private_network_host(host),
        _ => false,
    };
    if !scheme_ok
        || !url.username().is_empty()
        || url.password().is_some()
        || host.contains('%')
        || host.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(ProviderError::Other(INVALID_BASE.into()));
    }
    Ok(url)
}

/// Build `{base}/{path}` for a management route. A trailing `/v1` on the base
/// is dropped, and the base path and query are otherwise preserved. `query`
/// replaces the base query when given.
pub(super) fn management_url(
    base: &str,
    path: &str,
    query: Option<(&str, &str)>,
) -> Result<Url, ProviderError> {
    let mut url = validated_base_url(base)?;
    let trimmed = url.path().trim_end_matches('/');
    let root = trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_string();
    url.set_path(&format!("{root}/{path}"));
    url.set_fragment(None);
    if let Some((key, value)) = query {
        url.query_pairs_mut().clear().append_pair(key, value);
    }
    Ok(url)
}

/// Upstream `isPrivateNetworkHost`: `localhost`, `.local` names, and IP
/// literals that are loopback, RFC 1918, link-local, or IPv6 unique-local.
/// Other names (including `*.localhost`) and IPv4-mapped IPv6 literals stay
/// HTTPS-only, because a bearer key would otherwise cross the network in
/// plain text if the name resolved somewhere public.
fn is_private_network_host(host: &str) -> bool {
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    if normalized == "localhost"
        || normalized
            .strip_suffix(".local")
            .is_some_and(|label| !label.is_empty())
    {
        return true;
    }
    let ip_candidate = normalized
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(&normalized);
    match ip_candidate.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => is_private_ipv4(ip),
        Ok(IpAddr::V6(ip)) => is_private_ipv6(ip),
        Err(_) => false,
    }
}

fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

fn is_private_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || (ip.segments()[0] & 0xfe00) == 0xfc00
        || (ip.segments()[0] & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_https_anywhere_and_private_network_http() {
        for value in [
            "https://litellm.example.com",
            "litellm.example.com",
            "http://localhost:4000",
            "http://127.0.0.1:4000",
            "http://[::1]:4000",
            "http://10.1.2.3",
            "http://172.16.0.9",
            "http://192.168.1.20:4000",
            "http://169.254.10.10",
            "http://[fd12:3456::1]",
            "http://[fe80::1]",
            "http://proxy.local:4000",
        ] {
            assert!(validated_base_url(value).is_ok(), "rejected {value}");
        }
    }

    #[test]
    fn rejects_public_http_credentials_and_encoded_delimiters() {
        for value in [
            "",
            "http://litellm.example.com",
            "http://8.8.8.8",
            "http://172.32.0.1",
            "http://[2001:db8::1]",
            "http://example.com.evil.test",
            "ftp://10.0.0.1",
            "https://user:pass@litellm.example.com",
            "http://user@10.0.0.1",
            "https://example.com%2f.evil.test",
            "http://app.localhost:4000",
            "http://[::ffff:10.0.0.1]",
            "http://.local:4000",
        ] {
            assert!(validated_base_url(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn management_url_strips_v1_and_keeps_subpath() {
        let url = management_url("https://h.example.com/litellm/v1/", "key/info", None).unwrap();
        assert_eq!(url.as_str(), "https://h.example.com/litellm/key/info");
        let url = management_url("http://10.0.0.2:4000/v1", "key/info", None).unwrap();
        assert_eq!(url.as_str(), "http://10.0.0.2:4000/key/info");
    }

    #[test]
    fn management_url_encodes_query_and_replaces_base_query() {
        let url = management_url(
            "https://h.example.com?token=abc",
            "user/info",
            Some(("user_id", "a b&c")),
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://h.example.com/user/info?user_id=a+b%26c"
        );
        let kept = management_url("https://h.example.com?token=abc", "key/info", None).unwrap();
        assert_eq!(kept.as_str(), "https://h.example.com/key/info?token=abc");
    }
}
