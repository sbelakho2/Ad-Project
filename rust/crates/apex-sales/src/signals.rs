//! Public signals about a publisher: its `app-ads.txt` and its store
//! listings.
//!
//! # `app-ads.txt`
//!
//! Parsed per the IAB Tech Lab ads.txt specification (v1.1, which app-ads.txt
//! reuses): each data record is `<ad system domain>, <seller account id>,
//! <relationship>[, <certification authority id>]`, the relationship is
//! `DIRECT` or `RESELLER`, `#` starts a comment,
//! and `NAME=value` lines are variables (`CONTACT`, `SUBDOMAIN`,
//! `INVENTORYPARTNERDOMAIN`, `OWNERDOMAIN`, `MANAGERDOMAIN`). Every line the
//! file gets wrong is kept as an [`AdsTxtError`] with its line number, so an
//! email can cite the exact lines.
//!
//! # Which mediation platform?
//!
//! `app-ads.txt` lists the ad systems allowed to sell the inventory, not the
//! mediation platform. [`MediationModel`] turns the presence of the three
//! platforms' own `DIRECT` lines into a posterior over the platform with a
//! naive-Bayes model whose likelihoods are Beta posteriors: they start from
//! documented priors and are re-estimated from publishers who declared their
//! platform (inbound audit requests), so the inference improves with use.

use crate::json::Value;
use crate::model::{App, Store};

use std::collections::HashSet;

/// Seller relationship in an `app-ads.txt` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Relationship {
    /// The publisher's own account with the ad system.
    Direct,
    /// An intermediary sells on the publisher's behalf.
    Reseller,
}

/// One valid data record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdsRecord {
    /// 1-based line number.
    pub line: usize,
    /// Ad system domain, lowercase.
    pub domain: String,
    /// Seller account id as written.
    pub account_id: String,
    /// Relationship.
    pub relationship: Relationship,
    /// Certification authority (TAG) id, when present.
    pub cert_id: Option<String>,
}

/// What is wrong with a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdsTxtErrorKind {
    /// Fewer than three comma-separated fields.
    TooFewFields,
    /// The ad system domain is not a domain.
    BadDomain,
    /// The account id is empty.
    EmptyAccountId,
    /// The relationship is neither `DIRECT` nor `RESELLER`.
    BadRelationship,
    /// The same domain, account and relationship appear twice.
    Duplicate,
    /// Google's well-known certification id is missing or wrong.
    WrongGoogleCertId,
}

impl AdsTxtErrorKind {
    /// Human wording used in findings.
    pub fn describe(self) -> &'static str {
        match self {
            AdsTxtErrorKind::TooFewFields => "has fewer than the three required fields",
            AdsTxtErrorKind::BadDomain => "has an ad system domain that is not a domain",
            AdsTxtErrorKind::EmptyAccountId => "has an empty seller account id",
            AdsTxtErrorKind::BadRelationship => "has a relationship other than DIRECT or RESELLER",
            AdsTxtErrorKind::Duplicate => "repeats an earlier line",
            AdsTxtErrorKind::WrongGoogleCertId => "lists google.com without its certification id",
        }
    }
}

/// One bad line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdsTxtError {
    /// 1-based line number.
    pub line: usize,
    /// Kind.
    pub kind: AdsTxtErrorKind,
}

/// A parsed `app-ads.txt`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdsTxtReport {
    /// Valid data records.
    pub records: Vec<AdsRecord>,
    /// Variables as `(name, value, line)`, names uppercased.
    pub variables: Vec<(String, String, usize)>,
    /// Bad lines.
    pub errors: Vec<AdsTxtError>,
}

/// Google's certification authority id for `google.com` records.
pub const GOOGLE_CERT_ID: &str = "f08c47fec0942fa0";

/// Parse an `app-ads.txt` body.
pub fn parse_app_ads_txt(text: &str) -> AdsTxtReport {
    let mut report = AdsTxtReport::default();
    let mut seen: HashSet<(String, String, Relationship)> = HashSet::new();
    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        let line = raw.split('#').next().unwrap_or_default().trim();
        let line = line.trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if !line.contains(',') {
            if let Some((name, value)) = line.split_once('=') {
                let name = name.trim().to_ascii_uppercase();
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphabetic()) {
                    report
                        .variables
                        .push((name, value.trim().to_string(), line_no));
                    continue;
                }
            }
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        let error = |kind| AdsTxtError {
            line: line_no,
            kind,
        };
        if fields.len() < 3 {
            report.errors.push(error(AdsTxtErrorKind::TooFewFields));
            continue;
        }
        let domain = fields[0].to_ascii_lowercase();
        if !crate::model::is_domain(&domain) {
            report.errors.push(error(AdsTxtErrorKind::BadDomain));
            continue;
        }
        let account_id = fields[1].to_string();
        if account_id.is_empty() {
            report.errors.push(error(AdsTxtErrorKind::EmptyAccountId));
            continue;
        }
        let relationship = match fields[2].to_ascii_uppercase().as_str() {
            "DIRECT" => Relationship::Direct,
            "RESELLER" => Relationship::Reseller,
            _ => {
                report.errors.push(error(AdsTxtErrorKind::BadRelationship));
                continue;
            }
        };
        let cert_id = fields
            .get(3)
            .map(|c| c.to_ascii_lowercase())
            .filter(|c| !c.is_empty());
        let key = (
            domain.clone(),
            account_id.to_ascii_lowercase(),
            relationship,
        );
        if !seen.insert(key) {
            report.errors.push(error(AdsTxtErrorKind::Duplicate));
            continue;
        }
        if domain == "google.com" && cert_id.as_deref() != Some(GOOGLE_CERT_ID) {
            report
                .errors
                .push(error(AdsTxtErrorKind::WrongGoogleCertId));
        }
        report.records.push(AdsRecord {
            line: line_no,
            domain,
            account_id,
            relationship,
            cert_id,
        });
    }
    report
}

/// Ad network behind an ad system domain, for the networks that matter in
/// mobile mediation.
pub fn network_of(domain: &str) -> Option<&'static str> {
    const TABLE: [(&str, &str); 24] = [
        ("applovin.com", "AppLovin"),
        ("google.com", "Google"),
        ("unity3d.com", "Unity"),
        ("unity.com", "Unity"),
        ("ironsrc.com", "ironSource"),
        ("is.com", "ironSource"),
        ("facebook.com", "Meta"),
        ("meta.com", "Meta"),
        ("mintegral.com", "Mintegral"),
        ("vungle.com", "Liftoff"),
        ("liftoff.io", "Liftoff"),
        ("inmobi.com", "InMobi"),
        ("pangleglobal.com", "Pangle"),
        ("bytedance.com", "Pangle"),
        ("chartboost.com", "Chartboost"),
        ("fyber.com", "DT Exchange"),
        ("inner-active.com", "DT Exchange"),
        ("moloco.com", "Moloco"),
        ("bidmachine.io", "BidMachine"),
        ("yandex.com", "Yandex"),
        ("bigossp.com", "Bigo"),
        ("pubnative.net", "Verve"),
        ("smaato.com", "Smaato"),
        ("amazon-adsystem.com", "Amazon"),
    ];
    TABLE.iter().find(|(d, _)| *d == domain).map(|(_, n)| *n)
}

impl AdsTxtReport {
    /// Distinct known networks, sorted.
    pub fn networks(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = self
            .records
            .iter()
            .filter_map(|r| network_of(&r.domain))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Whether any `DIRECT` record belongs to one of `domains`.
    pub fn has_direct(&self, domains: &[&str]) -> bool {
        self.records
            .iter()
            .any(|r| r.relationship == Relationship::Direct && domains.contains(&r.domain.as_str()))
    }

    /// `CONTACT=` values that are email addresses, normalized.
    pub fn contact_emails(&self) -> Vec<String> {
        self.variables
            .iter()
            .filter(|(name, _, _)| name == "CONTACT")
            .filter_map(|(_, value, _)| {
                let v = value.trim_start_matches("mailto:");
                crate::model::normalize_email(v)
            })
            .collect()
    }

    /// Known networks that appear only through resellers, never `DIRECT`.
    pub fn reseller_only_networks(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = self
            .networks()
            .into_iter()
            .filter(|net| {
                !self.records.iter().any(|r| {
                    r.relationship == Relationship::Direct && network_of(&r.domain) == Some(*net)
                })
            })
            .collect();
        out.sort_unstable();
        out
    }
}

/// A quotable finding derived from public data, with the lines it rests on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Stable code (`ads_txt_errors`, `ads_txt_missing`, ...).
    pub code: &'static str,
    /// The statement, phrased to be quoted to the publisher.
    pub statement: String,
}

/// Findings from an `app-ads.txt` check of `domain`. `None` report means
/// the file was looked for and not found.
pub fn ads_txt_findings(domain: &str, report: Option<&AdsTxtReport>) -> Vec<Finding> {
    let url = format!("https://{domain}/app-ads.txt");
    let mut out = Vec::new();
    let Some(report) = report else {
        out.push(Finding {
            code: "ads_txt_missing",
            statement: format!(
                "{url} was not found, so ad buyers that check it cannot verify your inventory"
            ),
        });
        return out;
    };
    if !report.errors.is_empty() {
        let mut lines: Vec<String> = report.errors.iter().map(|e| e.line.to_string()).collect();
        lines.truncate(6);
        let more = if report.errors.len() > 6 { ", ..." } else { "" };
        let first = report
            .errors
            .first()
            .map(|e| e.kind.describe())
            .unwrap_or_default();
        out.push(Finding {
            code: "ads_txt_errors",
            statement: format!(
                "{} line{} of {url} fail{} the IAB format (line{} {}{more}); for example, line {} {first}",
                report.errors.len(),
                if report.errors.len() == 1 { "" } else { "s" },
                if report.errors.len() == 1 { "s" } else { "" },
                if lines.len() == 1 { "" } else { "s" },
                lines.join(", "),
                report.errors.first().map(|e| e.line).unwrap_or_default(),
            ),
        });
    }
    let reseller_only = report.reseller_only_networks();
    if !reseller_only.is_empty() {
        out.push(Finding {
            code: "reseller_only",
            statement: format!(
                "{url} authorizes {} only through resellers, with no DIRECT line",
                reseller_only.join(", ")
            ),
        });
    }
    out
}

/// Mediation platforms the engine distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// AppLovin MAX.
    Max,
    /// Unity LevelPlay.
    LevelPlay,
    /// Google AdMob mediation.
    AdMob,
    /// Anything else, or none.
    Other,
}

impl Platform {
    /// All platforms, in a fixed order.
    pub const ALL: [Platform; 4] = [
        Platform::Max,
        Platform::LevelPlay,
        Platform::AdMob,
        Platform::Other,
    ];

    /// Index in [`Platform::ALL`].
    pub fn index(self) -> usize {
        match self {
            Platform::Max => 0,
            Platform::LevelPlay => 1,
            Platform::AdMob => 2,
            Platform::Other => 3,
        }
    }

    /// Parse a declared platform name (form values like `max`, `levelplay`).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '-', '_'], "");
        match s.as_str() {
            "max" | "applovinmax" | "applovin" => Some(Platform::Max),
            "levelplay" | "unitylevelplay" | "ironsource" => Some(Platform::LevelPlay),
            "admob" | "googleadmob" => Some(Platform::AdMob),
            "other" | "none" => Some(Platform::Other),
            _ => None,
        }
    }
}

/// The three binary features the mediation model reads.
pub fn mediation_features(report: &AdsTxtReport) -> [bool; 3] {
    [
        report.has_direct(&["applovin.com"]),
        report.has_direct(&["ironsrc.com", "is.com", "unity3d.com", "unity.com"]),
        report.has_direct(&["google.com"]),
    ]
}

/// Naive-Bayes mediation inference with Beta-posterior likelihoods.
#[derive(Debug, Clone, PartialEq)]
pub struct MediationModel {
    /// Prior platform shares (sum to 1).
    pub prior: [f64; 4],
    /// `(alpha, beta)` of P(feature present | platform), `[feature][platform]`.
    pub likelihood: [[(f64, f64); 4]; 3],
}

impl Default for MediationModel {
    /// Documented priors, each worth 20 pseudo-observations: shares of
    /// mediated mobile publishers (MAX ahead, then AdMob and LevelPlay), and
    /// how often each platform's publishers carry each `DIRECT` line.
    fn default() -> Self {
        const STRENGTH: f64 = 20.0;
        let mean: [[f64; 4]; 3] = [
            // `DIRECT` applovin.com line, by platform: MAX, LevelPlay, AdMob, Other
            [0.95, 0.45, 0.30, 0.35],
            // `DIRECT` ironSource or Unity line
            [0.55, 0.97, 0.35, 0.40],
            // `DIRECT` google.com line
            [0.85, 0.80, 0.99, 0.70],
        ];
        let mut likelihood = [[(1.0, 1.0); 4]; 3];
        for (f, row) in mean.iter().enumerate() {
            for (p, m) in row.iter().enumerate() {
                likelihood[f][p] = (m * STRENGTH, (1.0 - m) * STRENGTH);
            }
        }
        Self {
            prior: [0.50, 0.20, 0.25, 0.05],
            likelihood,
        }
    }
}

impl MediationModel {
    /// Learn from a publisher whose platform is known.
    pub fn observe(&mut self, features: [bool; 3], platform: Platform) {
        let p = platform.index();
        for (f, present) in features.iter().enumerate() {
            let (a, b) = self.likelihood[f][p];
            self.likelihood[f][p] = if *present { (a + 1.0, b) } else { (a, b + 1.0) };
        }
    }

    /// Posterior platform probabilities, in [`Platform::ALL`] order.
    pub fn posterior(&self, features: [bool; 3]) -> [f64; 4] {
        let mut logp = [0.0f64; 4];
        for (p, lp) in logp.iter_mut().enumerate() {
            *lp = self.prior[p].max(1e-12).ln();
            for (f, present) in features.iter().enumerate() {
                let (a, b) = self.likelihood[f][p];
                let mean = a / (a + b);
                *lp += if *present {
                    mean.ln()
                } else {
                    (1.0 - mean).ln()
                };
            }
        }
        let max = logp.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mut probs = [0.0f64; 4];
        let mut total = 0.0;
        for (p, lp) in logp.iter().enumerate() {
            probs[p] = (lp - max).exp();
            total += probs[p];
        }
        for p in probs.iter_mut() {
            *p /= total;
        }
        probs
    }
}

/// A publisher-level view of one App Store lookup result.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreFacts {
    /// The app.
    pub app: App,
    /// Developer name as listed.
    pub developer: String,
    /// Developer website, when listed.
    pub seller_url: Option<String>,
}

/// Parse the App Store lookup/search API response
/// (`https://itunes.apple.com/lookup?id=...`). `now` is used to compute
/// days since the latest release.
pub fn parse_itunes_results(body: &str, now: crate::model::UnixSecs) -> Vec<StoreFacts> {
    let Ok(doc) = crate::json::parse(body) else {
        return Vec::new();
    };
    let results = doc
        .get("results")
        .and_then(Value::as_array)
        .unwrap_or_default();
    results
        .iter()
        .filter(|r| matches!(r.str_at("kind"), None | Some("software")))
        .filter_map(|r| {
            let id = r.get("trackId").and_then(Value::as_i64)?;
            let name = r.str_at("trackName")?.to_string();
            let developer = r.str_at("artistName").unwrap_or_default().to_string();
            let released = r
                .str_at("currentVersionReleaseDate")
                .and_then(parse_iso8601_utc);
            Some(StoreFacts {
                app: App {
                    store: Store::AppStore,
                    id: id.to_string(),
                    name,
                    genre: r.str_at("primaryGenreName").map(str::to_string),
                    rating_count: r
                        .get("userRatingCount")
                        .and_then(Value::as_i64)
                        .and_then(|n| u64::try_from(n).ok()),
                    free: r.f64_at("price").map(|p| p == 0.0),
                    days_since_update: released.map(|t| {
                        u32::try_from(((now - t) / crate::model::DAY).max(0)).unwrap_or(u32::MAX)
                    }),
                },
                developer,
                seller_url: r.str_at("sellerUrl").map(str::to_string),
            })
        })
        .collect()
}

/// Parse `YYYY-MM-DDTHH:MM:SSZ` (the App Store's format) to Unix seconds.
pub fn parse_iso8601_utc(s: &str) -> Option<crate::model::UnixSecs> {
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse::<i64>().ok() };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    Some(crate::calendar::days_from_civil(y, mo, d) * crate::model::DAY + h * 3600 + mi * 60 + se)
}

/// Size band of a publisher from its largest ratings count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SizeBand {
    /// No ratings data.
    Unknown,
    /// Under 2,000 ratings on the largest app.
    Small,
    /// 2,000 to 50,000.
    Core,
    /// Over 50,000.
    Large,
}

impl SizeBand {
    /// Band from a ratings count.
    pub fn of(max_rating_count: Option<u64>) -> Self {
        match max_rating_count {
            None => SizeBand::Unknown,
            Some(n) if n < 2_000 => SizeBand::Small,
            Some(n) if n <= 50_000 => SizeBand::Core,
            Some(_) => SizeBand::Large,
        }
    }

    /// All bands.
    pub const ALL: [SizeBand; 4] = [
        SizeBand::Unknown,
        SizeBand::Small,
        SizeBand::Core,
        SizeBand::Large,
    ];

    /// Stable string form.
    pub fn as_str(self) -> &'static str {
        match self {
            SizeBand::Unknown => "unknown",
            SizeBand::Small => "small",
            SizeBand::Core => "core",
            SizeBand::Large => "large",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{feff}# app-ads.txt for studio.example\n\
        applovin.com, abc123, DIRECT\n\
        google.com, pub-1, DIRECT, f08c47fec0942fa0\n\
        unity3d.com, 99, RESELLER\n\
        google.com, pub-2, RESELLER\n\
        mintegral.com, 7, DIRECT # comment\n\
        applovin.com, abc123, DIRECT\n\
        not a domain, 1, DIRECT\n\
        moloco.com,, DIRECT\n\
        inmobi.com, 5, FRIEND\n\
        broken line\n\
        CONTACT=mailto:Ads@Studio.Example\n\
        contact=https://studio.example/contact\n\
        SUBDOMAIN=games.studio.example\n";

    #[test]
    fn parses_records_variables_and_errors_with_line_numbers() {
        let r = parse_app_ads_txt(SAMPLE);
        assert_eq!(r.records.len(), 5);
        assert_eq!(r.records[0].line, 2);
        assert_eq!(r.records[1].cert_id.as_deref(), Some(GOOGLE_CERT_ID));
        let kinds: Vec<(usize, AdsTxtErrorKind)> =
            r.errors.iter().map(|e| (e.line, e.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                (5, AdsTxtErrorKind::WrongGoogleCertId),
                (7, AdsTxtErrorKind::Duplicate),
                (8, AdsTxtErrorKind::BadDomain),
                (9, AdsTxtErrorKind::EmptyAccountId),
                (10, AdsTxtErrorKind::BadRelationship),
                (11, AdsTxtErrorKind::TooFewFields),
            ]
        );
        assert_eq!(r.variables.len(), 3);
        assert_eq!(r.contact_emails(), vec!["ads@studio.example".to_string()]);
        assert_eq!(
            r.networks(),
            vec!["AppLovin", "Google", "Mintegral", "Unity"]
        );
        assert_eq!(r.reseller_only_networks(), vec!["Unity"]);
        assert!(r.has_direct(&["applovin.com"]) && !r.has_direct(&["unity3d.com"]));
    }

    #[test]
    fn randomized_lines_never_panic_and_stay_bounded() {
        let mut state = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let tokens = [
            "admob.com",
            "google.com",
            "pub-123",
            "DIRECT",
            "RESELLER",
            ",",
            ";",
            "#",
            " ",
            "\t",
            "é",
            "\u{0}",
            "//",
            "OWNERDOMAIN=example.com",
            "",
        ];
        for _ in 0..200 {
            let lines = (next() % 40) as usize;
            let mut text = String::new();
            for _ in 0..lines {
                let parts = (next() % 5) as usize;
                for index in 0..parts {
                    if index > 0 {
                        text.push_str(", ");
                    }
                    text.push_str(tokens[(next() as usize) % tokens.len()]);
                }
                text.push('\n');
            }
            let report = parse_app_ads_txt(&text);
            assert!(
                report.records.len() <= lines && report.errors.len() <= lines,
                "a report cannot exceed its input"
            );
        }
    }

    #[test]
    fn dedupe_matches_the_naive_first_seen_behavior_on_interleaved_duplicates() {
        // Golden reference: the earlier parser's vector scan, kept verbatim so
        // the set path is judged against the exact first-seen semantics.
        fn naive_parse(text: &str) -> AdsTxtReport {
            let mut report = AdsTxtReport::default();
            let mut seen: Vec<(String, String, Relationship)> = Vec::new();
            for (i, raw) in text.lines().enumerate() {
                let line_no = i + 1;
                let line = raw.split('#').next().unwrap_or_default().trim();
                let line = line.trim_start_matches('\u{feff}');
                if line.is_empty() {
                    continue;
                }
                if !line.contains(',') {
                    if let Some((name, value)) = line.split_once('=') {
                        let name = name.trim().to_ascii_uppercase();
                        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphabetic()) {
                            report
                                .variables
                                .push((name, value.trim().to_string(), line_no));
                            continue;
                        }
                    }
                }
                let fields: Vec<&str> = line.split(',').map(str::trim).collect();
                let error = |kind| AdsTxtError {
                    line: line_no,
                    kind,
                };
                if fields.len() < 3 {
                    report.errors.push(error(AdsTxtErrorKind::TooFewFields));
                    continue;
                }
                let domain = fields[0].to_ascii_lowercase();
                if !crate::model::is_domain(&domain) {
                    report.errors.push(error(AdsTxtErrorKind::BadDomain));
                    continue;
                }
                let account_id = fields[1].to_string();
                if account_id.is_empty() {
                    report.errors.push(error(AdsTxtErrorKind::EmptyAccountId));
                    continue;
                }
                let relationship = match fields[2].to_ascii_uppercase().as_str() {
                    "DIRECT" => Relationship::Direct,
                    "RESELLER" => Relationship::Reseller,
                    _ => {
                        report.errors.push(error(AdsTxtErrorKind::BadRelationship));
                        continue;
                    }
                };
                let cert_id = fields
                    .get(3)
                    .map(|c| c.to_ascii_lowercase())
                    .filter(|c| !c.is_empty());
                let key = (
                    domain.clone(),
                    account_id.to_ascii_lowercase(),
                    relationship,
                );
                if seen.contains(&key) {
                    report.errors.push(error(AdsTxtErrorKind::Duplicate));
                    continue;
                }
                seen.push(key);
                if domain == "google.com" && cert_id.as_deref() != Some(GOOGLE_CERT_ID) {
                    report
                        .errors
                        .push(error(AdsTxtErrorKind::WrongGoogleCertId));
                }
                report.records.push(AdsRecord {
                    line: line_no,
                    domain,
                    account_id,
                    relationship,
                    cert_id,
                });
            }
            report
        }

        let mut text = String::new();
        for i in 0..600 {
            let domain = match i % 3 {
                0 => "applovin.com",
                1 => "Unity3D.com",
                _ => "google.com",
            };
            let account = format!("acct-{}", i % 17);
            let relationship = if i % 2 == 0 { "DIRECT" } else { "RESELLER" };
            let cert = if domain.eq_ignore_ascii_case("google.com") {
                if i % 5 == 0 {
                    ""
                } else {
                    ", f08c47fec0942fa0"
                }
            } else {
                ""
            };
            text.push_str(&format!("{domain}, {account}, {relationship}{cert}\n"));
            // Interleave an immediate duplicate with different case, then a
            // gap, then a later duplicate; the naive scan reports every repeat.
            if i % 4 == 0 {
                text.push_str(&format!(
                    "{}, {}, {}\n",
                    domain.to_ascii_uppercase(),
                    account.to_ascii_uppercase(),
                    relationship.to_ascii_lowercase()
                ));
            }
            if i % 9 == 0 {
                text.push_str("not a domain, 1, DIRECT\n");
            }
        }
        let fast = parse_app_ads_txt(&text);
        let naive = naive_parse(&text);
        assert_eq!(fast, naive, "records, variables and errors must match");
        let duplicate_lines: Vec<usize> = fast
            .errors
            .iter()
            .filter(|error| error.kind == AdsTxtErrorKind::Duplicate)
            .map(|error| error.line)
            .collect();
        assert!(!duplicate_lines.is_empty());
        assert!(
            duplicate_lines.windows(2).all(|pair| pair[0] < pair[1]),
            "duplicates keep their line order"
        );
    }

    #[test]
    fn large_app_ads_txt_dedupe_is_a_linear_time_sanity_check() {
        const N: usize = 25_000;
        let mut text = String::with_capacity(N * 48);
        for i in 0..N {
            text.push_str(&format!("applovin.com, acct-{i}, DIRECT\n"));
        }
        // Every record repeats: the vector scan would probe the whole prefix
        // for each duplicate, the set resolves it in one hash lookup.
        for i in 0..N {
            text.push_str(&format!("applovin.com, acct-{i}, DIRECT\n"));
        }
        let started = std::time::Instant::now();
        let report = parse_app_ads_txt(&text);
        let elapsed = started.elapsed();
        assert_eq!(report.records.len(), N);
        assert_eq!(
            report
                .errors
                .iter()
                .filter(|error| error.kind == AdsTxtErrorKind::Duplicate)
                .count(),
            N
        );
        println!(
            "ads_txt_dedupe lines={} records={} duplicates={} elapsed={elapsed:?}",
            text.lines().count(),
            report.records.len(),
            N
        );
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "50k lines should stay well inside the linear-time sanity bound: {elapsed:?}"
        );
    }

    #[test]
    fn findings_cite_lines_and_missing_files() {
        let r = parse_app_ads_txt(SAMPLE);
        let f = ads_txt_findings("studio.example", Some(&r));
        assert_eq!(f[0].code, "ads_txt_errors");
        assert!(f[0]
            .statement
            .starts_with("6 lines of https://studio.example/app-ads.txt fail"));
        assert!(f[0].statement.contains("lines 5, 7, 8, 9, 10, 11"));
        assert_eq!(f[1].code, "reseller_only");
        let missing = ads_txt_findings("studio.example", None);
        assert_eq!(missing[0].code, "ads_txt_missing");
        let clean = parse_app_ads_txt("applovin.com, 1, DIRECT\n");
        assert!(ads_txt_findings("x.io", Some(&clean)).is_empty());
        let one = parse_app_ads_txt("x\n");
        assert!(ads_txt_findings("x.io", Some(&one))[0]
            .statement
            .starts_with("1 line of"));
        let many = parse_app_ads_txt(&"bad\n".repeat(8));
        assert!(ads_txt_findings("x.io", Some(&many))[0]
            .statement
            .contains(", ..."));
    }

    #[test]
    fn mediation_posterior_moves_with_evidence_and_learns() {
        let model = MediationModel::default();
        let max_like = model.posterior([true, false, true]);
        assert!(max_like[0] > 0.7, "{max_like:?}");
        let lp_like = model.posterior([false, true, true]);
        assert!(lp_like[1] > 0.5, "{lp_like:?}");
        let admob_like = model.posterior([false, false, true]);
        assert!(admob_like[2] > admob_like[0], "{admob_like:?}");
        let sum: f64 = max_like.iter().sum();
        assert!((sum - 1.0).abs() < 1e-12);

        // Learning: many LevelPlay publishers that also carry a `DIRECT`
        // applovin line shift that pattern toward LevelPlay.
        let mut learned = model.clone();
        for _ in 0..200 {
            learned.observe([true, true, true], Platform::LevelPlay);
        }
        let before = model.posterior([true, true, true])[1];
        let after = learned.posterior([true, true, true])[1];
        assert!(after > before + 0.15, "{before} {after}");
        assert_eq!(
            Platform::parse("Unity LevelPlay"),
            Some(Platform::LevelPlay)
        );
        assert_eq!(Platform::parse("AppLovin MAX"), Some(Platform::Max));
        assert_eq!(Platform::parse("admob"), Some(Platform::AdMob));
        assert_eq!(Platform::parse("none"), Some(Platform::Other));
        assert_eq!(Platform::parse("???"), None);
        let r = parse_app_ads_txt("applovin.com, 1, DIRECT\nis.com, 2, DIRECT\n");
        assert_eq!(mediation_features(&r), [true, true, false]);
    }

    #[test]
    fn parses_app_store_results() {
        let body = r#"{"resultCount":2,"results":[
            {"kind":"software","trackId":42,"trackName":"Tile Quest","artistName":"Studio","primaryGenreName":"Games","userRatingCount":12500,"price":0,"sellerUrl":"https://studio.example","currentVersionReleaseDate":"2026-09-16T07:00:00Z"},
            {"kind":"podcast","trackId":7,"trackName":"Not an app"}]}"#;
        let now = parse_iso8601_utc("2026-10-16T07:00:00Z").unwrap_or(0);
        let facts = parse_itunes_results(body, now);
        assert_eq!(facts.len(), 1);
        let f = &facts[0];
        assert_eq!(f.app.id, "42");
        assert_eq!(f.app.rating_count, Some(12_500));
        assert_eq!(f.app.free, Some(true));
        assert_eq!(f.app.days_since_update, Some(30));
        assert_eq!(f.seller_url.as_deref(), Some("https://studio.example"));
        assert!(parse_itunes_results("not json", now).is_empty());
        assert!(parse_itunes_results("{}", now).is_empty());
    }

    #[test]
    fn parses_iso_dates_and_rejects_bad_ones() {
        assert_eq!(parse_iso8601_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601_utc("2000-03-01T00:00:00Z"), Some(951_868_800));
        for bad in [
            "2026-13-01T00:00:00Z",
            "2026-01-01 00:00:00Z",
            "short",
            "2026-01-01T25:00:00Z",
        ] {
            assert_eq!(parse_iso8601_utc(bad), None, "{bad}");
        }
    }

    #[test]
    fn size_bands_and_networks() {
        assert_eq!(SizeBand::of(None), SizeBand::Unknown);
        assert_eq!(SizeBand::of(Some(10)), SizeBand::Small);
        assert_eq!(SizeBand::of(Some(2_000)), SizeBand::Core);
        assert_eq!(SizeBand::of(Some(50_001)), SizeBand::Large);
        assert_eq!(network_of("is.com"), Some("ironSource"));
        assert_eq!(network_of("example.com"), None);
        assert_eq!(SizeBand::Core.as_str(), "core");
        for k in [
            AdsTxtErrorKind::TooFewFields,
            AdsTxtErrorKind::BadDomain,
            AdsTxtErrorKind::EmptyAccountId,
            AdsTxtErrorKind::BadRelationship,
            AdsTxtErrorKind::Duplicate,
            AdsTxtErrorKind::WrongGoogleCertId,
        ] {
            assert!(!k.describe().is_empty());
        }
    }
}

/// Cross-crate parity with the audit rule.
///
/// The app-ads.txt corpus lives once, at
/// `rust/crates/apex-audit/tests/fixtures/adstxt/`, and is read by both
/// crates: this parser's test below and the audit rule's integration test
/// (`apex-audit/tests/app_ads_txt_corpus.rs`) assert against the same
/// `<stem>.expected.json` sidecars, so "the audit and the sales engine report
/// the same errors for the same file" is enforced at build time in both
/// crates. The directory is reached through `CARGO_MANIFEST_DIR/../apex-audit`
/// because this crate is dependency-free by design and cannot carry the
/// fixtures itself; CI runs in-repo, where that traversal always resolves.
#[cfg(test)]
mod app_ads_txt_corpus_parity {
    use std::path::PathBuf;

    use crate::json::Value;
    use crate::signals::{parse_app_ads_txt, AdsTxtErrorKind};

    /// The corpus file stems; the directory must contain exactly the
    /// `<stem>.txt` and `<stem>.expected.json` pairs for these stems.
    const CORPUS: [&str; 10] = [
        "clean-standard",
        "commented",
        "variables-only",
        "crlf-bom",
        "reseller",
        "malformed-two-field",
        "malformed-five-field",
        "mixed-duplicates",
        "google-cert",
        "comments-only",
    ];

    fn corpus_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../apex-audit/tests/fixtures/adstxt")
    }

    fn kind_name(kind: AdsTxtErrorKind) -> &'static str {
        match kind {
            AdsTxtErrorKind::TooFewFields => "TooFewFields",
            AdsTxtErrorKind::BadDomain => "BadDomain",
            AdsTxtErrorKind::EmptyAccountId => "EmptyAccountId",
            AdsTxtErrorKind::BadRelationship => "BadRelationship",
            AdsTxtErrorKind::Duplicate => "Duplicate",
            AdsTxtErrorKind::WrongGoogleCertId => "WrongGoogleCertId",
        }
    }

    #[test]
    fn corpus_directory_matches_the_hardcoded_list() {
        let mut found: Vec<String> = std::fs::read_dir(corpus_dir())
            .expect("the shared corpus directory must exist")
            .map(|entry| {
                entry
                    .expect("readable corpus entry")
                    .file_name()
                    .into_string()
                    .expect("utf-8 names")
            })
            .collect();
        found.sort();
        let mut expected: Vec<String> = Vec::new();
        for stem in CORPUS {
            expected.push(format!("{stem}.txt"));
            expected.push(format!("{stem}.expected.json"));
        }
        expected.sort();
        assert_eq!(
            found, expected,
            "corpus files drifted from the hardcoded list"
        );
        assert!(CORPUS.len() >= 8, "the corpus must hold at least 8 files");
    }

    #[test]
    fn corpus_error_sets_equal_the_expected_json() {
        let dir = corpus_dir();
        for stem in CORPUS {
            let file_name = format!("{stem}.txt");
            let text = std::fs::read_to_string(dir.join(&file_name)).expect("corpus file readable");
            let expected_text = std::fs::read_to_string(dir.join(format!("{stem}.expected.json")))
                .expect("sidecar readable");
            let expected = crate::json::parse(&expected_text).expect("sidecar is valid JSON");
            assert_eq!(
                expected.get("file").and_then(Value::as_str),
                Some(file_name.as_str())
            );

            let report = parse_app_ads_txt(&text);
            let expected_errors = expected
                .get("errors")
                .and_then(Value::as_array)
                .expect("sidecar errors array");
            let actual: Vec<(usize, &'static str)> = report
                .errors
                .iter()
                .map(|error| (error.line, kind_name(error.kind)))
                .collect();
            assert_eq!(
                actual.len(),
                expected_errors.len(),
                "{file_name}: error count"
            );
            for (index, wanted) in expected_errors.iter().enumerate() {
                let line = wanted
                    .get("line")
                    .and_then(Value::as_i64)
                    .expect("line number") as usize;
                let kind = wanted
                    .get("kind")
                    .and_then(Value::as_str)
                    .expect("error kind");
                assert_eq!(actual[index], (line, kind), "{file_name}: error {index}");
            }

            let records = expected
                .get("records")
                .and_then(Value::as_i64)
                .expect("records count");
            assert_eq!(report.records.len() as i64, records, "{file_name}: records");
            let variables = expected
                .get("variables")
                .and_then(Value::as_i64)
                .expect("variables count");
            assert_eq!(
                report.variables.len() as i64,
                variables,
                "{file_name}: variables"
            );

            let mut domains: Vec<&str> = report
                .records
                .iter()
                .map(|record| record.domain.as_str())
                .collect();
            domains.sort_unstable();
            domains.dedup();
            let wanted_domains: Vec<&str> = expected
                .get("domains")
                .and_then(Value::as_array)
                .expect("domains array")
                .iter()
                .map(|domain| domain.as_str().expect("domain string"))
                .collect();
            assert_eq!(
                domains, wanted_domains,
                "{file_name}: accepted-record domains"
            );
        }
    }
}
