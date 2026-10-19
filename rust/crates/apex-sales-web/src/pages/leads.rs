//! `/leads`: every tracked contact with its publisher, platform inference,
//! score and evidence; filtered views are stable GET URLs and the CSV
//! export ships exactly the filtered rows.

use apex_sales::model::Publisher;
use apex_sales::scoring::features;
use apex_sales::signals::{mediation_features, MediationModel, Platform, SizeBand};
use apex_ui::components::{action, data_table, field, select, Cell, EmptyState};
use apex_ui::escape::html_escape;
use apex_ui::format::{csv_field, csv_text_field, date_short, percent};

use crate::data::RequestData;
use crate::pages::shared::{self, PageMeta};

/// The four filters, each with a closed set of values; anything else falls
/// back to `all`, so a hostile query can never select an unknown mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filters {
    /// `all`, `active` or `suppressed`.
    pub status: String,
    /// `all`, `seed`, `free-audit` or `app-ads`.
    pub source: String,
    /// `all`, `max`, `levelplay`, `admob`, `other` or `none`.
    pub platform: String,
    /// `all`, `unknown`, `small`, `core` or `large`.
    pub band: String,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            status: "all".to_string(),
            source: "all".to_string(),
            platform: "all".to_string(),
            band: "all".to_string(),
        }
    }
}

const STATUS_VALUES: [&str; 3] = ["all", "active", "suppressed"];
const SOURCE_VALUES: [&str; 4] = ["all", "seed", "free-audit", "app-ads"];
const PLATFORM_VALUES: [&str; 6] = ["all", "max", "levelplay", "admob", "other", "none"];
const BAND_VALUES: [&str; 5] = ["all", "unknown", "small", "core", "large"];

impl Filters {
    /// Parse from query pairs; unknown values fall back to `all`.
    pub fn from_query(pairs: &[(String, String)]) -> Self {
        let get = |name: &str, allowed: &[&str], default: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.trim())
                .filter(|value| allowed.contains(value))
                .unwrap_or(default)
                .to_string()
        };
        Self {
            status: get("status", &STATUS_VALUES, "all"),
            source: get("source", &SOURCE_VALUES, "all"),
            platform: get("platform", &PLATFORM_VALUES, "all"),
            band: get("band", &BAND_VALUES, "all"),
        }
    }

    /// Whether any filter narrows the view.
    pub fn is_default(&self) -> bool {
        *self == Filters::default()
    }

    /// The query string for a stable URL (`status=seed...` order fixed).
    pub fn query(&self) -> String {
        let pairs = [
            ("status", &self.status),
            ("source", &self.source),
            ("platform", &self.platform),
            ("band", &self.band),
        ];
        pairs
            .iter()
            .filter(|(_, value)| value.as_str() != "all")
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("&")
    }
}

/// One rendered lead row.
#[derive(Debug, Clone, PartialEq)]
pub struct LeadRow {
    /// The contact.
    pub email: String,
    /// The publisher key (stable id).
    pub publisher_key: String,
    /// Publisher web domain, when known.
    pub domain: Option<String>,
    /// Publisher display name.
    pub name: String,
    /// Country, when known.
    pub country: Option<String>,
    /// Platform key for filtering.
    pub platform_key: String,
    /// Platform display label with its basis.
    pub platform_label: String,
    /// Size band.
    pub band: SizeBand,
    /// Posterior-mean interest probability.
    pub score: f64,
    /// Top three reason codes by absolute contribution.
    pub reasons: Vec<String>,
    /// Whether the contact is suppressed.
    pub suppressed: Option<&'static str>,
    /// Publisher evidence count.
    pub evidence_count: usize,
    /// Distinct evidence kinds.
    pub evidence_kinds: String,
    /// Latest evidence day (UTC) when any.
    pub latest_evidence_day: Option<String>,
    /// Contact source class (`seed`, `free-audit`, `app-ads`, `other`).
    pub source_class: &'static str,
    /// Contact name, when known.
    pub contact_name: Option<String>,
    /// Contact role, when known.
    pub contact_role: Option<String>,
    /// Contact kind (`named`, `role`, `generic`).
    pub contact_kind: &'static str,
    /// Contact consent flag.
    pub consent: bool,
    /// Raw contact source string.
    pub source_raw: String,
}

/// The classification of a contact's `source` for the source filter.
pub fn source_class(source: &str) -> &'static str {
    let lower = source.to_ascii_lowercase();
    if lower == "seed list" || lower.starts_with("seed") {
        "seed"
    } else if lower == "inbound" {
        "free-audit"
    } else if lower.contains("app-ads.txt") || lower.starts_with("https://") {
        "app-ads"
    } else {
        "other"
    }
}

/// The platform key and label for a publisher, from declared data or the
/// engine's mediation inference; `none` with the reason when neither exists.
pub fn platform_of(publisher: &Publisher, mediation: &MediationModel) -> (&'static str, String) {
    if let Some(platform) = publisher
        .declared_platforms
        .iter()
        .find_map(|value| Platform::parse(value))
    {
        return (
            platform_key(platform),
            format!("{} (declared)", platform_name(platform)),
        );
    }
    if let Some(report) = publisher.ads_txt() {
        let posterior = mediation.posterior(mediation_features(&report));
        let mut best = 0usize;
        for (index, value) in posterior.iter().enumerate() {
            if *value > posterior[best] {
                best = index;
            }
        }
        let platform = Platform::ALL[best];
        return (
            platform_key(platform),
            format!(
                "{} (inferred {})",
                platform_name(platform),
                percent(posterior[best])
            ),
        );
    }
    ("none", "Not available (no app-ads.txt)".to_string())
}

fn platform_key(platform: Platform) -> &'static str {
    match platform {
        Platform::Max => "max",
        Platform::LevelPlay => "levelplay",
        Platform::AdMob => "admob",
        Platform::Other => "other",
    }
}

fn platform_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Max => "AppLovin MAX",
        Platform::LevelPlay => "Unity LevelPlay",
        Platform::AdMob => "AdMob",
        Platform::Other => "Other",
    }
}

/// The suppression reason label.
pub fn suppression_label(reason: apex_sales::policy::SuppressionReason) -> &'static str {
    match reason {
        apex_sales::policy::SuppressionReason::OptOut => "opted out",
        apex_sales::policy::SuppressionReason::Complaint => "spam complaint",
        apex_sales::policy::SuppressionReason::HardBounce => "hard bounce",
        apex_sales::policy::SuppressionReason::Operator => "operator block",
    }
}

/// Build every lead row (unfiltered), sorted by domain then address.
pub fn all_rows(data: &RequestData) -> Vec<LeadRow> {
    let mut rows = Vec::new();
    // One index for the whole page: a per-row scan of the suppression list
    // would make the page quadratic in contacts × suppressions.
    let suppressions = data.suppression_index();
    for (email, contact) in &data.snap.contacts {
        let publisher = data.snap.publishers.get(&contact.publisher_key);
        let (platform_key, platform_label) = match publisher {
            Some(publisher) => {
                let (key, label) = platform_of(publisher, &data.learned.mediation);
                (key.to_string(), label)
            }
            None => (
                "none".to_string(),
                "Not available (no publisher record)".to_string(),
            ),
        };
        let band = SizeBand::of(publisher.and_then(Publisher::max_rating_count));
        let (score, reasons) = match publisher {
            Some(publisher) => {
                let fv = features(publisher, contact, &data.learned.mediation);
                let score = data.learned.scoring.predict(&fv.x);
                let mut reasons = data.learned.scoring.reasons(&fv);
                reasons.sort_by(|a, b| {
                    b.log_odds
                        .abs()
                        .total_cmp(&a.log_odds.abs())
                        .then_with(|| a.feature.cmp(b.feature))
                });
                reasons.truncate(3);
                (
                    score,
                    reasons
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect(),
                )
            }
            None => (f64::NAN, Vec::new()),
        };
        let (evidence_count, evidence_kinds, latest_evidence_day) = match publisher {
            Some(publisher) => {
                let mut kinds: Vec<&str> = publisher
                    .evidence
                    .iter()
                    .map(|evidence| evidence.kind.as_str())
                    .collect();
                kinds.sort_unstable();
                kinds.dedup();
                let latest = publisher.evidence.iter().map(|e| e.observed_at).max();
                (
                    publisher.evidence.len(),
                    kinds.join(", "),
                    latest.map(|at| date_short(at * 1000)),
                )
            }
            None => (0, String::new(), None),
        };
        let suppressed = suppressions.check(contact).map(suppression_label);
        rows.push(LeadRow {
            email: email.clone(),
            publisher_key: contact.publisher_key.clone(),
            domain: publisher.and_then(|p| p.domain.clone()),
            name: publisher
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "Not available (publisher record missing)".to_string()),
            country: publisher.and_then(|p| p.country.clone()),
            platform_key,
            platform_label,
            band,
            score,
            reasons,
            suppressed,
            evidence_count,
            evidence_kinds,
            latest_evidence_day,
            source_class: source_class(&contact.source),
            contact_name: contact.name.clone(),
            contact_role: contact.role.clone(),
            contact_kind: contact.kind.as_str(),
            consent: contact.consent,
            source_raw: contact.source.clone(),
        });
    }
    rows.sort_by(|a, b| {
        a.publisher_key
            .cmp(&b.publisher_key)
            .then_with(|| a.email.cmp(&b.email))
    });
    rows
}

/// Apply the filters.
pub fn filter_rows(rows: Vec<LeadRow>, filters: &Filters) -> Vec<LeadRow> {
    rows.into_iter()
        .filter(|row| match filters.status.as_str() {
            "active" => row.suppressed.is_none(),
            "suppressed" => row.suppressed.is_some(),
            _ => true,
        })
        .filter(|row| filters.source == "all" || row.source_class == filters.source.as_str())
        .filter(|row| filters.platform == "all" || row.platform_key == filters.platform)
        .filter(|row| filters.band == "all" || row.band.as_str() == filters.band)
        .collect()
}

/// Render `/leads`.
pub fn render(meta: &PageMeta, data: &RequestData, filters: &Filters) -> String {
    let rows = filter_rows(all_rows(data), filters);
    let total = data.snap.contacts.len();
    let export_query = filters.query();
    let export_href = if export_query.is_empty() {
        "/leads/export.csv".to_string()
    } else {
        format!("/leads/export.csv?{export_query}")
    };

    // Bound the rendered table; the export still carries every matching
    // row, so a large imported list cannot turn the page into a megabyte.
    const PAGE_CAP: usize = 500;
    let shown = rows.len().min(PAGE_CAP);
    let table_rows: Vec<Vec<Cell>> = rows
        .iter()
        .take(shown)
        .map(|row| {
            let domain_label = row
                .domain
                .clone()
                .unwrap_or_else(|| row.publisher_key.clone());
            let status = match row.suppressed {
                Some(reason) => Cell::Status(
                    format!("Suppressed ({reason})"),
                    apex_ui::status::StatusKind::Failure,
                ),
                None => Cell::Status("Active".to_string(), apex_ui::status::StatusKind::Verified),
            };
            let score = if row.score.is_finite() {
                percent(row.score)
            } else {
                apex_ui::format::or_not_available("no publisher record")
            };
            let evidence = if row.evidence_count == 0 {
                "Not available (no evidence yet)".to_string()
            } else if row.evidence_kinds.is_empty() {
                row.evidence_count.to_string()
            } else {
                format!("{} ({})", row.evidence_count, row.evidence_kinds)
            };
            vec![
                Cell::Link(domain_label, format!("/publishers/{}", row.publisher_key)),
                Cell::Text(row.name.clone()),
                match &row.country {
                    Some(country) => Cell::Text(country.clone()),
                    None => Cell::Muted("Not available (no country)".to_string()),
                },
                Cell::Text(row.platform_label.clone()),
                Cell::Text(row.band.as_str().to_string()),
                Cell::Num(score),
                if row.reasons.is_empty() {
                    Cell::Muted("Not available (no publisher record)".to_string())
                } else {
                    Cell::Text(row.reasons.join("; "))
                },
                status,
                Cell::Text(evidence),
            ]
        })
        .collect();

    let note = if rows.len() > shown {
        format!(
            "Showing the first {shown} of {} matching contacts; the export carries all of them.",
            rows.len()
        )
    } else if filters.is_default() {
        shared::count_note(rows.len(), total, "contacts")
    } else {
        format!(
            "{} (filters applied; the export carries exactly these rows)",
            shared::count_note(rows.len(), total, "contacts")
        )
    };
    let empty = EmptyState {
        reason: if total == 0 {
            "No contacts are tracked yet. Import a seed list (apex-sales import-seeds seeds.csv) or the free-audit leads (apex-sales import-leads leads.jsonl).".to_string()
        } else {
            format!(
                "No contacts match these filters. {total} contacts are tracked; clear the filters to see them."
            )
        },
        action: if filters.is_default() {
            None
        } else {
            Some(("Clear filters".to_string(), "/leads".to_string()))
        },
    };
    let table = data_table(
        Some("Contacts"),
        &[
            "Domain",
            "Name",
            "Country",
            "Platform",
            "Band",
            "Score",
            "Top reasons",
            "Status",
            "Evidence",
        ],
        &table_rows,
        Some(empty),
        Some(&note),
    );
    let body = format!(
        "{}\n{}\n{}\n{}\n{}",
        filter_form(filters, &note),
        action("Export the filtered view (CSV)", &export_href, false),
        table,
        "<p class=\"table-note\">Platform: declared on the free-audit form, else inferred from app-ads.txt DIRECT lines by the engine's mediation model; “Not available” means neither exists. Score: the log score's predicted interest probability over the sequence, with the three largest log-odds contributions.</p>",
        "<p class=\"table-note\">The export is the filtered view: it carries exactly the rows the filters show, with UTC days, RFC 4180 quoting and spreadsheet-formula neutralisation on text cells.</p>"
    );
    shared::page(meta, &body)
}

fn filter_form(filters: &Filters, note: &str) -> String {
    let filter_field =
        |id: &str, label: &str, name: &str, options: &[(&str, &str)], current: &str| {
            field(
                id,
                label,
                None,
                None,
                &select(
                    id,
                    name,
                    &options
                        .iter()
                        .map(|(value, text)| ((*value).to_string(), (*text).to_string()))
                        .collect::<Vec<_>>(),
                    Some(current),
                ),
            )
        };
    format!(
        "<form class=\"filter-bar\" method=\"get\">\n{}{}{}{}<button class=\"action action-primary\" type=\"submit\">Apply filters</button>\n<a class=\"filter-clear\" href=\"/leads\">Clear filters</a>\n</form>\n<p class=\"table-note\">{}</p>",
        filter_field(
            "status",
            "Status",
            "status",
            &[("all", "All statuses"), ("active", "Active"), ("suppressed", "Suppressed")],
            &filters.status
        ),
        filter_field(
            "source",
            "Source",
            "source",
            &[
                ("all", "All sources"),
                ("seed", "Seed list"),
                ("free-audit", "Free-audit form"),
                ("app-ads", "app-ads.txt contact"),
            ],
            &filters.source
        ),
        filter_field(
            "platform",
            "Platform",
            "platform",
            &[
                ("all", "All platforms"),
                ("max", "AppLovin MAX"),
                ("levelplay", "Unity LevelPlay"),
                ("admob", "AdMob"),
                ("other", "Other"),
                ("none", "Not available"),
            ],
            &filters.platform
        ),
        filter_field(
            "band",
            "Size band",
            "band",
            &[
                ("all", "All size bands"),
                ("unknown", "Unknown"),
                ("small", "Small (under 2,000 ratings)"),
                ("core", "Core (2,000 to 50,000 ratings)"),
                ("large", "Large (over 50,000 ratings)"),
            ],
            &filters.band
        ),
        html_escape(note)
    )
}

/// The CSV export of the filtered view: RFC 4180, CR/LF line ends, numeric
/// cells through [`csv_field`], text cells through [`csv_text_field`].
pub fn export_csv(data: &RequestData, filters: &Filters) -> String {
    let rows = filter_rows(all_rows(data), filters);
    let mut out = String::new();
    let headers = [
        "domain",
        "name",
        "country",
        "platform",
        "band",
        "score",
        "status",
        "evidence_count",
        "evidence_kinds",
        "contact_email",
        "contact_name",
        "contact_kind",
        "contact_role",
        "contact_source",
        "latest_evidence_day",
    ];
    out.push_str(
        &headers
            .iter()
            .map(|header| csv_text_field(header))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push_str("\r\n");
    for row in &rows {
        let domain = row
            .domain
            .clone()
            .unwrap_or_else(|| "Not available (no domain)".to_string());
        let country = row
            .country
            .clone()
            .unwrap_or_else(|| "Not available (no country)".to_string());
        let status = match row.suppressed {
            Some(reason) => format!("suppressed ({reason})"),
            None => "active".to_string(),
        };
        let score = if row.score.is_finite() {
            csv_field(&format!("{:.6}", row.score))
        } else {
            csv_field("")
        };
        let evidence_kinds = if row.evidence_kinds.is_empty() {
            "Not available (no evidence)".to_string()
        } else {
            row.evidence_kinds.clone()
        };
        let latest = row
            .latest_evidence_day
            .clone()
            .unwrap_or_else(|| "Not available (no evidence)".to_string());
        let cells = [
            csv_text_field(&domain),
            csv_text_field(&row.name),
            csv_text_field(&country),
            csv_text_field(&row.platform_label),
            csv_text_field(row.band.as_str()),
            score,
            csv_text_field(&status),
            csv_field(&row.evidence_count.to_string()),
            csv_text_field(&evidence_kinds),
            csv_text_field(&row.email),
            csv_text_field(row.contact_name.as_deref().unwrap_or("")),
            csv_text_field(row.contact_kind),
            csv_text_field(row.contact_role.as_deref().unwrap_or("")),
            csv_text_field(&row.source_raw),
            csv_text_field(&latest),
        ];
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_sales::model::DAY;
    use apex_sales::store::{Event, StateDir};

    fn seeded(dir: &std::path::Path, now: i64) -> RequestData {
        let store = StateDir::open(dir).unwrap();
        let mut publisher = apex_sales::model::Publisher::new("studio.example", "Studio \"Q\"");
        publisher.domain = Some("studio.example".into());
        publisher.country = Some("FI".into());
        publisher.apps.push(apex_sales::model::App {
            store: apex_sales::model::Store::AppStore,
            id: "42".into(),
            name: "Tile Quest".into(),
            genre: Some("Games".into()),
            rating_count: Some(20_000),
            free: Some(true),
            days_since_update: Some(3),
        });
        publisher.ads_txt_text = Some("applovin.com, 1, DIRECT\n".into());
        publisher.evidence.push(apex_sales::model::Evidence {
            id: "ads-missing".into(),
            kind: apex_sales::model::EvidenceKind::AppAdsTxt,
            source: "https://studio.example/app-ads.txt".into(),
            statement: "https://studio.example/app-ads.txt was not found".into(),
            observed_at: now - DAY,
        });
        store.put_publisher(&publisher).unwrap();
        store
            .put_contact(&apex_sales::model::Contact {
                email: "maria@studio.example".into(),
                name: Some("Maria".into()),
                role: Some("CEO".into()),
                publisher_key: "studio.example".into(),
                kind: apex_sales::model::ContactKind::Named,
                source: "seed list".into(),
                consent: false,
            })
            .unwrap();
        store
            .put_contact(&apex_sales::model::Contact {
                email: "spam@evil.example".into(),
                name: None,
                role: None,
                publisher_key: "evil.example".into(),
                kind: apex_sales::model::ContactKind::Named,
                source: "https://evil.example/app-ads.txt".into(),
                consent: false,
            })
            .unwrap();
        store
            .put_event(&Event::Suppress {
                at: now - DAY,
                email: Some("spam@evil.example".into()),
                publisher: None,
            })
            .unwrap();
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        crate::data::load_from(&env, now).unwrap()
    }

    const NOW: i64 = 20_745 * DAY + 10 * 3600;

    #[test]
    fn rows_carry_platform_band_score_and_status() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-leads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = seeded(&dir, NOW);
        let rows = all_rows(&data);
        assert_eq!(rows.len(), 2);
        let maria = rows
            .iter()
            .find(|row| row.email == "maria@studio.example")
            .unwrap();
        assert_eq!(maria.platform_key, "max");
        assert!(
            maria.platform_label.contains("inferred"),
            "{}",
            maria.platform_label
        );
        assert_eq!(maria.band, SizeBand::Core);
        assert!(maria.score > 0.0 && maria.score < 1.0);
        assert!(!maria.reasons.is_empty());
        assert!(maria.suppressed.is_none());
        assert_eq!(maria.evidence_count, 1);
        let spam = rows
            .iter()
            .find(|row| row.email == "spam@evil.example")
            .unwrap();
        assert_eq!(spam.suppressed, Some("operator block"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filters_narrow_and_unknown_values_fall_back() {
        let dir =
            std::env::temp_dir().join(format!("apex-sales-web-leads2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = seeded(&dir, NOW);
        let suppressed = Filters::from_query(&[("status".to_string(), "suppressed".to_string())]);
        assert_eq!(suppressed.status, "suppressed");
        let rows = filter_rows(all_rows(&data), &suppressed);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].email, "spam@evil.example");
        let hostile = Filters::from_query(&[
            ("status".to_string(), "\"><script>".to_string()),
            ("band".to_string(), "core".to_string()),
        ]);
        assert_eq!(hostile.status, "all");
        assert_eq!(hostile.band, "core");
        assert_eq!(hostile.query(), "band=core");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_csv_export_quotes_neutralizes_and_uses_crlf() {
        let dir =
            std::env::temp_dir().join(format!("apex-sales-web-leads3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = seeded(&dir, NOW);
        let csv = export_csv(&data, &Filters::default());
        assert!(csv.contains("\r\n"), "CR/LF line ends");
        assert!(csv.ends_with("\r\n"));
        assert!(csv.contains("domain,name,country,platform,band,score,status"));
        assert!(
            csv.contains("\"Studio \"\"Q\"\"\""),
            "quoted name field: {csv}"
        );
        assert!(csv.contains("Oct 2026"), "UTC day cells via date_short");
        let filtered = export_csv(
            &data,
            &Filters::from_query(&[("status".to_string(), "suppressed".to_string())]),
        );
        assert!(!filtered.contains("maria@studio.example"));
        assert!(filtered.contains("spam@evil.example"));
        // A formula-like cell is neutralized with a leading apostrophe.
        let hostile = csv_text_field("=cmd()");
        assert_eq!(hostile, "'=cmd()");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
