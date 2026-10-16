//! Duplicate publishers: the same studio found through different sources.
//!
//! Probabilistic record linkage after Fellegi and Sunter (1969): each
//! compared field contributes `log₂(m/u)` when it agrees and
//! `log₂[(1 − m)/(1 − u)]` when it disagrees, where `m` is the probability
//! of agreement for a true match and `u` for a non-match. The summed weight
//! is compared with two thresholds: above `upper` is a match, below `lower`
//! a non-match, in between goes to a person. Names are compared with
//! Jaro–Winkler similarity (Winkler, 1990).

/// Jaro similarity of two strings (on chars).
pub fn jaro(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let window = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut a_match = vec![false; a.len()];
    let mut b_match = vec![false; b.len()];
    let mut matches = 0usize;
    for (i, ca) in a.iter().enumerate() {
        let lo = i.saturating_sub(window);
        let hi = (i + window + 1).min(b.len());
        for j in lo..hi {
            if !b_match[j] && b[j] == *ca {
                a_match[i] = true;
                b_match[j] = true;
                matches += 1;
                break;
            }
        }
    }
    if matches == 0 {
        return 0.0;
    }
    let a_seq = a.iter().zip(&a_match).filter(|(_, m)| **m).map(|(c, _)| c);
    let b_seq = b.iter().zip(&b_match).filter(|(_, m)| **m).map(|(c, _)| c);
    let transpositions = a_seq.zip(b_seq).filter(|(x, y)| x != y).count() / 2;
    let m = matches as f64;
    (m / a.len() as f64 + m / b.len() as f64 + (m - transpositions as f64) / m) / 3.0
}

/// Jaro–Winkler similarity with prefix scale 0.1 over at most 4 chars.
pub fn jaro_winkler(a: &str, b: &str) -> f64 {
    let j = jaro(a, b);
    let prefix = a
        .chars()
        .zip(b.chars())
        .take(4)
        .take_while(|(x, y)| x == y)
        .count();
    j + prefix as f64 * 0.1 * (1.0 - j)
}

/// Lowercase, keep alphanumerics, drop common company suffixes.
pub fn normalize_name(name: &str) -> String {
    const SUFFIXES: [&str; 12] = [
        "ltd", "llc", "inc", "gmbh", "oy", "ou", "as", "ab", "sas", "studio", "studios", "games",
    ];
    let lower = name.to_lowercase();
    let words: Vec<String> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let kept: Vec<&str> = words
        .iter()
        .map(String::as_str)
        .filter(|w| !SUFFIXES.contains(w))
        .collect();
    if kept.is_empty() {
        words.join(" ")
    } else {
        kept.join(" ")
    }
}

/// Fields compared for two publisher records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRecord {
    /// Normalized web domain.
    pub domain: Option<String>,
    /// Display name.
    pub name: String,
    /// App Store developer name, when known.
    pub developer: Option<String>,
    /// Contact emails' domains.
    pub email_domains: Vec<String>,
}

/// Linkage decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkDecision {
    /// Same publisher.
    Match,
    /// Different publishers.
    NonMatch,
    /// A person decides.
    Review,
}

/// `(m, u)` per comparison and the decision thresholds, in bits.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkModel {
    /// Domain agreement.
    pub domain: (f64, f64),
    /// Name Jaro–Winkler ≥ 0.92.
    pub name: (f64, f64),
    /// Developer name agreement.
    pub developer: (f64, f64),
    /// Shared email domain.
    pub email_domain: (f64, f64),
    /// Match threshold.
    pub upper: f64,
    /// Non-match threshold.
    pub lower: f64,
}

impl Default for LinkModel {
    fn default() -> Self {
        Self {
            domain: (0.95, 0.0005),
            name: (0.85, 0.01),
            developer: (0.9, 0.005),
            email_domain: (0.8, 0.001),
            upper: 8.0,
            lower: 0.0,
        }
    }
}

fn weight(agree: bool, (m, u): (f64, f64)) -> f64 {
    if agree {
        (m / u).log2()
    } else {
        ((1.0 - m) / (1.0 - u)).log2()
    }
}

impl LinkModel {
    /// Match weight in bits; fields missing on either side contribute 0.
    pub fn weight(&self, a: &LinkRecord, b: &LinkRecord) -> f64 {
        let mut w = 0.0;
        if let (Some(x), Some(y)) = (&a.domain, &b.domain) {
            w += weight(x == y, self.domain);
        }
        let (na, nb) = (normalize_name(&a.name), normalize_name(&b.name));
        if !na.is_empty() && !nb.is_empty() {
            w += weight(jaro_winkler(&na, &nb) >= 0.92, self.name);
        }
        if let (Some(x), Some(y)) = (&a.developer, &b.developer) {
            w += weight(normalize_name(x) == normalize_name(y), self.developer);
        }
        if !a.email_domains.is_empty() && !b.email_domains.is_empty() {
            let shared = a.email_domains.iter().any(|d| b.email_domains.contains(d));
            w += weight(shared, self.email_domain);
        }
        w
    }

    /// Decide.
    pub fn decide(&self, a: &LinkRecord, b: &LinkRecord) -> LinkDecision {
        let w = self.weight(a, b);
        if w >= self.upper {
            LinkDecision::Match
        } else if w <= self.lower {
            LinkDecision::NonMatch
        } else {
            LinkDecision::Review
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jaro_winkler_matches_published_values() {
        assert!((jaro("MARTHA", "MARHTA") - 0.944_444).abs() < 1e-5);
        assert!((jaro_winkler("MARTHA", "MARHTA") - 0.961_111).abs() < 1e-5);
        assert!((jaro_winkler("DIXON", "DICKSONX") - 0.813_333).abs() < 1e-5);
        assert_eq!(jaro("", ""), 1.0);
        assert_eq!(jaro("a", ""), 0.0);
        assert_eq!(jaro("abc", "xyz"), 0.0);
    }

    #[test]
    fn names_normalize_suffixes() {
        assert_eq!(normalize_name("Tiny Fox Games Ltd."), "tiny fox");
        assert_eq!(normalize_name("Games Studio"), "games studio");
    }

    fn rec(domain: Option<&str>, name: &str, dev: Option<&str>, mails: &[&str]) -> LinkRecord {
        LinkRecord {
            domain: domain.map(str::to_string),
            name: name.into(),
            developer: dev.map(str::to_string),
            email_domains: mails.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn linkage_decides_three_ways() {
        let m = LinkModel::default();
        let a = rec(
            Some("tinyfox.io"),
            "Tiny Fox Games",
            Some("Tiny Fox Ltd"),
            &["tinyfox.io"],
        );
        let same = rec(Some("tinyfox.io"), "TinyFox", None, &["tinyfox.io"]);
        assert_eq!(m.decide(&a, &same), LinkDecision::Match);
        let other = rec(
            Some("bigbear.com"),
            "Big Bear",
            Some("Big Bear"),
            &["bigbear.com"],
        );
        assert_eq!(m.decide(&a, &other), LinkDecision::NonMatch);
        let unsure = rec(None, "Tiny Fox", None, &[]);
        assert_eq!(m.decide(&a, &unsure), LinkDecision::Review);
    }
}
