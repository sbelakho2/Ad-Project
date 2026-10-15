//! Metric declarations and the bounded label gate.
//!
//! A [`MetricSpec`] is a `static` declaration: name, kind (counter, gauge or
//! histogram with declared buckets), help text and the declared label set
//! with each label's [`Vocab`]. The [`Registry`](crate::Registry) refuses
//! writes that do not match the declaration, and [`LabelValue`] syntactically
//! refuses id-looking values (emails, URLs, query strings, invoice-like
//! references, PSP-like references, `namespace_123` ids) before any
//! vocabulary is consulted. Together they make it impossible to mint a
//! per-customer or per-object label by accident.

/// What a series stores.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MetricKind {
    /// Monotonic total.
    Counter,
    /// A value that goes up and down.
    Gauge,
    /// Observation distribution with declared cumulative buckets.
    Histogram {
        /// Upper bounds, strictly ascending; the `+Inf` bucket is implicit.
        buckets: &'static [f64],
    },
}

/// The declared value set of one label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Vocab {
    /// Exact allowed values.
    Values(&'static [&'static str]),
    /// A normalized route template: `/segment/{parameter}` shapes only.
    RouteTemplate,
}

impl Vocab {
    /// The kind name for error messages.
    pub(crate) fn kind_name(self) -> &'static str {
        match self {
            Vocab::Values(_) => "declared value set",
            Vocab::RouteTemplate => "route template",
        }
    }

    /// Whether `value` is inside this vocabulary.
    pub fn accepts(self, value: &str) -> bool {
        match self {
            Vocab::Values(values) => values.contains(&value),
            Vocab::RouteTemplate => is_route_template(value),
        }
    }
}

/// One declared label: its name and accepted vocabulary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelSpec {
    /// The label name exactly as it appears in the exposition text.
    pub name: &'static str,
    /// The accepted values.
    pub vocab: Vocab,
}

/// A complete metric declaration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricSpec {
    /// The metric name exactly as it appears in the exposition text.
    pub name: &'static str,
    /// The `# HELP` line.
    pub help: &'static str,
    /// Counter, gauge or histogram.
    pub kind: MetricKind,
    /// The declared labels, in exposition order.
    pub labels: &'static [LabelSpec],
}

impl MetricSpec {
    /// The histogram buckets, when this is a histogram.
    pub fn buckets(&self) -> Option<&'static [f64]> {
        match self.kind {
            MetricKind::Histogram { buckets } => Some(buckets),
            _ => None,
        }
    }
}

/// Why a label or label value was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelError {
    /// The value is empty.
    Empty {
        /// The offending label.
        label: String,
    },
    /// The value is longer than the bound allows.
    TooLong {
        /// The offending label.
        label: String,
        /// The byte length of the value.
        length: usize,
    },
    /// The value carries whitespace or a control character.
    NotASingleToken {
        /// The offending label.
        label: String,
    },
    /// The value looks like an email address.
    EmailLike {
        /// The offending label.
        label: String,
    },
    /// The value looks like a URL.
    UrlLike {
        /// The offending label.
        label: String,
    },
    /// The value looks like a query string.
    QueryStringLike {
        /// The offending label.
        label: String,
    },
    /// The value looks like an object identifier (`exp_0242`, `inc_018`).
    IdentifierLike {
        /// The offending label.
        label: String,
    },
    /// The value looks like an invoice or PSP reference (`INV-2026-104`,
    /// `8837621486530647`).
    ReferenceLike {
        /// The offending label.
        label: String,
    },
}

impl std::fmt::Display for LabelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LabelError::Empty { label } => write!(f, "label '{label}' value is empty"),
            LabelError::TooLong { label, length } => {
                write!(f, "label '{label}' value is {length} bytes long")
            }
            LabelError::NotASingleToken { label } => {
                write!(f, "label '{label}' value is not a single token")
            }
            LabelError::EmailLike { label } => {
                write!(f, "label '{label}' value looks like an email address")
            }
            LabelError::UrlLike { label } => write!(f, "label '{label}' value looks like a URL"),
            LabelError::QueryStringLike { label } => {
                write!(f, "label '{label}' value looks like a query string")
            }
            LabelError::IdentifierLike { label } => {
                write!(f, "label '{label}' value looks like an object id")
            }
            LabelError::ReferenceLike { label } => {
                write!(
                    f,
                    "label '{label}' value looks like an invoice or PSP reference"
                )
            }
        }
    }
}

impl std::error::Error for LabelError {}

/// The longest label value the registry accepts.
pub const MAX_LABEL_VALUE_BYTES: usize = 64;

/// A label value that passed the syntactic gate.
///
/// The syntactic gate refuses the shapes that must never become Prometheus
/// dimensions (section 06 page 112): emails, URLs, query strings and
/// identifier/reference-looking values. Semantic membership in the metric's
/// declared vocabulary is checked by [`Registry`](crate::Registry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelValue<'a> {
    name: &'static str,
    value: &'a str,
}

impl<'a> LabelValue<'a> {
    /// Validate `value` for `name` syntactically.
    pub fn new(name: &'static str, value: &'a str) -> Result<Self, LabelError> {
        if value.is_empty() {
            return Err(LabelError::Empty {
                label: name.to_string(),
            });
        }
        if value.len() > MAX_LABEL_VALUE_BYTES {
            return Err(LabelError::TooLong {
                label: name.to_string(),
                length: value.len(),
            });
        }
        if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(LabelError::NotASingleToken {
                label: name.to_string(),
            });
        }
        if value.contains('@') {
            return Err(LabelError::EmailLike {
                label: name.to_string(),
            });
        }
        if value.contains("://") || value.starts_with("http") {
            return Err(LabelError::UrlLike {
                label: name.to_string(),
            });
        }
        if value.contains('?') || value.contains('=') || value.contains('&') {
            return Err(LabelError::QueryStringLike {
                label: name.to_string(),
            });
        }
        if is_identifier_like(value) {
            return Err(LabelError::IdentifierLike {
                label: name.to_string(),
            });
        }
        if is_reference_like(value) {
            return Err(LabelError::ReferenceLike {
                label: name.to_string(),
            });
        }
        Ok(Self { name, value })
    }

    /// The label name.
    pub fn name(self) -> &'static str {
        self.name
    }

    /// The label value.
    pub fn value(self) -> &'a str {
        self.value
    }
}

/// `namespace_123` shapes: the id style the product uses for experiments,
/// changes, incidents and accounts (`exp_0242`, `inc_018`, `max_acct_1`).
fn is_identifier_like(value: &str) -> bool {
    let Some((_, tail)) = value.rsplit_once('_') else {
        return false;
    };
    !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())
}

/// Reference shapes with no separator convention to rely on: long runs of
/// digits (invoice numbers, PSP references) and mixed-case alphanumeric
/// references (`INV-2026-104`, `8837621486530647`).
fn is_reference_like(value: &str) -> bool {
    if value.len() >= 12 && value.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    let has_upper = value.bytes().any(|b| b.is_ascii_uppercase());
    let has_digit = value.bytes().any(|b| b.is_ascii_digit());
    let reference_chars = value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
    value.len() >= 10 && has_upper && has_digit && reference_chars
}

/// A normalized route template like `/v1/reports/{publisher}`: slash-separated
/// static segments (lowercase words, dots, dashes) and `{parameter}` segments.
/// Numeric or id-looking static segments are refused: the whole point of a
/// template label is that object identities never appear.
fn is_route_template(value: &str) -> bool {
    if !value.starts_with('/') || value.len() > 128 {
        return false;
    }
    for segment in value.split('/').skip(1) {
        if segment.is_empty() {
            continue;
        }
        if let Some(parameter) = segment
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
        {
            if parameter.is_empty()
                || !parameter
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'_')
            {
                return false;
            }
            continue;
        }
        let ok = segment
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
        if !ok {
            return false;
        }
        // A fully numeric segment (or an id-shaped one) is an object identity
        // in a route, not a template.
        if segment.bytes().all(|b| b.is_ascii_digit()) || is_identifier_like(segment) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::vocab::Platform;

    #[test]
    fn declared_values_are_accepted() {
        let value = LabelValue::new("platform", "applovin_max").expect("declared");
        assert_eq!(value.name(), "platform");
        assert_eq!(value.value(), "applovin_max");
        assert!(Vocab::Values(Platform::VALUES).accepts("applovin_max"));
        assert!(!Vocab::Values(Platform::VALUES).accepts("northstar"));
    }

    #[test]
    fn emails_urls_and_query_strings_are_refused() {
        for bad in [
            "ops@example.com",
            "https://example.com/reports",
            "http://console.example",
            "/route?a=b",
            "key=value",
            "a&b",
        ] {
            let error = LabelValue::new("route_template", bad).expect_err(bad);
            assert!(
                matches!(
                    error,
                    LabelError::EmailLike { .. }
                        | LabelError::UrlLike { .. }
                        | LabelError::QueryStringLike { .. }
                ),
                "{bad}: {error}"
            );
        }
    }

    #[test]
    fn id_like_values_are_refused_even_when_short() {
        // Publisher ids in the product's own id style.
        assert!(matches!(
            LabelValue::new("label", "max_acct_1"),
            Err(LabelError::IdentifierLike { .. })
        ));
        // Experiment / incident / change ids.
        for bad in ["exp_0242", "inc_018", "ch_0246", "op_31", "pub_12345"] {
            assert!(
                matches!(
                    LabelValue::new("label", bad),
                    Err(LabelError::IdentifierLike { .. })
                ),
                "{bad}"
            );
        }
        // Invoice and reference shapes.
        for bad in ["INV-2026-104", "8837621486530647", "DF-104-2026"] {
            assert!(
                matches!(
                    LabelValue::new("label", bad),
                    Err(LabelError::ReferenceLike { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn empty_long_and_multi_token_values_are_refused() {
        assert!(matches!(
            LabelValue::new("label", ""),
            Err(LabelError::Empty { .. })
        ));
        let long = "a".repeat(65);
        assert!(matches!(
            LabelValue::new("label", &long),
            Err(LabelError::TooLong { .. })
        ));
        assert!(matches!(
            LabelValue::new("label", "two words"),
            Err(LabelError::NotASingleToken { .. })
        ));
        assert!(matches!(
            LabelValue::new("label", "line\nbreak"),
            Err(LabelError::NotASingleToken { .. })
        ));
    }

    #[test]
    fn route_template_vocabulary_shapes() {
        let vocab = Vocab::RouteTemplate;
        assert!(vocab.accepts("/"));
        assert!(vocab.accepts("/v1/reports"));
        assert!(vocab.accepts("/v1/reports/{publisher}/daily"));
        assert!(vocab.accepts("/console/proposals/{id}"));
        assert!(!vocab.accepts("v1/reports"));
        assert!(!vocab.accepts("/v1/reports/12345"));
        assert!(!vocab.accepts("/v1/reports/{ID}"));
        assert!(!vocab.accepts("/v1/reports?page=2"));
        assert!(!vocab.accepts("/v1/exp_0242"));
    }

    #[test]
    fn label_errors_display_without_panicking() {
        let error = LabelValue::new("x", "a@b.c").expect_err("email");
        assert!(error.to_string().contains("email"));
        let error = LabelValue::new("x", "").expect_err("empty");
        assert!(error.to_string().contains("empty"));
    }
}
