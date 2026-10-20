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
        let src = if self.id.starts_with("archguard:") {
            self.link.as_deref().unwrap_or(&self.id)
        } else {
            &self.id
        };
        let s = src.trim_end_matches('/');
        s.rsplit('/').next().filter(|x| !x.is_empty()).unwrap_or(s)
    }
}

impl NewsItem {
    /// Items without a usable title can't be matched; they are treated as
    /// potentially relevant (fail-closed).
    pub fn is_untitled(&self) -> bool {
        self.title.trim().is_empty()
    }

    /// Title for display.
    pub fn display_title(&self) -> &str {
        if self.is_untitled() {
            "(untitled news item — review manually)"
        } else {
            &self.title
        }
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

/// For each `<item>` / `<entry>` in document order, whether the *source*
/// carries its own non-empty identifier (`<guid>`, `<id>` or `rdf:about`).
/// feed-rs synthesises a random id when none exists, so we must look at the
/// raw XML. Returns `None` if the document can't be scanned.
pub fn raw_has_guid(bytes: &[u8]) -> Option<Vec<bool>> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    // depth relative to the current item (1 = direct child)
    let mut item_depth: Option<usize> = None;
    let mut in_id = false;
    let mut has = false;
    loop {
        match reader.read_event_into(&mut buf).ok()? {
            Event::Start(e) => {
                let name = e.local_name();
                let name = name.as_ref();
                match item_depth {
                    None if name == b"item" || name == b"entry" => {
                        item_depth = Some(0);
                        has = e.attributes().flatten().any(|a| {
                            a.key.local_name().as_ref() == b"about"
                                && a.value.iter().any(|c| !c.is_ascii_whitespace())
                        });
                    }
                    Some(d) => {
                        if d == 0 && (name == b"guid" || name == b"id") {
                            in_id = true;
                        }
                        item_depth = Some(d + 1);
                    }
                    None => {}
                }
            }
            Event::Empty(e) => {
                let name = e.local_name();
                if item_depth.is_none() && matches!(name.as_ref(), b"item" | b"entry") {
                    out.push(false);
                }
            }
            Event::Text(t) if in_id => {
                if AsRef::<[u8]>::as_ref(&*t)
                    .iter()
                    .any(|c| !c.is_ascii_whitespace())
                {
                    has = true;
                }
            }
            Event::CData(t) if in_id => {
                if AsRef::<[u8]>::as_ref(&*t)
                    .iter()
                    .any(|c| !c.is_ascii_whitespace())
                {
                    has = true;
                }
            }
            Event::End(_) => match item_depth {
                Some(0) => {
                    out.push(has);
                    item_depth = None;
                    has = false;
                }
                Some(d) => {
                    if d == 1 {
                        in_id = false;
                    }
                    item_depth = Some(d - 1);
                }
                None => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Some(out)
}

/// Deterministic id for items without a source GUID: SHA-256 of the link,
/// or of title + published + summary when there is no link.
pub fn stable_id(link: Option<&str>, title: &str, published: Option<i64>, summary: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    match link.map(str::trim).filter(|l| !l.is_empty()) {
        Some(l) => {
            h.update(b"link\0");
            h.update(l.as_bytes());
        }
        None => {
            h.update(b"content\0");
            h.update(title.trim().as_bytes());
            h.update(b"\0");
            h.update(published.unwrap_or(0).to_string().as_bytes());
            h.update(b"\0");
            h.update(summary.trim().as_bytes());
        }
    }
    let hex: String = h
        .finalize()
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("archguard:{hex}")
}

/// Parse an RSS/Atom document into news items. Items without a GUID in the
/// source get a deterministic [`stable_id`] so acknowledgements persist.
/// Fail-closed: items are never dropped.
pub fn parse_feed(bytes: &[u8]) -> Result<Vec<NewsItem>> {
    let feed = feed_rs::parser::parse(bytes).context("failed to parse news feed")?;
    // If the raw scan disagrees with feed-rs, trust nothing: derive all ids.
    let guids = raw_has_guid(bytes).filter(|g| g.len() == feed.entries.len());
    Ok(feed
        .entries
        .into_iter()
        .enumerate()
        .map(|(idx, e)| {
            let link = e.links.first().map(|l| l.href.clone());
            let summary = strip_html(
                &e.summary
                    .map(|t| t.content)
                    .or_else(|| e.content.and_then(|c| c.body))
                    .unwrap_or_default(),
            );
            let title = e.title.map(|t| t.content).unwrap_or_default();
            let published = e.published.or(e.updated).map(|d| d.timestamp());
            let source_guid = guids.as_ref().is_some_and(|g| g[idx]) && !e.id.trim().is_empty();
            let id = if source_guid {
                e.id.trim().to_string()
            } else {
                stable_id(link.as_deref(), &title, published, &summary)
            };
            NewsItem {
                id,
                title,
                summary,
                link,
                published,
            }
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

    const NO_GUID: &[u8] = include_bytes!("../tests/fixtures/news_noguid.xml");

    #[test]
    fn missing_guid_ids_are_stable_across_parses() {
        let a = parse_feed(NO_GUID).unwrap();
        let b = parse_feed(NO_GUID).unwrap();
        assert_eq!(a.len(), 4);
        let ids: Vec<_> = a.iter().map(|i| i.id.clone()).collect();
        assert_eq!(ids, b.iter().map(|i| i.id.clone()).collect::<Vec<_>>());
        // Item with a real guid keeps it verbatim.
        assert_eq!(a[0].id, "tag:archlinux.org,2026:/news/has-guid/");
        // Others get deterministic, distinct ids.
        assert!(a[1..].iter().all(|i| i.id.starts_with("archguard:")));
        let uniq: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(uniq.len(), 4);
        // Link-based id ignores title edits; slug still usable for `ack`.
        assert_eq!(a[1].id, stable_id(a[1].link.as_deref(), "other", None, ""));
        assert_eq!(a[1].short_id(), "no-guid-with-link");
    }

    #[test]
    fn raw_guid_scan() {
        assert_eq!(raw_has_guid(NO_GUID).unwrap(), [true, false, false, false]);
        let atom = br#"<feed xmlns="http://www.w3.org/2005/Atom"><title>t</title>
<entry><id>urn:x</id><title>a</title><source><id>urn:src</id></source></entry>
<entry><title>b</title><source><id>urn:src</id></source></entry></feed>"#;
        assert_eq!(raw_has_guid(atom).unwrap(), [true, false]);
        let rdf = br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><item rdf:about="http://x/1"><title>a</title></item></rdf:RDF>"#;
        assert_eq!(raw_has_guid(rdf).unwrap(), [true]);
    }

    #[test]
    fn stable_id_inputs() {
        let a = stable_id(None, "t", Some(1), "s");
        assert_eq!(a, stable_id(None, " t ", Some(1), "s"));
        assert_ne!(a, stable_id(None, "t", Some(2), "s"));
        assert_ne!(a, stable_id(None, "t", Some(1), "x"));
        assert_ne!(stable_id(Some("l"), "", None, ""), a);
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
        // Fail-closed: the item without title/link is kept, not dropped.
        assert_eq!(items.len(), 3);
        assert_eq!(items.iter().filter(|i| i.is_untitled()).count(), 1);
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
