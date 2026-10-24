//! Architecture §28.8 — OpenGraph link preview fetcher.
//!
//! Single public async function: [`fetch_link_preview`]. Strict
//! constraints because this code makes outbound HTTP to attacker-
//! controlled hosts:
//!
//! * 5-second response timeout.
//! * 256 KB body cap (read up to that, then stop).
//! * A neutral `User-Agent` ([`USER_AGENT`]): a previewed site must not
//!   learn that the sender runs Rekindle. No cookies, no `Accept-Language`.
//! * Only a [`fetchable_url`]: `https`, and no `.onion`, `.i2p`,
//!   localhost, reserved (`test`, `invalid`, `example`), disallowed IP
//!   literal or mixed-script (IDN homograph) host. Every redirect hop is
//!   re-checked the same way ([`redirect_allowed`]), at most 5 hops.
//! * No `og:image`: receivers would fetch it and reveal their IP.
//! * Returns plain text/html only.
//! * SSRF guard ([`SsrfGuardedResolver`]): every resolved address — the
//!   initial request and each of the up-to-5 redirect hops, since
//!   reqwest re-resolves on each one — is checked against loopback,
//!   private, link-local (which covers the `169.254.169.254` cloud
//!   metadata endpoint), unspecified, and multicast ranges before a
//!   connection is made. This is sender-side: the composing user's own
//!   client is what fetches a URL they typed, gated by the `EMBED_LINKS`
//!   permission; without this guard that client could be pointed at an
//!   internal service or cloud metadata endpoint, directly or via a
//!   public URL that redirects.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use rekindle_types::link_preview::LinkPreview;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use thiserror::Error;
use url::Url;

const MAX_BODY_BYTES: usize = 256 * 1024;
/// Longest URL Rekindle fetches, shows or opens.
pub const MAX_URL_LEN: usize = 2048;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// The User-Agent every preview fetch sends. Signal sends this string so a
/// previewed site serves its OpenGraph tags without learning which app
/// fetched them (prod research batch 7, L4).
pub const USER_AGENT: &str = "WhatsApp/2";
/// Most redirect hops a preview fetch follows.
const MAX_REDIRECTS: usize = 5;
/// Top-level names a preview never fetches: hidden-service networks,
/// localhost and the RFC 2606 / RFC 6761 reserved names.
const REFUSED_TLDS: [&str; 6] = ["onion", "i2p", "localhost", "test", "invalid", "example"];

#[derive(Debug, Error)]
pub enum LinkPreviewError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("URL must be https")]
    BadScheme,
    #[error("URL is {got} bytes, max {MAX_URL_LEN}")]
    TooLong { got: usize },
    #[error("URL has no host")]
    NoHost,
    #[error("host {0} is never fetched")]
    RefusedHost(String),
    #[error("more than {MAX_REDIRECTS} redirects")]
    TooManyRedirects,
    #[error("HTTP fetch failed: {0}")]
    Http(String),
    #[error("response is not text/html")]
    NotHtml,
    #[error("body exceeded {MAX_BODY_BYTES} bytes")]
    BodyTooLarge,
}

/// DNS resolver that refuses to hand back addresses in loopback,
/// private, link-local, unspecified, or multicast ranges — the classic
/// SSRF targets. `169.254.169.254` (the AWS/GCP/Azure cloud metadata
/// endpoint) falls under IPv4 link-local, so `is_link_local()` alone
/// covers it without a separate special case.
///
/// `reqwest::Client::dns_resolver` is called for the initial request
/// AND for every redirect hop, so wiring the check in here (rather than
/// once against the original parsed URL) is what makes it survive a
/// public-looking URL that redirects to an internal address.
#[derive(Debug, Default)]
struct SsrfGuardedResolver;

impl Resolve for SsrfGuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let resolved: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|e| format!("DNS resolution for {host} failed: {e}"))?
                .collect();
            let safe: Vec<SocketAddr> = resolved
                .into_iter()
                .filter(|addr| !is_disallowed_target(&addr.ip()))
                .collect();
            if safe.is_empty() {
                return Err(format!(
                    "SSRF guard: {host} resolved only to disallowed (loopback/private/\
                     link-local/unspecified/multicast) addresses — refusing to connect"
                )
                .into());
            }
            Ok(Box::new(safe.into_iter()) as Addrs)
        })
    }
}

/// Whether `ip` falls in a range link-preview fetches must never reach.
fn is_disallowed_target(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_disallowed_v4(v4),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unique_local() // fc00::/7
                || v6.is_unicast_link_local() // fe80::/10
                || v6.to_ipv4_mapped().is_some_and(|v4| is_disallowed_v4(&v4))
        }
    }
}

fn is_disallowed_v4(v4: &Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local() // covers 169.254.169.254 cloud metadata
        || v4.is_unspecified()
        || v4.is_multicast()
        || v4.is_broadcast()
}

/// Parse a URL Rekindle may fetch, show or open: `https`, with a host, at
/// most [`MAX_URL_LEN`] bytes. Plain `http` is refused: a preview or a
/// click must not leak the user's IP and the page over cleartext.
pub fn https_url(url: &str) -> Result<Url, LinkPreviewError> {
    if url.len() > MAX_URL_LEN {
        return Err(LinkPreviewError::TooLong { got: url.len() });
    }
    let parsed = Url::parse(url).map_err(|e| LinkPreviewError::InvalidUrl(e.to_string()))?;
    if parsed.scheme() != "https" {
        return Err(LinkPreviewError::BadScheme);
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err(LinkPreviewError::NoHost);
    }
    Ok(parsed)
}

/// Parse a URL a link preview may fetch: an [`https_url`] whose host is
/// not refused. Refused: a disallowed IP literal (reqwest connects to an IP
/// literal without asking [`SsrfGuardedResolver`]), a name under one of
/// [`REFUSED_TLDS`] (so `localhost` itself), and a mixed-script name, one
/// that mixes ASCII and non-ASCII characters once its punycode is decoded,
/// which Signal refuses as an IDN homograph.
pub fn fetchable_url(url: &str) -> Result<Url, LinkPreviewError> {
    let parsed = https_url(url)?;
    let refused = |host: &str| Err(LinkPreviewError::RefusedHost(host.to_string()));
    match parsed.host() {
        Some(url::Host::Ipv4(v4)) if is_disallowed_v4(&v4) => return refused(&v4.to_string()),
        Some(url::Host::Ipv6(v6)) if is_disallowed_target(&IpAddr::V6(v6)) => {
            return refused(&v6.to_string());
        }
        Some(url::Host::Domain(domain)) => {
            let name = domain.trim_end_matches('.').to_ascii_lowercase();
            let tld = name.rsplit('.').next().unwrap_or_default();
            if REFUSED_TLDS.contains(&tld) {
                return refused(domain);
            }
            let unicode = url::quirks::domain_to_unicode(&name);
            if !unicode.is_ascii() && unicode.chars().any(|c| c.is_ascii_alphanumeric()) {
                return refused(domain);
            }
        }
        _ => {}
    }
    Ok(parsed)
}

/// Whether a fetch that has already followed `hops` redirects may follow
/// one more to `next`: under [`MAX_REDIRECTS`], and `next` a
/// [`fetchable_url`].
pub fn redirect_allowed(hops: usize, next: &Url) -> Result<(), LinkPreviewError> {
    if hops >= MAX_REDIRECTS {
        return Err(LinkPreviewError::TooManyRedirects);
    }
    fetchable_url(next.as_str()).map(drop)
}

/// Longest preview title kept, in chars.
const MAX_TITLE_CHARS: usize = 200;
/// Longest preview description kept, in chars.
const MAX_DESCRIPTION_CHARS: usize = 500;
/// Longest preview site name kept, in chars.
const MAX_SITE_NAME_CHARS: usize = 100;

/// Strip control and bidi characters, trim, cap at `max` chars; `None` if
/// nothing is left.
fn clean_text(text: Option<String>, max: usize) -> Option<String> {
    let is_bidi = |c: char| matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    let cleaned: String = text?
        .chars()
        .filter(|&c| !c.is_control() && !is_bidi(c))
        .collect::<String>()
        .trim()
        .chars()
        .take(max)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Normalize a preview so it is safe to show: an [`https_url`], and
/// cleaned, length-capped text. Applied to every preview Rekindle builds
/// and every preview a peer sends; `None` drops the preview.
#[must_use]
pub fn accept_inbound(preview: LinkPreview) -> Option<LinkPreview> {
    let url = https_url(&preview.url).ok()?;
    Some(LinkPreview {
        message_id: preview.message_id,
        url: url.to_string(),
        title: clean_text(preview.title, MAX_TITLE_CHARS),
        description: clean_text(preview.description, MAX_DESCRIPTION_CHARS),
        site_name: clean_text(preview.site_name, MAX_SITE_NAME_CHARS),
        fetched_at: preview.fetched_at,
    })
}

/// Fetch the OpenGraph text metadata for an https URL. No image is taken:
/// receivers would have to fetch it, revealing their IP to the site.
pub async fn fetch_link_preview(
    url: &str,
    message_id: &str,
) -> Result<LinkPreview, LinkPreviewError> {
    let parsed = fetchable_url(url)?;

    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::custom(
            |attempt| match redirect_allowed(attempt.previous().len(), attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(e) => attempt.error(e),
            },
        ))
        .dns_resolver(Arc::new(SsrfGuardedResolver))
        .build()
        .map_err(|e| LinkPreviewError::Http(e.to_string()))?;

    let response = client
        .get(parsed.clone())
        .send()
        .await
        .map_err(|e| LinkPreviewError::Http(e.to_string()))?;

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.starts_with("text/html") {
        return Err(LinkPreviewError::NotHtml);
    }

    let bytes = read_capped_body(response).await?;
    let html = String::from_utf8_lossy(&bytes);
    let metadata = extract_open_graph(&html);

    accept_inbound(LinkPreview {
        message_id: message_id.to_string(),
        url: parsed.to_string(),
        title: metadata.title,
        description: metadata.description,
        site_name: metadata.site_name,
        fetched_at: now_unix_ms(),
    })
    .ok_or(LinkPreviewError::BadScheme)
}

async fn read_capped_body(response: reqwest::Response) -> Result<Vec<u8>, LinkPreviewError> {
    use futures::StreamExt as _;

    let mut stream = response.bytes_stream();
    let mut buf = Vec::with_capacity(8 * 1024);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| LinkPreviewError::Http(e.to_string()))?;
        if buf.len() + chunk.len() > MAX_BODY_BYTES {
            // We have enough to extract metadata from the head; bail.
            buf.extend_from_slice(&chunk[..MAX_BODY_BYTES.saturating_sub(buf.len())]);
            return Ok(buf);
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

#[derive(Default)]
struct Metadata {
    title: Option<String>,
    description: Option<String>,
    site_name: Option<String>,
}

/// Lightweight OpenGraph extractor. We avoid a full HTML parser
/// because the surface area we care about is narrow (4 specific meta
/// tags + the `<title>` element). Tag-name matching is case-insensitive
/// and quote-style-tolerant; attributes inside `<meta>` may appear in
/// either order.
fn extract_open_graph(html: &str) -> Metadata {
    let mut m = Metadata::default();
    if let Some(title) = extract_title(html) {
        m.title = Some(title);
    }
    if let Some(v) = extract_meta_content(html, "og:title") {
        m.title = Some(v);
    }
    if let Some(v) = extract_meta_content(html, "og:description") {
        m.description = Some(v);
    } else if let Some(v) = extract_meta_content(html, "description") {
        m.description = Some(v);
    }
    if let Some(v) = extract_meta_content(html, "og:site_name") {
        m.site_name = Some(v);
    }
    m
}

fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start_tag = lower.find("<title")?;
    let close = lower[start_tag..].find('>')? + start_tag + 1;
    let end = lower[close..].find("</title>")? + close;
    let title = html.get(close..end)?.trim().to_string();
    if title.is_empty() {
        None
    } else {
        Some(decode_html_entities(&title))
    }
}

/// Find `<meta property="X" content="Y">` (or `name="X"` variant) and
/// return Y. Supports either attribute order and either single or
/// double quotes around the values.
fn extract_meta_content(html: &str, key: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let key_lower = key.to_ascii_lowercase();
    let mut search_start = 0;
    while let Some(meta_pos) = lower[search_start..].find("<meta") {
        let abs_meta = search_start + meta_pos;
        let close = lower[abs_meta..].find('>').map(|i| abs_meta + i)?;
        let tag_lower = &lower[abs_meta..close];
        let tag_orig = &html[abs_meta..close];
        // Match on either property="og:foo" or name="og:foo".
        let has_key = tag_has_attr_value(tag_lower, "property", &key_lower)
            || tag_has_attr_value(tag_lower, "name", &key_lower);
        if has_key {
            if let Some(content) = extract_attr_value(tag_orig, "content") {
                return Some(decode_html_entities(content.trim()));
            }
        }
        search_start = close + 1;
    }
    None
}

fn tag_has_attr_value(tag_lower: &str, attr: &str, value_lower: &str) -> bool {
    let needle_eq = format!("{attr}=");
    let mut search = 0;
    while let Some(idx) = tag_lower[search..].find(&needle_eq) {
        let abs = search + idx + needle_eq.len();
        if let Some(quote_char) = tag_lower.as_bytes().get(abs) {
            let quote = *quote_char as char;
            if quote == '"' || quote == '\'' {
                let val_start = abs + 1;
                if let Some(end) = tag_lower[val_start..].find(quote) {
                    if &tag_lower[val_start..val_start + end] == value_lower {
                        return true;
                    }
                }
            }
        }
        search = abs;
    }
    false
}

fn extract_attr_value<'a>(tag_orig: &'a str, attr: &str) -> Option<&'a str> {
    let tag_lower = tag_orig.to_ascii_lowercase();
    let attr_lower = attr.to_ascii_lowercase();
    let needle = format!("{attr_lower}=");
    let idx = tag_lower.find(&needle)?;
    let abs = idx + needle.len();
    let quote = *tag_orig.as_bytes().get(abs)? as char;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let val_start = abs + 1;
    let end = tag_orig[val_start..].find(quote)? + val_start;
    Some(&tag_orig[val_start..end])
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

fn now_unix_ms() -> u64 {
    rekindle_utils::time::timestamp_ms()
}

#[cfg(test)]
mod tests;
