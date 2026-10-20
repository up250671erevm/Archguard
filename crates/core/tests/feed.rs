//! Repository-level feed tests with fixtures.

use archguard_core::news::{parse_feed, read_limited, MAX_FEED_BYTES};

#[test]
fn corrupt_rss_is_an_error_not_a_panic() {
    let bytes = include_bytes!("fixtures/corrupt.xml");
    assert!(parse_feed(bytes).is_err());
}

#[test]
fn garbage_and_empty_inputs() {
    assert!(parse_feed(b"").is_err());
    assert!(parse_feed(&[0xff, 0xfe, 0x00, 0x01]).is_err());
    assert!(parse_feed(b"<html><body>502 Bad Gateway</body></html>").is_err());
}

#[test]
fn good_fixture_parses() {
    let items = parse_feed(include_bytes!("fixtures/news.xml")).unwrap();
    assert_eq!(items.len(), 3);
}

#[test]
fn oversized_feed_rejected() {
    let big = vec![b' '; (MAX_FEED_BYTES + 1) as usize];
    assert!(read_limited(&big[..], MAX_FEED_BYTES).is_err());
}
