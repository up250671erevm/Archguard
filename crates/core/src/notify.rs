//! Desktop notification content (transport-agnostic).

use crate::check::Report;
use anyhow::{bail, Context, Result};

/// A desktop notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub summary: String,
    pub body: String,
    pub critical: bool,
}

/// Sends notifications (notify-rust in the binaries, mocks in tests).
pub trait Notifier {
    fn send(&self, n: &Notification) -> Result<()>;
}

/// Fallback transport via `notify-send` (libnotify).
pub struct NotifySend;

impl Notifier for NotifySend {
    fn send(&self, n: &Notification) -> Result<()> {
        let st = std::process::Command::new("notify-send")
            .args(["--app-name=archguard", "--icon=system-software-update"])
            .arg(format!(
                "--urgency={}",
                if n.critical { "critical" } else { "normal" }
            ))
            .arg("--")
            .arg(&n.summary)
            .arg(&n.body)
            .status()
            .context("running notify-send")?;
        if !st.success() {
            bail!("notify-send failed with {st}");
        }
        Ok(())
    }
}

/// Build the notification for a check result, or `None` if all is fine.
pub fn compose(report: &Report, pacnew: usize) -> Option<Notification> {
    let n = report.findings.len();
    if n == 0 && pacnew == 0 {
        return None;
    }
    let mut body = String::new();
    for f in report.findings.iter().take(5) {
        body.push_str("• ");
        body.push_str(f.item.display_title());
        body.push('\n');
    }
    if n > 5 {
        body.push_str(&format!("… and {} more\n", n - 5));
    }
    if pacnew > 0 {
        body.push_str(&format!(
            "{pacnew} .pacnew/.pacsave file(s) pending — run `archguard pacnew list`"
        ));
    }
    let summary = if n > 0 {
        format!("archguard: upgrade blocked by {n} news item(s)")
    } else {
        "archguard: configuration files need attention".to_string()
    };
    Some(Notification {
        summary,
        body: body.trim_end().to_string(),
        critical: n > 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::Finding;
    use crate::news::NewsItem;

    fn finding(t: &str) -> Finding {
        Finding {
            item: NewsItem {
                id: t.into(),
                title: t.into(),
                summary: String::new(),
                link: None,
                published: None,
            },
            packages: Default::default(),
        }
    }

    #[test]
    fn nothing_to_say() {
        assert_eq!(compose(&Report::default(), 0), None);
    }

    #[test]
    fn blocking_is_critical_and_truncated() {
        let r = Report {
            findings: (0..7).map(|i| finding(&format!("n{i}"))).collect(),
        };
        let n = compose(&r, 2).unwrap();
        assert!(n.critical);
        assert!(n.summary.contains("7 news"));
        assert!(n.body.contains("• n4") && !n.body.contains("• n5"));
        assert!(n.body.contains("and 2 more"));
        assert!(n.body.contains("2 .pacnew"));
    }

    #[test]
    fn pacnew_only_is_normal() {
        let n = compose(&Report::default(), 1).unwrap();
        assert!(!n.critical);
        assert!(n.summary.contains("configuration"));
    }
}
