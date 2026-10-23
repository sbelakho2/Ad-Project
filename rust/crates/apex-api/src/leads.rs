//! Lead capture for the free audit.
//!
//! A publisher is asked to leave a contact and the shape of their portfolio
//! before anyone talks about a contract. Leads are stored append-only (one
//! JSON object per line) and, when a mail webhook is configured, forwarded to
//! it as a product event so the follow-up can live in the same system the team
//! already reads.
//!
//! The same store serves the JSON route (`POST /v1/leads`) and the
//! server-rendered free-audit form (`POST /free-audit`). The form flow adds a
//! human studio name (`display_name`) next to the slug-shaped `publisher`
//! field, the revenue band, free-text notes and a stable `request_id`.
//!
//! ## Retry-safe request ids
//!
//! Every stored request gets `req_` plus 10 hex characters of a salted
//! SHA-256 over `(email, studio, platforms, revenue band, notes)`; the id
//! therefore carries no personal data by itself and is stable for identical
//! submissions, while a submission that differs in any hashed field (the
//! revenue band included) is a distinct lead. Within a 24-hour window an
//! identical submission returns the earlier request id and records nothing
//! new, so a browser retry of the form cannot create a second lead.
//!
//! The salt comes from `APEX_LEAD_ID_SALT` when the deployment sets one. When
//! it is unset, [`FileLeadStore`] generates 32 random bytes once, stores them
//! hex-encoded beside the leads file (`<leads file>.salt`, created with mode
//! 0600) and reuses them on every later start: ids are stable across
//! restarts, per deployment, and never computable from the public source. An
//! in-process caller may still pass an explicit salt to [`FileLeadStore`]; the
//! public default constant is only a fallback for the analytics emitter, not
//! a stored-lead salt.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use apex_connectors::transport::{HttpBody, HttpMethod, HttpRequest, Transport, TransportError};
use apex_core::PlatformId;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;

use crate::error::ApiError;

/// Fallback salt for in-process ids when no deployment salt is configured
/// (the analytics emitter's ids of rejected submissions and in-memory stores).
/// [`FileLeadStore`] never uses it: without `APEX_LEAD_ID_SALT` the file store
/// derives (or generates once) its own persistent salt beside the leads file.
pub const DEFAULT_LEAD_ID_SALT: &str = "apex-free-audit-v1";

/// Window in milliseconds inside which an identical submission is treated as
/// a retry of the same request (24 hours).
pub const DEDUP_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

/// Cap on the bytes the store scans (or re-reads) to rebuild its dedup index.
pub const MAX_LEAD_SCAN_BYTES: u64 = 32 * 1024 * 1024;

/// The exact revenue bands the free-audit form offers.
pub const REVENUE_BANDS: [&str; 5] = [
    "Under $10,000",
    "$10,000 to $50,000",
    "$50,000 to $100,000",
    "$100,000 to $300,000",
    "Over $300,000",
];

/// The canonical platform values of both lead surfaces, as `(wire value,
/// label)`. The HTML form and the stored lead use these values.
pub const PLATFORM_CHOICES: [(&str, &str); 3] = [
    ("max", "MAX"),
    ("levelplay", "LevelPlay"),
    ("admob", "AdMob"),
];

/// The marketing form's revenue slugs, as `(slug, canonical band)`.
pub const REVENUE_ALIASES: [(&str, &str); 5] = [
    ("under-10k", "Under $10,000"),
    ("10k-50k", "$10,000 to $50,000"),
    ("50k-100k", "$50,000 to $100,000"),
    ("100k-300k", "$100,000 to $300,000"),
    ("over-300k", "Over $300,000"),
];

/// Map a submitted platform value to the canonical lead spelling (`max`,
/// `levelplay`, `admob`). The accepted spellings and their canonical platform
/// names come from [`crate::sources::canonical_platform_name`], the one
/// vocabulary the lead surfaces and the URL/file parser both derive from;
/// [`PLATFORM_CHOICES`] supplies the lead wire spelling of each canonical
/// name, so the lead forms and [`crate::sources::parse_platform`] cannot
/// drift apart. Unknown values are returned trimmed and lowercased so
/// validation can refuse them while the caller keeps a safe value to
/// preserve.
pub fn canonical_platform(value: &str) -> String {
    let Some(canonical) = crate::sources::canonical_platform_name(value) else {
        return value.trim().to_ascii_lowercase();
    };
    PLATFORM_CHOICES
        .iter()
        .find(|(wire, _)| crate::sources::canonical_platform_name(wire) == Some(canonical))
        .map_or_else(|| canonical.to_string(), |(wire, _)| (*wire).to_string())
}

/// Map a submitted revenue value (an exact band or a marketing slug) to the
/// canonical band. Unknown values are returned trimmed.
pub fn canonical_revenue(value: &str) -> String {
    let value = value.trim();
    for (slug, band) in REVENUE_ALIASES {
        if slug.eq_ignore_ascii_case(value) {
            return band.to_string();
        }
    }
    value.to_string()
}

/// Exact field-error copy of the free-audit request page (PDF page 29).
pub const EMAIL_ERROR: &str = "Enter an email address we can reply to.";
/// Studio field error.
pub const STUDIO_ERROR: &str = "Enter your publisher or studio name.";
/// Platforms field error.
pub const PLATFORM_ERROR: &str = "Choose at least one platform.";
/// Notes field error.
pub const NOTES_ERROR: &str = "Shorten this note to 2,000 characters.";
/// Revenue field error (the form only offers the exact bands).
pub const REVENUE_ERROR: &str = "Choose a revenue band from the list.";
/// Error-summary line shown above the form.
pub const FORM_SUMMARY: &str = "Check the highlighted fields before sending.";

/// Submission-failure copy (PDF page 29).
pub const SUBMISSION_FAILURE_TITLE: &str = "Your request was not confirmed.";
/// Submission-failure body.
pub const SUBMISSION_FAILURE_BODY: &str =
    "We could not confirm that the request was saved. Your details remain in the form so you can \
     try again. You can also email hello@apexmediation.ee.";
/// Submission-failure retry action.
pub const TRY_AGAIN_LABEL: &str = "Try again";

/// One captured lead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lead {
    pub publisher: String,
    pub email: String,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub monthly_revenue_range: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// The studio name as the visitor typed it (spaces allowed). The
    /// `publisher` field stays the id-shaped slug.
    #[serde(default)]
    pub display_name: Option<String>,
    /// The stable, personal-data-free request id (`req_` + 10 hex).
    #[serde(default)]
    pub request_id: Option<String>,
}

/// Input validation for a lead. Kept separate from storage so a malformed lead
/// is refused before anything is written.
pub fn validate_lead(lead: &Lead) -> Result<(), String> {
    if !crate::is_valid_publisher(&lead.publisher) {
        return Err("publisher must be a valid publisher name".to_string());
    }
    validate_email(&lead.email)?;
    if lead.platforms.is_empty() {
        return Err("at least one platform is required".to_string());
    }
    for platform in &lead.platforms {
        let canonical = canonical_platform(platform);
        if !PLATFORM_CHOICES.iter().any(|(name, _)| *name == canonical) {
            return Err(format!("unknown platform '{platform}'"));
        }
    }
    if let Some(display_name) = &lead.display_name {
        let display_name = display_name.trim();
        if display_name.is_empty() || display_name.chars().count() > 120 {
            return Err("display name must be 1 to 120 characters".to_string());
        }
    }
    if let Some(range) = &lead.monthly_revenue_range {
        let canonical = canonical_revenue(range);
        if !REVENUE_BANDS.contains(&canonical.as_str()) {
            return Err("revenue band is not one of the offered bands".to_string());
        }
    }
    if let Some(notes) = &lead.notes {
        if notes.chars().count() > 2000 {
            return Err("notes must be at most 2,000 characters".to_string());
        }
    }
    Ok(())
}

/// The shared email rule of both the JSON route and the free-audit form.
fn validate_email(email: &str) -> Result<(), String> {
    let email = email.trim();
    if email.len() < 6 || email.len() > 320 {
        return Err("email length is out of range".to_string());
    }
    let (local, domain) = email
        .split_once('@')
        .ok_or_else(|| "email must contain '@'".to_string())?;
    if local.is_empty() || !domain.contains('.') || email.contains(char::is_whitespace) {
        return Err("email is not well formed".to_string());
    }
    Ok(())
}

/// A parsed free-audit form submission, before validation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FreeAuditRequest {
    pub email: String,
    pub studio: String,
    pub platforms: Vec<String>,
    pub revenue: Option<String>,
    pub notes: Option<String>,
}

/// One field error of the free-audit form, linked by field id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    /// The id of the field the summary links to.
    pub field: &'static str,
    /// The exact error copy shown at the field.
    pub message: String,
}

/// A rejected free-audit submission: the safe values to preserve and the
/// field errors to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeAuditInvalid {
    /// The values to render back into the form (never the raw body).
    pub request: FreeAuditRequest,
    /// The field errors, in form order.
    pub errors: Vec<FieldError>,
}

/// Parse and validate a free-audit form body. A body that cannot even be
/// parsed as `application/x-www-form-urlencoded` yields one summary-level
/// error with an empty form, never a raw echo. The failure is boxed so the
/// error type stays small at every call site.
pub fn parse_free_audit_request(body: &[u8]) -> Result<FreeAuditRequest, Box<FreeAuditInvalid>> {
    let fields = match parse_form(body) {
        Ok(fields) => fields,
        Err(_) => {
            return Err(Box::new(FreeAuditInvalid {
                request: FreeAuditRequest::default(),
                errors: vec![FieldError {
                    field: "summary",
                    message: FORM_SUMMARY.to_string(),
                }],
            }));
        }
    };
    let platforms = fields
        .get("platforms")
        .map(|raw| normalized_platforms(raw))
        .unwrap_or_default();
    let request = FreeAuditRequest {
        email: fields.get("email").cloned().unwrap_or_default(),
        studio: fields.get("studio").cloned().unwrap_or_default(),
        platforms,
        revenue: scalar(fields.get("revenue")).map(|value| canonical_revenue(&value)),
        notes: scalar(fields.get("notes")),
    };
    let errors = validate_free_audit(&request);
    if errors.is_empty() {
        Ok(request)
    } else {
        Err(Box::new(FreeAuditInvalid { request, errors }))
    }
}

/// Validate the free-audit form fields, returning errors in form order.
pub fn validate_free_audit(request: &FreeAuditRequest) -> Vec<FieldError> {
    let mut errors = Vec::new();
    if validate_email(&request.email).is_err() {
        errors.push(FieldError {
            field: "email",
            message: EMAIL_ERROR.to_string(),
        });
    }
    let studio = request.studio.trim();
    if studio.is_empty() || studio.chars().count() > 120 {
        errors.push(FieldError {
            field: "studio",
            message: STUDIO_ERROR.to_string(),
        });
    }
    let platforms_valid = !request.platforms.is_empty()
        && request.platforms.iter().all(|platform| {
            let canonical = canonical_platform(platform);
            PLATFORM_CHOICES.iter().any(|(name, _)| *name == canonical)
        });
    if !platforms_valid {
        errors.push(FieldError {
            field: "platforms",
            message: PLATFORM_ERROR.to_string(),
        });
    }
    if let Some(revenue) = &request.revenue {
        let canonical = canonical_revenue(revenue);
        if !REVENUE_BANDS.contains(&canonical.as_str()) {
            errors.push(FieldError {
                field: "revenue",
                message: REVENUE_ERROR.to_string(),
            });
        }
    }
    if request
        .notes
        .as_deref()
        .is_some_and(|notes| notes.chars().count() > 2000)
    {
        errors.push(FieldError {
            field: "notes",
            message: NOTES_ERROR.to_string(),
        });
    }
    errors
}

/// Split a repeated/joined platforms field into deduplicated canonical
/// values. Unknown values are kept (canonicalized only) so validation refuses
/// them with the field error while the request still carries the safe text.
fn normalized_platforms(raw: &str) -> Vec<String> {
    let mut platforms: Vec<String> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for item in raw.split(',') {
        let item = canonical_platform(item);
        if !item.is_empty() && seen.insert(item.clone()) {
            platforms.push(item);
        }
    }
    platforms
}

/// Canonicalize the platform list of a deserialized lead in place.
fn normalize_lead_platforms(platforms: &mut Vec<String>) {
    let mut normalized: Vec<String> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for platform in platforms.drain(..) {
        let platform = canonical_platform(&platform);
        if !platform.is_empty() && seen.insert(platform.clone()) {
            normalized.push(platform);
        }
    }
    *platforms = normalized;
}

/// An empty optional scalar becomes `None`.
fn scalar(value: Option<&String>) -> Option<String> {
    value.cloned().filter(|value| !value.trim().is_empty())
}

/// The slug-shaped publisher id for a studio name: lowercase ASCII
/// alphanumerics plus `-` and `_`, runs of other characters collapsed to one
/// `-`, at most 48 characters. An empty result becomes `audit-request`.
pub fn publisher_slug(studio: &str) -> String {
    let mut slug = String::new();
    for ch in studio.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '_' {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' && (slug.is_empty() || slug.ends_with('-')) {
            continue;
        }
        slug.push(mapped);
    }
    let slug = slug
        .trim_end_matches('-')
        .chars()
        .take(48)
        .collect::<String>();
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "audit-request".to_string()
    } else {
        slug
    }
}

/// The request id for a set of submission fields: `req_` plus 10 hex
/// characters of SHA-256 over the salt and the normalized fields (email,
/// studio, canonical platforms, canonical revenue band, notes). The id is
/// stable for identical submissions and carries no personal data by itself;
/// a submission that differs in any hashed field, the revenue band included,
/// is a different request id.
pub fn request_id_for_fields(
    salt: &str,
    email: &str,
    studio: &str,
    platforms: &[String],
    monthly_revenue_range: Option<&str>,
    notes: Option<&str>,
) -> String {
    let mut sorted: Vec<String> = platforms
        .iter()
        .map(|platform| canonical_platform(platform))
        .filter(|platform| !platform.is_empty())
        .collect();
    sorted.sort_unstable();
    sorted.dedup();
    let parts = [
        email.trim().to_lowercase(),
        studio.trim().to_string(),
        sorted.join(","),
        canonical_revenue(monthly_revenue_range.unwrap_or("")),
        notes.unwrap_or("").trim().to_string(),
    ];
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    for part in &parts {
        hasher.update([0x1f]);
        hasher.update(part.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    format!("req_{}", &digest[..10])
}

/// The request id for a stored lead, using its display name when present and
/// its stored revenue band.
pub fn request_id_for_lead(salt: &str, lead: &Lead) -> String {
    let studio = lead.display_name.as_deref().unwrap_or(&lead.publisher);
    request_id_for_fields(
        salt,
        &lead.email,
        studio,
        &lead.platforms,
        lead.monthly_revenue_range.as_deref(),
        lead.notes.as_deref(),
    )
}

/// Parse a lead from a JSON body or a plain HTML `application/x-www-form-urlencoded`
/// form, so the marketing page's form works without JavaScript.
pub fn parse_lead(body: &[u8], content_type: Option<&str>) -> Result<Lead, String> {
    if content_type
        .unwrap_or_default()
        .to_ascii_lowercase()
        .contains("application/x-www-form-urlencoded")
    {
        let fields = parse_form(body)?;
        let platforms = fields
            .get("platforms")
            .map(|raw| normalized_platforms(raw))
            .unwrap_or_default();
        Ok(Lead {
            publisher: fields.get("publisher").cloned().unwrap_or_default(),
            email: fields.get("email").cloned().unwrap_or_default(),
            platforms,
            monthly_revenue_range: fields
                .get("monthlyRevenueRange")
                .map(|value| canonical_revenue(value)),
            notes: fields.get("notes").cloned(),
            display_name: fields.get("displayName").cloned(),
            request_id: fields.get("requestId").cloned(),
        })
    } else {
        let mut lead: Lead = serde_json::from_slice(body).map_err(|error| error.to_string())?;
        normalize_lead_platforms(&mut lead.platforms);
        if let Some(range) = lead.monthly_revenue_range.take() {
            lead.monthly_revenue_range = Some(canonical_revenue(&range));
        }
        Ok(lead)
    }
}

pub(crate) fn parse_form(
    body: &[u8],
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let text = std::str::from_utf8(body).map_err(|_| "form body is not UTF-8".to_string())?;
    let mut fields = std::collections::BTreeMap::new();
    for pair in text.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key)?;
        let value = percent_decode(value)?;
        // A repeated key (a multiple checkbox) is joined with a comma.
        fields
            .entry(key)
            .and_modify(|existing: &mut String| {
                existing.push(',');
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    Ok(fields)
}

/// Percent-decode a form component (`+` is a space, `%xx` its byte).
fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' => {
                let high = bytes
                    .get(index + 1)
                    .and_then(|b| hex_value(*b))
                    .ok_or("bad percent escape")?;
                let low = bytes
                    .get(index + 2)
                    .and_then(|b| hex_value(*b))
                    .ok_or("bad percent escape")?;
                out.push((high << 4) | low);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| "form component is not UTF-8".to_string())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Where leads are kept. `store` returns the stable request id of the lead
/// (the deduplicated one when an identical submission arrived within the last
/// 24 hours).
#[async_trait::async_trait]
pub trait LeadStore: Send + Sync + std::fmt::Debug {
    async fn store(&self, lead: &Lead, at_unix_ms: i64) -> Result<String, ApiError>;
}

/// Append-only JSONL lead store, optionally forwarding each lead to a mail
/// webhook (the ApexMail hook).
///
/// The store keeps an in-memory index of request ids seen in the last 24
/// hours, rebuilt from the file on first use, so an identical retry returns
/// the original id without appending a second line or sending a second
/// webhook; a saved lead is success even when the webhook fails.
///
/// The id salt is the explicitly configured one when given; otherwise the
/// store reads (or generates once, mode 0600) `<leads file>.salt` beside the
/// leads file, so ids are stable across restarts and differ per deployment.
pub struct FileLeadStore {
    path: PathBuf,
    webhook: Option<String>,
    transport: Option<Arc<dyn Transport>>,
    salt: Option<String>,
    lock: Mutex<LeadStoreState>,
}

#[derive(Debug, Default)]
struct LeadStoreState {
    hydrated: bool,
    /// Request id -> the instant it was first recorded inside the dedup
    /// window (never renewed, so an id's timestamp is immutable).
    recent: HashMap<String, i64>,
    /// The same ids ordered by that instant. Expiry removes the expired
    /// prefix, so a submit costs the expired entries rather than a rescan of
    /// every live id at the 20k-lead reference size.
    by_seen: BTreeMap<i64, Vec<String>>,
    salt: Option<String>,
    /// Ids dropped by expiry (tests and diagnostics).
    pruned: u64,
}

impl LeadStoreState {
    /// Drop every id whose 24-hour window closed at `at_unix_ms`.
    fn prune_recent(&mut self, at_unix_ms: i64) {
        while let Some((seen, _)) = self.by_seen.first_key_value() {
            if at_unix_ms.saturating_sub(*seen) <= DEDUP_WINDOW_MS {
                break;
            }
            let Some((_, ids)) = self.by_seen.pop_first() else {
                break;
            };
            for id in ids {
                if self.recent.remove(&id).is_some() {
                    self.pruned += 1;
                }
            }
        }
    }
}

impl std::fmt::Debug for FileLeadStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileLeadStore")
            .field("path", &self.path)
            .field("has_webhook", &self.webhook.is_some())
            .field("has_salt", &self.salt.is_some())
            .finish()
    }
}

impl FileLeadStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            webhook: None,
            transport: None,
            salt: None,
            lock: Mutex::new(LeadStoreState::default()),
        }
    }

    /// Forward each stored lead to a mail webhook (the ApexMail hook).
    #[must_use]
    pub fn with_mail_webhook(
        mut self,
        url: impl Into<String>,
        transport: Arc<dyn Transport>,
    ) -> Self {
        self.webhook = Some(url.into());
        self.transport = Some(transport);
        self
    }

    /// Use a deployment-specific salt for request ids (`APEX_LEAD_ID_SALT`).
    /// The explicit salt wins over the generated `<leads file>.salt`.
    #[must_use]
    pub fn with_id_salt(mut self, salt: impl Into<String>) -> Self {
        let salt = salt.into();
        if !salt.trim().is_empty() {
            self.salt = Some(salt);
        }
        self
    }

    /// The persistent salt file beside the leads file.
    fn salt_path(&self) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(".salt");
        PathBuf::from(name)
    }

    /// The deployment salt: an explicit one from [`Self::with_id_salt`] wins;
    /// otherwise it is read from (or, on first use, generated into) the salt
    /// file beside the leads file. Resolved once per store instance.
    fn salt_for(&self, state: &mut LeadStoreState) -> Result<String, ApiError> {
        if let Some(salt) = &state.salt {
            return Ok(salt.clone());
        }
        let salt = match &self.salt {
            Some(explicit) => explicit.clone(),
            None => read_or_create_salt(&self.salt_path())?,
        };
        state.salt = Some(salt.clone());
        Ok(salt)
    }

    /// Best-effort forward: a webhook failure never loses the stored lead.
    async fn forward(&self, line: &str) {
        let (Some(url), Some(transport)) = (&self.webhook, &self.transport) else {
            return;
        };
        let request = HttpRequest {
            platform: PlatformId::Admob,
            operation: "leadsWebhook".to_string(),
            method: HttpMethod::Post,
            url: url.clone(),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("Accept".into(), "application/json".into()),
            ],
            body: Some(HttpBody::Form(line.to_string())),
            timeout_ms: 10_000,
        };
        match transport.request(request).await {
            Ok(response) if (200..300).contains(&response.status) => {}
            Ok(response) => {
                tracing::warn!(status = response.status, "lead webhook refused");
            }
            Err(TransportError::Platform(error)) => {
                tracing::warn!(%error, "lead webhook failed");
            }
            Err(error) => tracing::warn!(%error, "lead webhook transport failed"),
        }
    }

    /// Rebuild the dedup index from the append-only file (bounded to the last
    /// [`MAX_LEAD_SCAN_BYTES`]; unparseable lines are skipped). Only the first
    /// line may be a partial one when the file exceeds the scan bound.
    fn hydrate(&self, state: &mut LeadStoreState) -> Result<(), ApiError> {
        let salt = self.salt_for(state)?;
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                state.hydrated = true;
                return Ok(());
            }
            Err(error) => {
                return Err(ApiError::Internal(format!("lead file read: {error}")));
            }
        };
        let tail = if bytes.len() as u64 > MAX_LEAD_SCAN_BYTES {
            &bytes[bytes.len() - MAX_LEAD_SCAN_BYTES as usize..]
        } else {
            &bytes[..]
        };
        for (index, line) in tail.split(|byte| *byte == b'\n').enumerate() {
            if index == 0 && tail.len() != bytes.len() {
                continue; // a partial first line of the tail window
            }
            if line.is_empty() {
                continue;
            }
            let Ok(stored) = serde_json::from_slice::<StoredLeadLine>(line) else {
                continue;
            };
            let id = stored
                .lead
                .request_id
                .clone()
                .unwrap_or_else(|| request_id_for_lead(&salt, &stored.lead));
            let entry = state.recent.entry(id).or_insert(stored.at_unix_ms);
            *entry = (*entry).max(stored.at_unix_ms);
        }
        state.by_seen.clear();
        for (id, seen) in &state.recent {
            state.by_seen.entry(*seen).or_default().push(id.clone());
        }
        state.hydrated = true;
        Ok(())
    }
    /// Deduplicate and append under the lock (synchronous, so the mutex guard
    /// can never be held across an await). Returns the request id and, when a
    /// new line was appended, the line to forward to the mail webhook.
    fn append_if_new(
        &self,
        lead: &Lead,
        at_unix_ms: i64,
    ) -> Result<(String, Option<String>, Lead), ApiError> {
        let mut state = self
            .lock
            .lock()
            .map_err(|_| ApiError::Internal("lead lock poisoned".into()))?;
        if !state.hydrated {
            self.hydrate(&mut state)?;
        }
        let salt = self.salt_for(&mut state)?;
        let request_id = lead
            .request_id
            .clone()
            .unwrap_or_else(|| request_id_for_lead(&salt, lead));
        let mut stored_lead = lead.clone();
        stored_lead.request_id = Some(request_id.clone());

        state.prune_recent(at_unix_ms);
        if state.recent.contains_key(&request_id) {
            return Ok((request_id, None, stored_lead));
        }

        let record = serde_json::json!({
            "atUnixMs": at_unix_ms,
            "lead": stored_lead,
        });
        let mut line = serde_json::to_string(&record)
            .map_err(|error| ApiError::Internal(format!("lead serialization: {error}")))?;
        line.push('\n');
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ApiError::Internal(format!("lead directory: {error}")))?;
        }
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&self.path)
            .map_err(|error| ApiError::Internal(format!("lead file: {error}")))?;
        file.write_all(line.as_bytes())
            .map_err(|error| ApiError::Internal(format!("lead write: {error}")))?;
        state.recent.insert(request_id.clone(), at_unix_ms);
        state
            .by_seen
            .entry(at_unix_ms)
            .or_default()
            .push(request_id.clone());
        Ok((request_id, Some(line), stored_lead))
    }
}

/// Read the persistent salt beside the leads file, generating it on first use:
/// 32 random bytes, hex-encoded, created with mode 0600 (unix). Two stores
/// over the same leads path, and every later restart, therefore share one
/// salt while different deployments keep different ones. A concurrent creator
/// wins the race and its salt is used.
fn read_or_create_salt(path: &Path) -> Result<String, ApiError> {
    if let Some(salt) = read_salt(path)? {
        return Ok(salt);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ApiError::Internal(format!("lead salt directory: {error}")))?;
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let generated = hex::encode(bytes);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(generated.as_bytes())
                .map_err(|error| ApiError::Internal(format!("lead salt write: {error}")))?;
            Ok(generated)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_salt(path)?
            .ok_or_else(|| {
                ApiError::Internal(format!("lead salt file {} is empty", path.display()))
            }),
        Err(error) => Err(ApiError::Internal(format!(
            "lead salt create {}: {error}",
            path.display()
        ))),
    }
}

/// The salt file's trimmed contents, `None` when the file is missing or
/// empty.
fn read_salt(path: &Path) -> Result<Option<String>, ApiError> {
    match std::fs::read_to_string(path) {
        Ok(raw) => {
            let salt = raw.trim();
            if salt.is_empty() {
                Ok(None)
            } else {
                Ok(Some(salt.to_string()))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ApiError::Internal(format!(
            "lead salt read {}: {error}",
            path.display()
        ))),
    }
}

/// One append-only JSONL line: the timestamp and the stored lead.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredLeadLine {
    at_unix_ms: i64,
    lead: Lead,
}

#[async_trait::async_trait]
impl LeadStore for FileLeadStore {
    async fn store(&self, lead: &Lead, at_unix_ms: i64) -> Result<String, ApiError> {
        validate_lead(lead).map_err(|_| ApiError::BadRequest("invalid_lead"))?;
        let (request_id, line, _) = self.append_if_new(lead, at_unix_ms)?;
        if let Some(line) = line {
            self.forward(&line).await;
        }
        Ok(request_id)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Mutex;

    use apex_connectors::transport::{HttpRequest, HttpResponse, Transport, TransportError};

    use super::*;

    #[test]
    fn normalized_platforms_skip_empty_duplicates_in_order() {
        assert_eq!(
            normalized_platforms("applovin_max, ,AppLovin MAX,admob,admob"),
            vec![
                "max".to_string(),
                "applovin max".to_string(),
                "admob".to_string()
            ]
        );
    }

    fn lead() -> Lead {
        Lead {
            publisher: "pub1".into(),
            email: "founder@studio.example".into(),
            platforms: vec!["applovin_max".into()],
            monthly_revenue_range: Some("$10,000 to $50,000".into()),
            notes: None,
            display_name: None,
            request_id: None,
        }
    }

    #[test]
    fn random_percent_encodings_never_panic_and_never_expand() {
        let mut state = 0x2545f4914f6cdd1du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let alphabet = b"0123456789abcdefABCDEF%+&= /";
        for _ in 0..500 {
            let len = (next() % 120) as usize;
            let text: String = (0..len)
                .map(|_| alphabet[(next() as usize) % alphabet.len()] as char)
                .collect();
            if let Ok(decoded) = percent_decode(&text) {
                assert!(
                    decoded.len() <= text.len(),
                    "decoding never creates characters"
                );
            }
        }
    }

    #[test]
    fn validate_accepts_a_well_formed_lead() {
        assert!(validate_lead(&lead()).is_ok());
        let with_display = Lead {
            display_name: Some("Northstar Studio".into()),
            ..lead()
        };
        assert!(validate_lead(&with_display).is_ok());
    }

    #[test]
    fn validate_refuses_bad_publisher_email_platforms_band_notes_and_display() {
        let mut bad = lead();
        bad.publisher = "../etc".into();
        assert!(validate_lead(&bad).is_err());

        let mut bad = lead();
        bad.email = "no-at-sign".into();
        assert!(validate_lead(&bad).is_err());

        let mut bad = lead();
        bad.email = "a@localhost".into();
        assert!(validate_lead(&bad).is_err(), "domain needs a dot");

        let mut bad = lead();
        bad.platforms = vec!["tiktok".into()];
        assert!(validate_lead(&bad).is_err());

        let mut bad = lead();
        bad.platforms = vec![];
        assert!(validate_lead(&bad).is_err());

        let mut bad = lead();
        bad.monthly_revenue_range = Some("lots".into());
        assert!(
            validate_lead(&bad).is_err(),
            "only the offered bands are valid"
        );

        let mut alias = lead();
        alias.monthly_revenue_range = Some("10k-50k".into());
        alias.platforms = vec!["applovin_max".into()];
        assert!(
            validate_lead(&alias).is_ok(),
            "aliases stay accepted on the way in"
        );

        let mut bad = lead();
        bad.display_name = Some("   ".into());
        assert!(validate_lead(&bad).is_err());

        let mut bad = lead();
        bad.display_name = Some("x".repeat(121));
        assert!(validate_lead(&bad).is_err());

        let mut bad = lead();
        bad.notes = Some("n".repeat(2001));
        assert!(validate_lead(&bad).is_err());
    }

    #[test]
    fn parse_lead_reads_json_and_urlencoded_forms() {
        let json = br#"{"publisher":"pub1","email":"a@studio.example","platforms":["admob"],"notes":"hi"}"#;
        let parsed = parse_lead(json, Some("application/json")).expect("json");
        assert_eq!(parsed.publisher, "pub1");
        assert_eq!(parsed.platforms, vec!["admob".to_string()]);
        assert_eq!(parsed.notes.as_deref(), Some("hi"));
        assert_eq!(parsed.display_name, None);

        let json_extra = br#"{"publisher":"pub1","email":"a@studio.example","platforms":["admob"],"displayName":"Northstar Studio","requestId":"req_0123456789"}"#;
        let parsed = parse_lead(json_extra, Some("application/json")).expect("json");
        assert_eq!(parsed.display_name.as_deref(), Some("Northstar Studio"));
        assert_eq!(parsed.request_id.as_deref(), Some("req_0123456789"));

        // The historic JSON spelling and the revenue slug canonicalize on
        // both parse paths, so retries deduplicate across surfaces.
        let json_alias = br#"{"publisher":"pub1","email":"a@studio.example","platforms":["applovin_max","admob","applovin_max"],"monthlyRevenueRange":"10k-50k"}"#;
        let parsed = parse_lead(json_alias, Some("application/json")).expect("json");
        assert_eq!(parsed.platforms, vec!["max", "admob"]);
        assert_eq!(
            parsed.monthly_revenue_range.as_deref(),
            Some("$10,000 to $50,000")
        );
        assert!(validate_lead(&parsed).is_ok());

        let form =
            b"publisher=pub1&email=a%40studio.example&platforms=admob%2Clevelplay&notes=why+not";
        let parsed = parse_lead(form, Some("application/x-www-form-urlencoded")).expect("form");
        assert_eq!(parsed.email, "a@studio.example");
        assert_eq!(
            parsed.platforms,
            vec!["admob".to_string(), "levelplay".to_string()]
        );
        assert_eq!(parsed.notes.as_deref(), Some("why not"));
        assert!(validate_lead(&parsed).is_ok());

        assert!(parse_lead(b"not json", None).is_err());
        assert!(parse_lead(b"publisher=%zz", Some("application/x-www-form-urlencoded")).is_err());
    }

    #[test]
    fn free_audit_parsing_accepts_the_exact_bands_and_rejects_others() {
        let ok = b"email=you%40studio.example&studio=Northstar+Studio&platforms=max&platforms=admob&revenue=%2410%2C000+to+%2450%2C000&notes=hello";
        let parsed = parse_free_audit_request(ok).expect("valid");
        assert_eq!(parsed.email, "you@studio.example");
        assert_eq!(parsed.studio, "Northstar Studio");
        assert_eq!(
            parsed.platforms,
            vec!["max".to_string(), "admob".to_string()]
        );
        assert_eq!(parsed.revenue.as_deref(), Some("$10,000 to $50,000"));

        // The marketing form posts revenue slugs; they canonicalize to the
        // exact bands before validation.
        let slug = b"email=you%40studio.example&studio=X&platforms=applovin_max&revenue=10k-50k";
        let parsed = parse_free_audit_request(slug).expect("slug accepted");
        assert_eq!(parsed.platforms, vec!["max".to_string()]);
        assert_eq!(parsed.revenue.as_deref(), Some("$10,000 to $50,000"));

        let bad_band = b"email=you%40studio.example&studio=X&platforms=max&revenue=lots";
        let invalid = parse_free_audit_request(bad_band).expect_err("refused");
        assert_eq!(invalid.errors.len(), 1);
        assert_eq!(invalid.errors[0].field, "revenue");
        assert_eq!(invalid.errors[0].message, REVENUE_ERROR);
        // The safe preserved values survive the refusal.
        assert_eq!(invalid.request.studio, "X");
        assert_eq!(invalid.request.revenue.as_deref(), Some("lots"));

        let not_a_form = b"\xff\xfe";
        let invalid = parse_free_audit_request(not_a_form).expect_err("refused");
        assert_eq!(invalid.errors[0].message, FORM_SUMMARY);
        assert_eq!(invalid.request, FreeAuditRequest::default());
    }

    #[test]
    fn wire_values_canonicalize_to_one_stored_name() {
        // Historic JSON spelling and the form value mean the same platform.
        assert_eq!(canonical_platform("applovin_max"), "max");
        assert_eq!(canonical_platform(" MAX "), "max");
        assert_eq!(canonical_platform("levelplay"), "levelplay");
        assert_eq!(canonical_platform("admob"), "admob");
        assert_eq!(canonical_platform("TikTok"), "tiktok");
        for (slug, band) in REVENUE_ALIASES {
            assert_eq!(canonical_revenue(slug), band);
        }
        assert_eq!(canonical_revenue(" Over $300,000 "), "Over $300,000");
        assert_eq!(canonical_revenue("lots"), "lots");
    }

    #[test]
    fn free_audit_validation_reports_fields_in_order_with_exact_copy() {
        let request = FreeAuditRequest {
            email: "bad".into(),
            studio: "  ".into(),
            platforms: vec![],
            revenue: Some("nope".into()),
            notes: Some("n".repeat(2001)),
        };
        let errors = validate_free_audit(&request);
        let fields: Vec<&str> = errors.iter().map(|error| error.field).collect();
        assert_eq!(fields, ["email", "studio", "platforms", "revenue", "notes"]);
        assert_eq!(errors[0].message, EMAIL_ERROR);
        assert_eq!(errors[1].message, STUDIO_ERROR);
        assert_eq!(errors[2].message, PLATFORM_ERROR);
        assert_eq!(errors[3].message, REVENUE_ERROR);
        assert_eq!(errors[4].message, NOTES_ERROR);

        let unknown_platform = FreeAuditRequest {
            email: "you@studio.example".into(),
            studio: "Studio".into(),
            platforms: vec!["tiktok".into()],
            revenue: None,
            notes: None,
        };
        assert_eq!(validate_free_audit(&unknown_platform)[0].field, "platforms");

        let notes_at_limit = FreeAuditRequest {
            notes: Some("n".repeat(2000)),
            platforms: vec!["max".into()],
            ..unknown_platform
        };
        assert!(validate_free_audit(&notes_at_limit).is_empty());
    }

    #[test]
    fn publisher_slug_is_id_shaped_and_falls_back() {
        assert_eq!(publisher_slug("Northstar Studio"), "northstar-studio");
        assert_eq!(publisher_slug("  Dot.Com  Ltd "), "dot-com-ltd");
        assert_eq!(publisher_slug("Ünïcode"), "n-code");
        assert_eq!(publisher_slug("---"), "audit-request");
        assert_eq!(publisher_slug(""), "audit-request");
        let long = publisher_slug(&"a".repeat(80));
        assert_eq!(long.len(), 48);
        assert!(crate::is_valid_publisher(&long));
        assert!(crate::is_valid_publisher(&publisher_slug(
            "Northstar Studio"
        )));
    }

    #[test]
    fn request_ids_are_stable_short_and_personal_data_free() {
        let platforms = vec!["admob".to_string(), "max".to_string()];
        let id = request_id_for_fields(
            "salt",
            "You@Studio.example",
            "Northstar",
            &platforms,
            None,
            None,
        );
        assert!(id.starts_with("req_"));
        assert_eq!(id.len(), 14);
        assert!(id[4..].bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!id.contains("studio") && !id.contains("you"));
        // Stable under case/trim/order normalization.
        let again = request_id_for_fields(
            "salt",
            " you@studio.example ",
            " Northstar ",
            &["max".to_string(), "admob".to_string()],
            None,
            None,
        );
        assert_eq!(id, again);
        // Different salt, different id.
        let other = request_id_for_fields(
            "other",
            "you@studio.example",
            "Northstar",
            &platforms,
            None,
            None,
        );
        assert_ne!(id, other);
        // Notes participate in the identity.
        let noted = request_id_for_fields(
            "salt",
            "you@studio.example",
            "Northstar",
            &platforms,
            None,
            Some("hi"),
        );
        assert_ne!(id, noted);
        // The canonical revenue band participates too: a different band is a
        // different request, while the marketing slug and the exact band mean
        // the same one.
        let banded = request_id_for_fields(
            "salt",
            "you@studio.example",
            "Northstar",
            &platforms,
            Some("$10,000 to $50,000"),
            None,
        );
        assert_ne!(id, banded);
        let slugged = request_id_for_fields(
            "salt",
            "you@studio.example",
            "Northstar",
            &platforms,
            Some("10k-50k"),
            None,
        );
        assert_eq!(banded, slugged);
        let other_band = request_id_for_fields(
            "salt",
            "you@studio.example",
            "Northstar",
            &platforms,
            Some("$50,000 to $100,000"),
            None,
        );
        assert_ne!(banded, other_band);
    }

    #[test]
    fn request_id_for_lead_prefers_the_display_name() {
        let mut one = lead();
        one.display_name = Some("Northstar Studio".into());
        let a = request_id_for_lead("salt", &one);
        one.publisher = "different-slug".into();
        assert_eq!(a, request_id_for_lead("salt", &one));
    }

    #[derive(Default)]
    struct FakeTransport {
        seen: Mutex<Vec<HttpRequest>>,
    }

    #[async_trait::async_trait]
    impl Transport for FakeTransport {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.seen.lock().unwrap().push(req);
            Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: b"{}".to_vec(),
            })
        }
    }

    #[tokio::test]
    async fn the_store_surfaces_io_errors() {
        // A store whose "path" is a directory cannot open the file: the error
        // is typed, never a panic.
        let dir = std::env::temp_dir().join(format!("apex-leads-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(dir.with_extension("salt"));
        std::fs::create_dir_all(&dir).unwrap();
        let store = FileLeadStore::new(&dir); // path is a directory
        let err = store.store(&lead(), 1).await.expect_err("io error");
        assert!(matches!(err, ApiError::Internal(_)), "{err:?}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(dir.with_extension("salt"));
    }

    #[tokio::test]
    async fn the_store_appends_jsonl_and_forwards_to_the_webhook() {
        let dir = std::env::temp_dir().join(format!("apex-leads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("leads.jsonl");
        let transport = Arc::new(FakeTransport::default());
        let store = FileLeadStore::new(&path)
            .with_mail_webhook("https://mail.example/hook", transport.clone());
        let first = store
            .store(&lead(), 1_700_000_000_000)
            .await
            .expect("store");
        assert!(first.starts_with("req_"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "lead files carry contact PII: {mode:o}");
        }

        // A distinct lead (different notes) appends a second line.
        let mut other = lead();
        other.notes = Some("a different request".into());
        store.store(&other, 1_700_000_001_000).await.expect("store");

        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(
            contents.lines().count(),
            2,
            "append-only, one line per distinct lead"
        );
        assert!(contents.contains("\"publisher\":\"pub1\""));
        assert!(contents.contains("\"requestId\":\"req_"));
        {
            let calls = transport.seen.lock().unwrap();
            assert_eq!(calls.len(), 2, "each distinct lead is forwarded once");
            assert!(calls[0].url.contains("mail.example"));
        }

        assert!(matches!(
            store
                .store(
                    &Lead {
                        email: "bad".into(),
                        ..lead()
                    },
                    0
                )
                .await,
            Err(ApiError::BadRequest("invalid_lead"))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn identical_submissions_within_the_window_deduplicate() {
        let dir = std::env::temp_dir().join(format!("apex-leads-dedup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("leads.jsonl");
        let transport = Arc::new(FakeTransport::default());
        let store = FileLeadStore::new(&path)
            .with_mail_webhook("https://mail.example/hook", transport.clone());
        let at = 1_700_000_000_000;
        let first = store.store(&lead(), at).await.expect("first");
        // The same fields a minute later (a retry) return the same id.
        let retry = store.store(&lead(), at + 60_000).await.expect("retry");
        assert_eq!(first, retry);
        // Normalization: case and order do not create a new request.
        let mut normalized = lead();
        normalized.email = " FOUNDER@STUDIO.EXAMPLE ".into();
        normalized.platforms = vec!["applovin_max".into()];
        let normalized = store.store(&normalized, at + 120_000).await.expect("norm");
        assert_eq!(first, normalized);

        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(contents.lines().count(), 1, "no duplicate line");
        assert_eq!(transport.seen.lock().unwrap().len(), 1, "one webhook call");

        // Outside the 24-hour window the same submission is a new request.
        let later = store
            .store(&lead(), at + DEDUP_WINDOW_MS + 1)
            .await
            .expect("later");
        assert_eq!(first, later, "the id is a pure function of the fields");
        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(contents.lines().count(), 2, "window passed, appended again");

        // A fresh store over the same file rebuilds the index from disk.
        let fresh = FileLeadStore::new(&path);
        let across_restart = fresh.store(&lead(), at + DEDUP_WINDOW_MS + 2).await;
        assert_eq!(across_restart.expect("dedup after restart"), first);
        assert_eq!(
            std::fs::read_to_string(&path)
                .expect("read")
                .lines()
                .count(),
            2
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn submissions_differing_only_in_the_revenue_band_are_distinct_leads() {
        let dir = std::env::temp_dir().join(format!("apex-leads-band-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("leads.jsonl");
        let store = FileLeadStore::new(&path);
        let at = 1_700_000_000_000;
        let first_band = lead(); // "$10,000 to $50,000"
        let first = store.store(&first_band, at).await.expect("first");
        let mut second_band = lead();
        second_band.monthly_revenue_range = Some("$50,000 to $100,000".into());
        let second = store
            .store(&second_band, at + 60_000)
            .await
            .expect("second");
        assert_ne!(
            first, second,
            "a different band within the window is a different request"
        );
        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(
            contents.lines().count(),
            2,
            "the distinct band is not collapsed onto the first lead"
        );
        // The same fields (band included) still deduplicate.
        let retry = store.store(&first_band, at + 120_000).await.expect("retry");
        assert_eq!(first, retry);
        assert_eq!(
            std::fs::read_to_string(&path)
                .expect("read")
                .lines()
                .count(),
            2
        );
        // And the restart path keeps the two apart too: a fresh store over
        // the same file rehydrates both ids.
        let fresh = FileLeadStore::new(&path);
        let band_two_again = fresh.store(&second_band, at + 180_000).await;
        assert_eq!(band_two_again.expect("fresh dedup"), second);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_salt_is_generated_once_beside_the_leads_file_and_stays_stable() {
        let dir = std::env::temp_dir().join(format!("apex-leads-salt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("leads.jsonl");
        let salt_path = dir.join("leads.jsonl.salt");
        let store = FileLeadStore::new(&path);
        assert!(
            !salt_path.exists(),
            "the salt file is created lazily, on first use"
        );
        let first = store
            .store(&lead(), 1_700_000_000_000)
            .await
            .expect("store");
        let salt = std::fs::read_to_string(&salt_path).expect("salt file created on first use");
        let salt = salt.trim();
        assert_eq!(salt.len(), 64, "32 random bytes as hex");
        assert!(salt.bytes().all(|byte| byte.is_ascii_hexdigit()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&salt_path)
                .expect("salt metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the salt file is created 0600");
        }
        // The stored id is the one this salt derives.
        assert_eq!(first, request_id_for_lead(salt, &lead()));

        // A second store over the same leads path derives the same salt, so
        // ids are stable across restarts.
        let restarted = FileLeadStore::new(&path);
        let mut other = lead();
        other.notes = Some("another submission".into());
        let id = restarted
            .store(&other, 1_700_000_001_000)
            .await
            .expect("restarted store");
        assert_eq!(id, request_id_for_lead(salt, &other));

        // A different deployment path gets a different salt.
        let other_dir = dir.join("deploy-two");
        std::fs::create_dir_all(&other_dir).unwrap();
        let other_store = FileLeadStore::new(other_dir.join("leads.jsonl"));
        other_store
            .store(&lead(), 1_700_000_000_000)
            .await
            .expect("other deployment");
        let other_salt =
            std::fs::read_to_string(other_dir.join("leads.jsonl.salt")).expect("other salt");
        assert_ne!(salt, other_salt.trim());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_explicit_salt_wins_over_the_generated_file() {
        let dir =
            std::env::temp_dir().join(format!("apex-leads-salt-explicit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("leads.jsonl");
        let salt_path = dir.join("leads.jsonl.salt");
        // Seed the generated salt with one stored lead.
        let seeded_store = FileLeadStore::new(&path);
        let mut seeded = lead();
        seeded.notes = Some("seed".into());
        seeded_store
            .store(&seeded, 1_700_000_000_000)
            .await
            .expect("seed");
        let file_salt = std::fs::read_to_string(&salt_path).expect("salt file");
        let file_salt = file_salt.trim().to_string();

        // The explicit deployment salt (the env-provided one) is what the ids
        // use, and it never rewrites the generated file.
        let explicit = FileLeadStore::new(&path).with_id_salt("deployment-salt");
        let id = explicit
            .store(&lead(), 1_700_000_001_000)
            .await
            .expect("explicit store");
        assert_eq!(id, request_id_for_lead("deployment-salt", &lead()));
        assert_ne!(id, request_id_for_lead(&file_salt, &lead()));
        assert_eq!(
            std::fs::read_to_string(&salt_path)
                .expect("salt file")
                .trim(),
            file_salt
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The dedup index expires the ordered prefix instead of rescanning every
    /// live id: a 100k-lead file prunes exactly the expired entries on the
    /// first submit after the window, and the live entry count is exact.
    #[test]
    fn the_dedup_index_expires_the_prefix_at_one_hundred_thousand_leads() {
        let dir = std::env::temp_dir().join(format!("apex-leads-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("leads.jsonl");
        let at = 1_700_000_000_000i64;
        let mut body = String::with_capacity(100_000 * 160);
        for index in 0..100_000 {
            let line = serde_json::json!({
                "atUnixMs": at,
                "lead": {
                    "publisher": format!("p{index}"),
                    "email": format!("lead{index}@studio.example"),
                    "platforms": ["max"],
                }
            });
            body.push_str(&serde_json::to_string(&line).expect("line"));
            body.push('\n');
        }
        std::fs::write(&path, body).expect("seed leads");

        let store = FileLeadStore::new(&path);
        let mut warm = lead();
        warm.publisher = "warm".into();
        warm.email = "warm@studio.example".into();
        store.append_if_new(&warm, at).expect("warm submit");
        {
            let state = store.lock.lock().expect("state");
            assert_eq!(state.recent.len(), 100_001, "all hydrated ids are live");
            assert_eq!(state.pruned, 0);
        }

        let mut fresh = lead();
        fresh.publisher = "fresh".into();
        fresh.email = "fresh@studio.example".into();
        store
            .append_if_new(&fresh, at + DEDUP_WINDOW_MS + 1)
            .expect("fresh submit");
        let state = store.lock.lock().expect("state");
        assert_eq!(state.pruned, 100_001, "exactly the expired prefix");
        assert_eq!(state.recent.len(), 1, "only the fresh id remains");
        assert_eq!(state.by_seen.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_callers_request_id_is_preserved_and_used_for_dedup() {
        let dir = std::env::temp_dir().join(format!("apex-leads-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("leads.jsonl");
        let store = FileLeadStore::new(&path).with_id_salt("unit-test-salt");
        let mut pinned = lead();
        pinned.request_id = Some("req_aabbccddee".into());
        let id = store
            .store(&pinned, 1_700_000_000_000)
            .await
            .expect("store");
        assert_eq!(id, "req_aabbccddee");
        let again = store
            .store(&pinned, 1_700_000_030_000)
            .await
            .expect("retry");
        assert_eq!(again, "req_aabbccddee");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
