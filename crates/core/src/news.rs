//! Arch news feed fetching and parsing.

use anyhow::{bail, Context, Result};
use std::io::Read;
use std::time::Duration;

/// Maximum accepted feed size (the real Arch feed is ~100 KiB).
pub const MAX_FEED_BYTES: u64 = 4 * 1024 * 1024;

/// Read at most `max` bytes; error if the source is larger.
pub fn read_limited<R: Read>(r: R, max: u64) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    r.take(max + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max {
        bail!("news feed exceeds {max} bytes");
    }
    Ok(buf)
}

/// A single news item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsItem {
    /// Stable identifier (feed GUID, usually the article URL).
    pub id: String,
    pub title: String,
    /// Plain-ish text body (HTML tags stripped).
    pub summary: String,
    pub link: Option<String>,
    /// Unix timestamp (seconds), if the feed provides one.
    pub published: Option<i64>,
}

impl NewsItem {
    /// Short human-friendly id: last path segment of the id/link (the slug).
    pub fn short_id(&self) -> &str {
        let s = self.id.trim_end_matches('/');
        s.rsplit('/').next().filter(|x| !x.is_empty()).unwrap_or(s)
    }
}

/// Very small HTML tag stripper sufficient for matching purposes.
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Parse an RSS/Atom document into news items.
pub fn parse_feed(bytes: &[u8]) -> Result<Vec<NewsItem>> {
    let feed = feed_rs::parser::parse(bytes).context("failed to parse news feed")?;
    Ok(feed
        .entries
        .into_iter()
        .filter_map(|e| {
            let link = e.links.first().map(|l| l.href.clone());
            let summary = e
                .summary
                .map(|t| t.content)
                .or_else(|| e.content.and_then(|c| c.body))
                .unwrap_or_default();
            let title = e.title.map(|t| t.content).unwrap_or_default();
            if title.trim().is_empty() && link.is_none() {
                return None; // nothing a user could read or ack meaningfully
            }
            let published = e.published.or(e.updated).map(|d| d.timestamp());
            // Empty GUIDs: fall back to link, then title+date; skip if nothing usable.
            let id = [Some(e.id), link.clone()]
                .into_iter()
                .flatten()
                .map(|s| s.trim().to_string())
                .find(|s| !s.is_empty())
                .or_else(|| {
                    (!title.trim().is_empty())
                        .then(|| format!("title:{}:{}", published.unwrap_or(0), title.trim()))
                })?;
            Some(NewsItem {
                id,
                title,
                summary: strip_html(&summary),
                link,
                published,
            })
        })
        .collect())
}

/// Source of news items (real HTTP or a fixture in tests).
pub trait NewsSource {
    fn fetch(&self) -> Result<Vec<NewsItem>>;
}

/// Fetches the feed over HTTP(S).
pub struct HttpNews {
    pub url: String,
}

impl NewsSource for HttpNews {
    fn fetch(&self) -> Result<Vec<NewsItem>> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("archguard/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let resp = client
            .get(&self.url)
            .send()
            .and_then(|r| r.error_for_status())
            .with_context(|| format!("fetching {}", self.url))?;
        if resp.content_length().is_some_and(|l| l > MAX_FEED_BYTES) {
            bail!("news feed exceeds {MAX_FEED_BYTES} bytes");
        }
        let bytes = read_limited(resp, MAX_FEED_BYTES)?;
        parse_feed(&bytes)
    }
}

/// Static list of items (tests / offline use).
pub struct StaticNews(pub Vec<NewsItem>);

impl NewsSource for StaticNews {
    fn fetch(&self) -> Result<Vec<NewsItem>> {
        Ok(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/news.xml");

    #[test]
    fn parses_fixture() {
        let items = parse_feed(FIXTURE).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title, "foolib >= 2.0 requires manual intervention");
        assert_eq!(
            items[0].short_id(),
            "foolib-20-requires-manual-intervention"
        );
        assert!(items[0].summary.contains("rm /usr/lib/foolib.cache"));
        assert!(!items[0].summary.contains("<p>"));
        assert!(items[0].published.is_some());
    }

    #[test]
    fn strip_html_works() {
        assert_eq!(strip_html("<p>a &amp; b</p>").trim(), "a & b");
    }

    #[test]
    fn size_limit() {
        assert_eq!(read_limited(&b"abcd"[..], 4).unwrap(), b"abcd");
        assert!(read_limited(&b"abcde"[..], 4).is_err());
    }

    #[test]
    fn empty_ids_fallback_or_skip() {
        let xml = br#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
<item><title>No guid</title><link>https://archlinux.org/news/no-guid/</link></item>
<item><title>Only title</title><guid>  </guid></item>
<item><description>nothing</description></item>
</channel></rss>"#;
        let items = parse_feed(xml).unwrap();
        assert!(items.iter().all(|i| !i.id.trim().is_empty()));
        let ids: std::collections::BTreeSet<_> = items.iter().map(|i| &i.id).collect();
        assert_eq!(ids.len(), items.len(), "ids must be unique: {items:?}");
        assert!(items.iter().any(|i| i.title == "No guid"));
        assert!(items.iter().any(|i| i.title == "Only title"));
        assert!(items.iter().all(|i| !i.title.is_empty()));
    }

    #[test]
    fn invalid_feed_errors() {
        assert!(parse_feed(b"not xml").is_err());
    }

    #[test]
    fn short_id_fallback() {
        let mut n = NewsItem {
            id: "plain".into(),
            title: String::new(),
            summary: String::new(),
            link: None,
            published: None,
        };
        assert_eq!(n.short_id(), "plain");
        n.id = "https://a/b/c/".into();
        assert_eq!(n.short_id(), "c");
    }
}
