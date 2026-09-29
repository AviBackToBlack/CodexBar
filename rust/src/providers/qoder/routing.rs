//! Qoder regional site routing.
//!
//! Qoder runs two independent sites, `qoder.com` and `qoder.com.cn`. A session
//! cookie is only valid for the site that issued it, so every credential is
//! bound to exactly one site and is never presented to the other one.
//!
//! Browser candidates are bound by the domain they were read for. A manual
//! credential is routed by its content: a plain `Cookie` header is
//! international unless a `Domain=` attribute names a site, a pasted cURL
//! command or raw HTTP request selects the site from its URL or `Host` header,
//! and anything ambiguous or conflicting is rejected before any request is
//! made. Ported from upstream v0.66.0 `QoderCookieRouting.swift`.

use regex_lite::Regex;
use std::collections::BTreeSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QoderSite {
    International,
    China,
}

impl QoderSite {
    pub(super) const ALL: [Self; 2] = [Self::International, Self::China];

    pub(super) const fn domain(self) -> &'static str {
        match self {
            Self::International => "qoder.com",
            Self::China => "qoder.com.cn",
        }
    }

    pub(super) const fn origin(self) -> &'static str {
        match self {
            Self::International => "https://qoder.com",
            Self::China => "https://qoder.com.cn",
        }
    }

    pub(super) fn usage_url(self) -> String {
        format!("{}/api/v2/me/usages/big_model_credits", self.origin())
    }

    pub(super) fn referer(self) -> String {
        format!("{}/account/usage", self.origin())
    }
}

/// A manual credential resolved to one site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ManualCredential {
    pub(super) site: QoderSite,
    pub(super) cookie_header: String,
}

/// Which pasted form selected the site; decides where the cookie comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteKind {
    Curl,
    HttpRequest,
    PlainCookie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Site(QoderSite, RouteKind),
    Invalid,
}

/// Resolve a manual header to its site and normalized cookie header, or `None`
/// when the routing is invalid or no cookie can be extracted.
pub(super) fn manual_credential(raw: &str) -> Option<ManualCredential> {
    let Route::Site(site, kind) = manual_route(raw) else {
        return None;
    };
    let cookie_header = match kind {
        RouteKind::Curl => super::normalize_cookie_header(&curl_cookie_value(raw)?),
        RouteKind::HttpRequest => super::normalize_cookie_header(&http_cookie_value(raw)?),
        RouteKind::PlainCookie => plain_cookie_header(raw),
    }?;
    Some(ManualCredential {
        site,
        cookie_header,
    })
}

#[cfg(test)]
pub(super) fn site_for_manual_header(raw: &str) -> Option<QoderSite> {
    match manual_route(raw) {
        Route::Site(site, _) => Some(site),
        Route::Invalid => None,
    }
}

fn manual_route(raw: &str) -> Route {
    if let Some(route) = curl_request_route(raw) {
        return route;
    }
    if let Some(route) = http_request_route(raw) {
        return route;
    }
    plain_cookie_route(raw)
}

// ---------------------------------------------------------------------------
// Plain Cookie header
// ---------------------------------------------------------------------------

/// Swift `split(separator:maxSplits: 1)` with empty subsequences omitted: a
/// pair needs a non-empty name and a non-empty remainder.
fn split_pair(text: &str, separator: char) -> Option<(&str, &str)> {
    let (first, rest) = text.trim_start_matches(separator).split_once(separator)?;
    (!rest.is_empty()).then_some((first, rest))
}

fn plain_cookie_route(raw: &str) -> Route {
    let mut routed: Option<QoderSite> = None;
    for part in raw.split(';').filter(|part| !part.is_empty()) {
        let Some((name, value)) = split_pair(part, '=') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("domain") {
            continue;
        }
        let Some(site) = site_for_host(value.trim().trim_matches(['"', '\''])) else {
            return Route::Invalid;
        };
        if routed.is_some_and(|previous| previous != site) {
            return Route::Invalid;
        }
        routed = Some(site);
    }
    Route::Site(
        routed.unwrap_or(QoderSite::International),
        RouteKind::PlainCookie,
    )
}

/// A `Domain=` attribute only routes the credential; it is not a cookie, so it
/// is not sent.
fn plain_cookie_header(raw: &str) -> Option<String> {
    let normalized = super::normalize_cookie_header(raw)?;
    let pairs = normalized
        .split("; ")
        .filter(|pair| {
            !pair
                .split_once('=')
                .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case("domain"))
        })
        .collect::<Vec<_>>();
    (!pairs.is_empty()).then(|| pairs.join("; "))
}

// ---------------------------------------------------------------------------
// Raw HTTP request
// ---------------------------------------------------------------------------

const SUPPORTED_METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];
const UNSUPPORTED_KNOWN_METHODS: [&str; 2] = ["trace", "connect"];

fn is_newline(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

fn lines(raw: &str) -> impl Iterator<Item = &str> {
    raw.split(is_newline).filter(|line| !line.is_empty())
}

fn is_http_request_method_token(token: &str) -> bool {
    !token.is_empty() && token.chars().all(|c| c.is_ascii_alphabetic())
}

fn http_request_route(raw: &str) -> Option<Route> {
    let mut request_site: Option<QoderSite> = None;
    let mut saw_request_line = false;

    for line in lines(raw) {
        let parts = line
            .trim()
            .split([' ', '\t'])
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.len() < 2 {
            continue;
        }
        let method = parts[0].to_ascii_lowercase();
        let is_http_versioned_line =
            parts.len() >= 3 && parts[2].to_ascii_lowercase().starts_with("http/");
        if UNSUPPORTED_KNOWN_METHODS.contains(&method.as_str())
            || (is_http_versioned_line && is_http_request_method_token(parts[0]))
        {
            if !SUPPORTED_METHODS.contains(&method.as_str()) {
                return Some(Route::Invalid);
            }
        } else if !SUPPORTED_METHODS.contains(&method.as_str()) {
            continue;
        }

        if saw_request_line {
            return Some(Route::Invalid);
        }
        saw_request_line = true;

        let target = parts[1];
        if target.starts_with('/') {
            request_site = None;
        } else if let Some(site) = site_for_url_text(target) {
            request_site = Some(site);
        } else {
            return Some(Route::Invalid);
        }
    }

    if !saw_request_line {
        return None;
    }
    let host_sites = host_header_sites(raw);
    if host_sites.iter().any(Option::is_none) {
        return Some(Route::Invalid);
    }
    let concrete = host_sites.into_iter().flatten().collect::<Vec<_>>();
    if concrete.iter().any(|site| Some(site) != concrete.first()) {
        return Some(Route::Invalid);
    }

    let site = match (request_site, concrete.first().copied()) {
        (Some(request), Some(host)) if request != host => return Some(Route::Invalid),
        (Some(request), _) => request,
        (None, Some(host)) => host,
        (None, None) => return Some(Route::Invalid),
    };
    Some(Route::Site(site, RouteKind::HttpRequest))
}

fn host_header_sites(raw: &str) -> Vec<Option<QoderSite>> {
    lines(raw)
        .filter_map(|line| split_pair(line, ':'))
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case("host"))
        .map(|(_, value)| site_for_host(value))
        .collect()
}

fn http_cookie_value(raw: &str) -> Option<String> {
    lines(raw)
        .filter_map(|line| split_pair(line, ':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("cookie"))
        .map(|(_, value)| value.trim().to_string())
}

// ---------------------------------------------------------------------------
// cURL command
// ---------------------------------------------------------------------------

type Targets = Vec<(usize, QoderSite)>;

fn curl_request_route(raw: &str) -> Option<Route> {
    let Some(preprocessed) = preprocessed_curl_shell_text(raw) else {
        return contains_curl_executable_text(raw).then_some(Route::Invalid);
    };
    let tokens = shell_tokens(&preprocessed);
    let Some(curl_index) = curl_command_index(&tokens) else {
        return tokens
            .iter()
            .any(|token| is_curl_executable_token(token))
            .then_some(Route::Invalid);
    };
    if !tokens.iter().all(|token| is_token_text_safe(token)) {
        return Some(Route::Invalid);
    }

    let (Some(explicit), Some(url_targets)) = (
        explicit_curl_url_targets(&tokens, curl_index),
        url_token_targets(&tokens, curl_index),
    ) else {
        return Some(Route::Invalid);
    };

    let indices = explicit
        .iter()
        .chain(&url_targets)
        .map(|(index, _)| *index)
        .collect::<BTreeSet<_>>();
    if indices.len() != 1 {
        return Some(Route::Invalid);
    }
    let target_index = *indices.first()?;
    let trusted_index = curl_index + 1;

    let target_site = if let Some((_, site)) = explicit.iter().find(|(i, _)| *i == target_index) {
        *site
    } else if target_index == trusted_index
        && let Some((_, site)) = url_targets.iter().find(|(i, _)| *i == trusted_index)
    {
        *site
    } else {
        return Some(Route::Invalid);
    };

    let Some(header_sites) = curl_header_values(&tokens, curl_index).and_then(|headers| {
        headers
            .iter()
            .map(|header| inspect_curl_header_host(header))
            .filter_map(|inspection| match inspection {
                HostInspection::Ignored => None,
                HostInspection::Site(site) => Some(Some(site)),
                HostInspection::Invalid => Some(None),
            })
            .collect::<Option<Vec<_>>>()
    }) else {
        return Some(Route::Invalid);
    };
    if header_sites.iter().any(|site| *site != target_site) {
        return Some(Route::Invalid);
    }
    Some(Route::Site(target_site, RouteKind::Curl))
}

fn curl_cookie_value(raw: &str) -> Option<String> {
    let preprocessed = preprocessed_curl_shell_text(raw)?;
    let tokens = shell_tokens(&preprocessed);
    let curl_index = curl_command_index(&tokens)?;
    curl_header_values(&tokens, curl_index)?
        .into_iter()
        .find_map(|header| {
            let (name, value) = header.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("cookie")
                .then(|| value.trim().to_string())
        })
}

fn curl_command_index(tokens: &[String]) -> Option<usize> {
    let index = tokens
        .iter()
        .position(|token| !is_shell_assignment(token))?;
    is_curl_executable_token(&tokens[index]).then_some(index)
}

fn is_curl_executable_token(token: &str) -> bool {
    if token.contains('=') || token.contains("://") {
        return false;
    }
    let executable = token
        .split('/')
        .rfind(|segment| !segment.is_empty())
        .unwrap_or(token);
    executable.eq_ignore_ascii_case("curl")
}

fn contains_curl_executable_text(text: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    if shell_tokens(text)
        .iter()
        .any(|token| is_curl_executable_token(token))
    {
        return true;
    }
    PATTERN
        .get_or_init(|| {
            Regex::new(r"(?i)(^|[\s;])(?:[^\s;=]+/)?curl($|[\s;])").expect("curl text regex")
        })
        .is_match(text)
}

fn is_shell_assignment(token: &str) -> bool {
    if token.contains(';') {
        return false;
    }
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && name.chars().all(|c| c == '_' || c.is_alphanumeric())
}

fn is_token_text_safe(token: &str) -> bool {
    token.chars().all(is_text_safe)
}

fn is_text_safe(character: char) -> bool {
    let value = u32::from(character);
    value >= 0x20 && value != 0x7F
}

fn explicit_curl_url_targets(tokens: &[String], curl_index: usize) -> Option<Targets> {
    let mut targets = Targets::new();
    let mut index = curl_index + 1;
    while index < tokens.len() {
        let token = &tokens[index];
        let lowercased = token.to_ascii_lowercase();
        if lowercased == "--url" {
            let value_index = index + 1;
            let site = site_for_url_text(tokens.get(value_index)?)?;
            targets.push((value_index, site));
            index = value_index + 1;
            continue;
        }
        if lowercased.starts_with("--url=") {
            targets.push((index, site_for_url_text(&token["--url=".len()..])?));
        }
        index += 1;
    }
    Some(targets)
}

fn url_token_targets(tokens: &[String], curl_index: usize) -> Option<Targets> {
    let mut targets = Targets::new();
    for (index, token) in tokens.iter().enumerate().skip(curl_index + 1) {
        if let Some(site) = site_for_url_text(token) {
            targets.push((index, site));
        } else if host_for_url_text(token).is_some() {
            return None;
        }
    }
    Some(targets)
}

enum ShortHeader {
    Attached(String),
    NextToken,
    Invalid,
}

fn short_options_contain(token: &str, option: char) -> bool {
    token.starts_with('-') && !token.starts_with("--") && token[1..].contains(option)
}

fn short_header_value(token: &str) -> Option<ShortHeader> {
    if !token.starts_with('-') || token.starts_with("--") {
        return None;
    }
    let options = &token[1..];
    let header_option = options.find('H')?;
    if !options[..header_option]
        .chars()
        .all(|flag| "fsSL".contains(flag))
    {
        return Some(ShortHeader::Invalid);
    }
    let attached = &options[header_option + 1..];
    Some(if attached.is_empty() {
        ShortHeader::NextToken
    } else {
        ShortHeader::Attached(attached.to_string())
    })
}

/// Every `-H` / `--header` value in the command, or `None` when the command
/// can change its target or headers in ways this parser cannot see (config
/// files, variable expansion, header files, `--location-trusted`).
fn curl_header_values(tokens: &[String], curl_index: usize) -> Option<Vec<String>> {
    let mut values = Vec::new();
    let mut index = curl_index + 1;
    while index < tokens.len() {
        let token = &tokens[index];
        let lowercased = token.to_ascii_lowercase();
        if lowercased == "--config"
            || lowercased.starts_with("--config=")
            || lowercased.starts_with("--expand-")
            || lowercased == "--location-trusted"
            || short_options_contain(token, 'K')
        {
            return None;
        }
        if lowercased == "--header" {
            values.push(tokens.get(index + 1)?.clone());
            index += 2;
        } else if lowercased.starts_with("--header=") {
            values.push(token["--header=".len()..].to_string());
            index += 1;
        } else if let Some(short) = short_header_value(token) {
            match short {
                ShortHeader::Attached(value) => {
                    values.push(value);
                    index += 1;
                }
                ShortHeader::NextToken => {
                    values.push(tokens.get(index + 1)?.clone());
                    index += 2;
                }
                ShortHeader::Invalid => return None,
            }
        } else {
            index += 1;
        }
    }
    Some(values)
}

enum HostInspection {
    Ignored,
    Site(QoderSite),
    Invalid,
}

fn inspect_curl_header_host(header_value: &str) -> HostInspection {
    let trimmed = header_value.trim();
    if trimmed.is_empty() || trimmed.starts_with('@') {
        return HostInspection::Invalid;
    }
    let Some((name, value)) = trimmed.split_once(':') else {
        let lowercased = trimmed.to_lowercase();
        let looks_like_host = lowercased == "host"
            || lowercased.starts_with("host ")
            || lowercased.starts_with("host\t")
            || lowercased == "host;"
            || lowercased.starts_with("host;");
        return if looks_like_host {
            HostInspection::Invalid
        } else {
            HostInspection::Ignored
        };
    };
    if !name.trim().eq_ignore_ascii_case("host") {
        return HostInspection::Ignored;
    }
    site_for_host(value).map_or(HostInspection::Invalid, HostInspection::Site)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShellQuote {
    Single,
    Double,
}

/// Normalize line continuations and reject anything a shell would evaluate
/// (control operators, substitutions, expansions, control characters, or an
/// unterminated quote). `None` means the text is not a plain curl command.
fn preprocessed_curl_shell_text(text: &str) -> Option<String> {
    let scalars = text.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    let mut quote: Option<ShellQuote> = None;

    while index < scalars.len() {
        let scalar = scalars[index];
        let next = scalars.get(index + 1).copied();

        if scalar == '\\' {
            if quote.is_none() {
                if next == Some('\n') {
                    index += 2;
                    continue;
                }
                if next == Some('\r') && scalars.get(index + 2) == Some(&'\n') {
                    index += 3;
                    continue;
                }
            }
            if quote != Some(ShellQuote::Single)
                && let Some(escaped) = next
                && matches!(escaped, '\'' | '"' | '\\')
            {
                output.push(scalar);
                output.push(escaped);
                index += 2;
                continue;
            }
            if quote == Some(ShellQuote::Double) {
                return None;
            }
        }

        if !is_text_safe(scalar) {
            return None;
        }

        match quote {
            Some(ShellQuote::Single) => {
                if scalar == '\'' {
                    quote = None;
                }
            }
            Some(ShellQuote::Double) => {
                if scalar == '"' {
                    quote = None;
                } else if scalar == '`' || is_unsupported_dollar_expansion(&scalars, index) {
                    return None;
                }
            }
            None => {
                if ";|&<>".contains(scalar) {
                    return None;
                } else if scalar == '\'' {
                    quote = Some(ShellQuote::Single);
                } else if scalar == '"' {
                    quote = Some(ShellQuote::Double);
                } else if scalar == '`' || is_unsupported_dollar_expansion(&scalars, index) {
                    return None;
                }
            }
        }

        output.push(scalar);
        index += 1;
    }

    quote.is_none().then_some(output)
}

fn is_unsupported_dollar_expansion(scalars: &[char], index: usize) -> bool {
    if scalars[index] != '$' {
        return false;
    }
    let Some(next) = scalars.get(index + 1).copied() else {
        return false;
    };
    matches!(next, '\'' | '"' | '(' | '{' | '[')
        || next == '_'
        || next.is_alphabetic()
        || next.is_ascii_digit()
        || "*@#?$!-".contains(next)
}

fn shell_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for character in text.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
            continue;
        }
        if character.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(character);
    }

    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

// ---------------------------------------------------------------------------
// Hosts and URLs
// ---------------------------------------------------------------------------

fn host_for_url_text(text: &str) -> Option<String> {
    let trimmed = text.trim().trim_matches(['"', '\'']);
    let lowercased = trimmed.to_lowercase();
    if !(lowercased.starts_with("https://") || lowercased.starts_with("http://")) {
        return None;
    }
    reqwest::Url::parse(trimmed)
        .ok()?
        .host_str()
        .map(str::to_lowercase)
}

fn site_for_url_text(text: &str) -> Option<QoderSite> {
    site_for_host(&host_for_url_text(text)?)
}

/// Accepts `qoder.com`, `qoder.com.cn` and their `www.` forms, with an
/// optional leading dot, quotes, and a numeric port.
fn site_for_host(host: &str) -> Option<QoderSite> {
    let lowercased = host.trim().trim_matches(['"', '\'']).to_lowercase();
    let mut normalized = lowercased.strip_prefix('.').unwrap_or(&lowercased);
    if let Some((hostname, port)) = normalized.rsplit_once(':') {
        let valid_port = !hostname.contains(':')
            && !port.is_empty()
            && port.chars().all(|c| c.is_ascii_digit())
            && port
                .parse::<u32>()
                .is_ok_and(|port| (1..=65535).contains(&port));
        if !valid_port {
            return None;
        }
        normalized = hostname;
    }
    match normalized {
        "qoder.com" | "www.qoder.com" => Some(QoderSite::International),
        "qoder.com.cn" | "www.qoder.com.cn" => Some(QoderSite::China),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
