//! Shared ratchet-baseline grammar and comparison.
//!
//! Every ratchet baseline in `tools/coverage/` uses one on-disk format: `count
//! path` rows plus a `TOTAL n` line, with `#` comments. One parser and one
//! comparator keep the docs-links, docs-references and prose ratchets from
//! drifting apart; the stale-row policy is the only per-gate choice.

use std::collections::BTreeMap;

/// Parsed `count path` rows and the optional total (0 when absent).
pub fn parse_count_rows(text: &str) -> (BTreeMap<String, usize>, usize) {
    let mut rows = BTreeMap::new();
    let mut total = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim_start().starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("TOTAL") {
            if rest.starts_with(' ') || rest.starts_with('\t') {
                total = rest.trim().parse::<usize>().unwrap_or(0);
                continue;
            }
        }
        let mut parts = trimmed.splitn(2, ' ');
        if let (Some(count), Some(path)) = (parts.next(), parts.next()) {
            if let Ok(n) = count.parse::<usize>() {
                let path = path.trim();
                if !path.is_empty() {
                    rows.insert(path.to_string(), n);
                }
            }
        }
    }
    (rows, total)
}

/// What a baseline row with no current counterpart means for a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stale {
    /// A stale row fails the gate (link and reference ratchets).
    Failure,
    /// A stale row is reported as a note (prose ratchet).
    Note,
}

/// Ratchet comparison shared by the gates: per-file overages against the
/// baseline rows, stale rows under the chosen policy, and a total ceiling.
pub fn ratchet(
    actual: &BTreeMap<String, usize>,
    allowed: &BTreeMap<String, usize>,
    actual_total: usize,
    allowed_total: usize,
    noun: &str,
    stale: Stale,
) -> (Vec<String>, Vec<String>) {
    let mut failures = Vec::new();
    let mut notes = Vec::new();
    for (file, count) in actual {
        let limit = allowed.get(file).copied().unwrap_or(0);
        if *count > limit {
            failures.push(format!("{file}: {count} {noun} (baseline {limit})"));
        }
    }
    for file in allowed.keys() {
        if !actual.contains_key(file) {
            match stale {
                Stale::Failure => failures.push(format!(
                    "{file}: baseline row no longer needed (stale), remove it"
                )),
                Stale::Note => notes.push(format!("{file}: baseline row no longer needed")),
            }
        }
    }
    if actual_total > allowed_total {
        failures.push(format!(
            "total {actual_total} exceeds baseline total {allowed_total}"
        ));
    }
    (failures, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_reads_rows_comments_and_total() {
        let (rows, total) = parse_count_rows("# note\n3 docs/a.md\n1 docs/b c.md\nTOTAL 4\n");
        assert_eq!(rows.get("docs/a.md"), Some(&3));
        assert_eq!(rows.get("docs/b c.md"), Some(&1));
        assert_eq!(total, 4);
        let (_, missing_total) = parse_count_rows("1 a.md\n");
        assert_eq!(missing_total, 0);
    }

    #[test]
    fn ratchet_flags_overage_stale_and_total_under_both_policies() {
        let allowed: BTreeMap<String, usize> =
            [("a.md".to_string(), 1), ("gone.md".to_string(), 2)]
                .into_iter()
                .collect();
        let actual: BTreeMap<String, usize> = [("a.md".to_string(), 3), ("new.md".to_string(), 1)]
            .into_iter()
            .collect();
        let (failures, notes) = ratchet(&actual, &allowed, 4, 3, "broken links", Stale::Failure);
        assert!(failures.iter().any(|f| f.contains("a.md: 3 broken links")));
        assert!(failures
            .iter()
            .any(|f| f.contains("new.md: 1 broken links")));
        assert!(failures
            .iter()
            .any(|f| f.contains("gone.md") && f.contains("stale")));
        assert!(failures.iter().any(|f| f.contains("total 4 exceeds")));
        assert!(notes.is_empty());
        let (failures, notes) = ratchet(&actual, &allowed, 4, 3, "violations", Stale::Note);
        assert!(!failures.iter().any(|f| f.contains("gone.md")));
        assert!(notes.iter().any(|n| n.contains("gone.md")));
    }
}
