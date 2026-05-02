//! Body-selection and truncation rules for full-text documents.
//!
//! Spec 08 §Body selection enforces a deterministic order:
//!
//! 1. Raw payload >4 KB **and** classifier summary present ⇒ use the
//!    summary as `body`.
//! 2. Otherwise use the redacted raw payload text.
//! 3. Empty body after redaction ⇒ skip the event (counted but not
//!    indexed).
//!
//! The body is then truncated to `max_body_bytes`; truncation flips
//! the document's `truncated` flag so the dashboard can surface
//! tail-loss accurately.

/// Spec 08 cut-off above which the classifier summary, if present, is
/// preferred over the raw payload.
pub const OVERSIZE_BODY_BYTES: usize = 4 * 1024;

/// Outcome of running the body-selection rule on one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodySource {
    /// Took the classifier-supplied summary because the raw payload
    /// exceeded `OVERSIZE_BODY_BYTES`.
    Summary,
    /// Took the redacted raw text directly.
    Raw,
    /// Both summary and raw produced empty strings — caller drops the
    /// event with a counter increment per spec 08 §Failure modes.
    Empty,
}

/// Result of body selection — the chosen text plus metadata about
/// which branch fired and whether truncation kicked in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedBody {
    /// The chosen text after truncation to `max_body_bytes`.
    pub body: String,
    /// Which selection branch produced the body.
    pub source: BodySource,
    /// `true` when truncation discarded part of the original input.
    pub truncated: bool,
}

/// Pick the document body following the spec-08 rule.
///
/// `raw` is the redacted payload text. `summary` is the classifier's
/// optional short summary. `max_body_bytes` is the per-document size
/// cap (default 10 MB, configurable on bootstrap restores).
pub fn select_body(raw: &str, summary: Option<&str>, max_body_bytes: usize) -> SelectedBody {
    let raw_len = raw.len();
    let oversize = raw_len > OVERSIZE_BODY_BYTES;

    let (chosen, source) = if oversize {
        match summary {
            Some(s) if !s.trim().is_empty() => (s.to_string(), BodySource::Summary),
            _ => (raw.to_string(), BodySource::Raw),
        }
    } else if !raw.trim().is_empty() {
        (raw.to_string(), BodySource::Raw)
    } else {
        return SelectedBody {
            body: String::new(),
            source: BodySource::Empty,
            truncated: false,
        };
    };

    let (body, truncated) = truncate_to(chosen, max_body_bytes);
    SelectedBody {
        body,
        source,
        truncated,
    }
}

/// Truncate `text` to at most `max_bytes` bytes, splitting on a UTF-8
/// boundary so the result is always valid UTF-8. Returns the truncated
/// string plus a flag indicating whether anything was dropped.
fn truncate_to(text: String, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    // Walk back from `max_bytes` to the nearest char boundary.
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut truncated = text;
    truncated.truncate(cut);
    (truncated, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_raw_uses_raw_directly() {
        let out = select_body("hello world", None, 1024);
        assert_eq!(out.body, "hello world");
        assert_eq!(out.source, BodySource::Raw);
        assert!(!out.truncated);
    }

    #[test]
    fn oversize_with_summary_uses_summary() {
        let raw = "x".repeat(OVERSIZE_BODY_BYTES + 1);
        let out = select_body(&raw, Some("brief summary"), 1024);
        assert_eq!(out.body, "brief summary");
        assert_eq!(out.source, BodySource::Summary);
        assert!(!out.truncated);
    }

    #[test]
    fn oversize_without_summary_falls_back_to_raw() {
        let raw = "x".repeat(OVERSIZE_BODY_BYTES + 1);
        let out = select_body(&raw, None, 1024 * 1024);
        assert_eq!(out.source, BodySource::Raw);
        assert_eq!(out.body.len(), OVERSIZE_BODY_BYTES + 1);
    }

    #[test]
    fn empty_raw_returns_empty_branch() {
        let out = select_body("", None, 1024);
        assert_eq!(out.source, BodySource::Empty);
        assert!(out.body.is_empty());
    }

    #[test]
    fn whitespace_only_raw_returns_empty_branch() {
        let out = select_body("   \n\t", None, 1024);
        assert_eq!(out.source, BodySource::Empty);
    }

    #[test]
    fn body_is_truncated_to_max_bytes() {
        let raw = "y".repeat(50);
        let out = select_body(&raw, None, 10);
        assert_eq!(out.body.len(), 10);
        assert!(out.truncated);
    }

    #[test]
    fn truncation_respects_utf8_boundary() {
        // Each `é` is two UTF-8 bytes — cutting at byte 5 would split
        // the third codepoint, so the truncator backs off to byte 4.
        let raw = "ééé";
        let (body, truncated) = truncate_to(raw.to_string(), 5);
        assert!(truncated);
        assert_eq!(body, "éé");
    }
}
