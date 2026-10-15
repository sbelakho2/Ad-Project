//! The status vocabulary: one chip component and one severity mapping.
//!
//! Status always includes a text label (never color alone). The labels are
//! the design document's: `Read-only`, `Verified`, `Needs attention`,
//! `Failed`, `Not available`, `Running`, `Completed`, `Queued`.

use crate::escape::html_escape;

/// The status kinds every surface uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatusKind {
    /// Neutral information, no severity.
    Neutral,
    /// Read-only mode: no platform settings are changed.
    ReadOnly,
    /// Verified completion.
    Verified,
    /// Attention needed (warning severity).
    Attention,
    /// Failure and harm (critical severity).
    Failure,
    /// Not available: absent data with a reason, never a zero.
    Unavailable,
    /// A durable job is running.
    InProgress,
    /// A durable job completed.
    Completed,
    /// A durable job is queued behind other work.
    Queued,
}

impl StatusKind {
    /// The CSS classes a chip of this kind carries.
    pub fn class(&self) -> &'static str {
        match self {
            StatusKind::Neutral => "status status-neutral",
            StatusKind::ReadOnly => "status status-read-only",
            StatusKind::Verified => "status status-verified",
            StatusKind::Attention => "status status-attention",
            StatusKind::Failure => "status status-failure",
            StatusKind::Unavailable => "status status-unavailable",
            StatusKind::InProgress => "status status-in-progress",
            StatusKind::Completed => "status status-completed",
            StatusKind::Queued => "status status-queued",
        }
    }

    /// The canonical label for this kind. Used by the status-pair component,
    /// which shows a label with the kind's standard text so status never
    /// depends on color alone.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            StatusKind::Neutral => "Info",
            StatusKind::ReadOnly => "Read-only",
            StatusKind::Verified => "Verified",
            StatusKind::Attention => "Needs attention",
            StatusKind::Failure => "Failed",
            StatusKind::Unavailable => "Not available",
            StatusKind::InProgress => "Running",
            StatusKind::Completed => "Completed",
            StatusKind::Queued => "Queued",
        }
    }
}

/// Render `label` as a status chip of `kind`. The label is escaped.
pub fn chip(label: &str, kind: StatusKind) -> String {
    format!(
        "<span class=\"{}\">{}</span>",
        kind.class(),
        html_escape(label)
    )
}

/// Map an audit severity string to a [`StatusKind`], case-insensitively:
/// `critical` to [`StatusKind::Failure`], `warning` to
/// [`StatusKind::Attention`], `info` to [`StatusKind::Neutral`]. Anything
/// else is [`StatusKind::Neutral`].
pub fn severity(kind: &str) -> StatusKind {
    match kind.trim().to_ascii_lowercase().as_str() {
        "critical" => StatusKind::Failure,
        "warning" => StatusKind::Attention,
        "info" => StatusKind::Neutral,
        _ => StatusKind::Neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_distinct_class_and_label() {
        let kinds = [
            StatusKind::Neutral,
            StatusKind::ReadOnly,
            StatusKind::Verified,
            StatusKind::Attention,
            StatusKind::Failure,
            StatusKind::Unavailable,
            StatusKind::InProgress,
            StatusKind::Completed,
            StatusKind::Queued,
        ];
        let mut classes = Vec::new();
        let mut labels = Vec::new();
        for kind in kinds {
            let class = kind.class();
            assert!(class.starts_with("status status-"), "{class}");
            assert!(!labels.contains(&kind.label()), "duplicate label");
            classes.push(class);
            labels.push(kind.label());
        }
        assert_eq!(classes.len(), 9);
    }

    #[test]
    fn chip_wraps_and_escapes_the_label() {
        assert_eq!(
            chip("Verified", StatusKind::Verified),
            "<span class=\"status status-verified\">Verified</span>"
        );
        let hostile = chip("<script>\"x", StatusKind::Failure);
        assert!(!hostile.contains("<script>"));
        assert!(hostile.contains("&lt;script&gt;"));
        assert_eq!(
            chip("Read-only", StatusKind::ReadOnly),
            "<span class=\"status status-read-only\">Read-only</span>"
        );
    }

    #[test]
    fn severity_maps_case_insensitively() {
        assert_eq!(severity("critical"), StatusKind::Failure);
        assert_eq!(severity("Critical"), StatusKind::Failure);
        assert_eq!(severity("CRITICAL"), StatusKind::Failure);
        assert_eq!(severity("warning"), StatusKind::Attention);
        assert_eq!(severity(" Warning "), StatusKind::Attention);
        assert_eq!(severity("info"), StatusKind::Neutral);
        assert_eq!(severity("INFO"), StatusKind::Neutral);
        assert_eq!(severity("unexpected"), StatusKind::Neutral);
        assert_eq!(severity(""), StatusKind::Neutral);
    }
}
