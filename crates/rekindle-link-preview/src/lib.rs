//! Architecture §28.8 — OpenGraph link preview fetcher.
//!
//! Single public async function: [`fetch_link_preview`]. Strict
//! constraints because this code makes outbound HTTP to attacker-
//! controlled hosts:
//!
//! * 5-second response timeout.
//! * 256 KB body cap (read up to that, then stop).
//! * `User-Agent: Rekindle-LinkPreview/1.0 (+https://rekindle.app)`.
//! * No redirects to non-`http(s)` schemes; max 5 hops.
//! * Returns plain text/html only.

use std::time::Duration;

use rekindle_types::link_preview::LinkPreview;
use thiserror::Error;
use url::Url;

const MAX_BODY_BYTES: usize = 256 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const USER_AGENT: &str = "Rekindle-LinkPreview/1.0 (+https://rekindle.app)";

#[derive(Debug, Error)]
pub enum LinkPreviewError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("URL scheme must be http or https")]
    BadScheme,
    #[error("HTTP fetch failed: {0}")]
    Http(String),
    #[error("response is not text/html")]
    NotHtml,
    #[error("body exceeded {MAX_BODY_BYTES} bytes")]
    BodyTooLarge,
}

pub async fn fetch_link_preview(
    url: &str,
    message_id: &str,
) -> Result<LinkPreview, LinkPreviewError> {
    let parsed = Url::parse(url).map_err(|e| LinkPreviewError::InvalidUrl(e.to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(LinkPreviewError::BadScheme);
    }

    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(5))
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

    Ok(LinkPreview {
        message_id: message_id.to_string(),
        url: parsed.to_string(),
        title: metadata.title,
        description: metadata.description,
        image_url: metadata.image_url,
        site_name: metadata.site_name,
        fetched_at: now_unix_ms(),
    })
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
    image_url: Option<String>,
    site_name: Option<String>,
}

/// Lightweight OpenGraph extractor. We avoid a full HTML parser
/// because the surface area we care about is narrow (5 specific meta
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
    if let Some(v) = extract_meta_content(html, "og:image") {
        m.image_url = Some(v);
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
mod tests {
    use super::*;

    #[test]
    fn extracts_basic_open_graph() {
        let html = r#"
            <html><head>
                <title>Page Title</title>
                <meta property="og:title" content="OG Title">
                <meta property="og:description" content="OG Description">
                <meta property="og:image" content="https://example.com/image.png">
                <meta property="og:site_name" content="Example Site">
            </head></html>
        "#;
        let m = extract_open_graph(html);
        assert_eq!(m.title.as_deref(), Some("OG Title"));
        assert_eq!(m.description.as_deref(), Some("OG Description"));
        assert_eq!(m.image_url.as_deref(), Some("https://example.com/image.png"));
        assert_eq!(m.site_name.as_deref(), Some("Example Site"));
    }

    #[test]
    fn falls_back_to_title_tag_and_meta_description() {
        let html = r#"
            <html><head>
                <title>Plain Title</title>
                <meta name="description" content="Plain description">
            </head></html>
        "#;
        let m = extract_open_graph(html);
        assert_eq!(m.title.as_deref(), Some("Plain Title"));
        assert_eq!(m.description.as_deref(), Some("Plain description"));
    }

    #[test]
    fn handles_attribute_order_swapped() {
        let html = r#"<meta content="Swapped" property="og:title">"#;
        let m = extract_open_graph(html);
        assert_eq!(m.title.as_deref(), Some("Swapped"));
    }

    #[test]
    fn handles_single_quotes() {
        let html = r"<meta property='og:title' content='Single'>";
        let m = extract_open_graph(html);
        assert_eq!(m.title.as_deref(), Some("Single"));
    }

    #[test]
    fn decodes_basic_html_entities() {
        let html = r#"<meta property="og:title" content="Tom &amp; Jerry"#.to_owned()
            + r#"">"#;
        let m = extract_open_graph(&html);
        assert_eq!(m.title.as_deref(), Some("Tom & Jerry"));
    }
}
