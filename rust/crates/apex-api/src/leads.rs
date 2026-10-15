//! Lead capture for the free audit.
//!
//! A publisher is asked to leave a contact and the shape of their portfolio
//! before anyone talks about a contract. Leads are stored append-only (one
//! JSON object per line) and, when a mail webhook is configured, forwarded to
//! it as a product event so the follow-up can live in the same system the team
//! already reads.

use std::path::PathBuf;
use std::sync::Arc;

use apex_connectors::transport::{HttpBody, HttpMethod, HttpRequest, Transport, TransportError};
use apex_core::PlatformId;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use crate::error::ApiError;

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
}

/// Input validation for a lead. Kept separate from storage so a malformed lead
/// is refused before anything is written.
pub fn validate_lead(lead: &Lead) -> Result<(), String> {
    if !crate::is_valid_publisher(&lead.publisher) {
        return Err("publisher must be a valid publisher name".to_string());
    }
    let email = lead.email.trim();
    if email.len() < 6 || email.len() > 320 {
        return Err("email length is out of range".to_string());
    }
    let (local, domain) = email
        .split_once('@')
        .ok_or_else(|| "email must contain '@'".to_string())?;
    if local.is_empty() || !domain.contains('.') || email.contains(char::is_whitespace) {
        return Err("email is not well formed".to_string());
    }
    if lead.platforms.is_empty() {
        return Err("at least one platform is required".to_string());
    }
    for platform in &lead.platforms {
        if !matches!(platform.as_str(), "applovin_max" | "levelplay" | "admob") {
            return Err(format!("unknown platform '{platform}'"));
        }
    }
    Ok(())
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
            .map(|raw| {
                raw.split(',')
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Ok(Lead {
            publisher: fields.get("publisher").cloned().unwrap_or_default(),
            email: fields.get("email").cloned().unwrap_or_default(),
            platforms,
            monthly_revenue_range: fields.get("monthlyRevenueRange").cloned(),
            notes: fields.get("notes").cloned(),
        })
    } else {
        serde_json::from_slice(body).map_err(|error| error.to_string())
    }
}

fn parse_form(body: &[u8]) -> Result<std::collections::BTreeMap<String, String>, String> {
    let text = std::str::from_utf8(body).map_err(|_| "form body is not UTF-8".to_string())?;
    let mut fields = std::collections::BTreeMap::new();
    for pair in text.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key)?;
        let value = percent_decode(value)?;
        // A repeated key (a multiple `select`) is joined with a comma.
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

/// Where leads are kept.
#[async_trait::async_trait]
pub trait LeadStore: Send + Sync + std::fmt::Debug {
    async fn store(&self, lead: &Lead, at_unix_ms: i64) -> Result<(), ApiError>;
}

/// Append-only JSONL lead store, optionally forwarding each lead to a mail
/// webhook (the ApexMail hook).
pub struct FileLeadStore {
    path: PathBuf,
    webhook: Option<String>,
    transport: Option<Arc<dyn Transport>>,
    lock: Mutex<()>,
}

impl std::fmt::Debug for FileLeadStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileLeadStore")
            .field("path", &self.path)
            .field("has_webhook", &self.webhook.is_some())
            .finish()
    }
}

impl FileLeadStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            webhook: None,
            transport: None,
            lock: Mutex::new(()),
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

    /// Best-effort forward: a webhook failure never loses the stored lead.
    async fn forward(&self, line: &str, at_unix_ms: i64) {
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
        let _ = at_unix_ms;
    }
}

#[async_trait::async_trait]
impl LeadStore for FileLeadStore {
    async fn store(&self, lead: &Lead, at_unix_ms: i64) -> Result<(), ApiError> {
        validate_lead(lead).map_err(|_| ApiError::BadRequest("invalid_lead"))?;
        let record = serde_json::json!({
            "atUnixMs": at_unix_ms,
            "lead": lead,
        });
        let mut line = serde_json::to_string(&record)
            .map_err(|error| ApiError::Internal(format!("lead serialization: {error}")))?;
        line.push('\n');
        let _guard = self
            .lock
            .lock()
            .map_err(|_| ApiError::Internal("lead lock poisoned".into()))?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ApiError::Internal(format!("lead directory: {error}")))?;
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| ApiError::Internal(format!("lead file: {error}")))?;
        file.write_all(line.as_bytes())
            .map_err(|error| ApiError::Internal(format!("lead write: {error}")))?;
        drop(_guard);
        self.forward(&line, at_unix_ms).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Mutex;

    use apex_connectors::transport::{HttpRequest, HttpResponse, Transport, TransportError};

    use super::*;

    fn lead() -> Lead {
        Lead {
            publisher: "pub1".into(),
            email: "founder@studio.example".into(),
            platforms: vec!["applovin_max".into()],
            monthly_revenue_range: Some("10k-50k".into()),
            notes: None,
        }
    }

    #[test]
    fn validate_accepts_a_well_formed_lead() {
        assert!(validate_lead(&lead()).is_ok());
    }

    #[test]
    fn parse_lead_reads_json_and_urlencoded_forms() {
        let json = br#"{"publisher":"pub1","email":"a@studio.example","platforms":["admob"],"notes":"hi"}"#;
        let parsed = parse_lead(json, Some("application/json")).expect("json");
        assert_eq!(parsed.publisher, "pub1");
        assert_eq!(parsed.platforms, vec!["admob".to_string()]);
        assert_eq!(parsed.notes.as_deref(), Some("hi"));

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
    fn validate_refuses_bad_publisher_email_and_platforms() {
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
        std::fs::create_dir_all(&dir).unwrap();
        let store = FileLeadStore::new(&dir); // path is a directory
        let err = store.store(&lead(), 1).await.expect_err("io error");
        assert!(matches!(err, ApiError::Internal(_)), "{err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_store_appends_jsonl_and_forwards_to_the_webhook() {
        let dir = std::env::temp_dir().join(format!("apex-leads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("leads.jsonl");
        let transport = Arc::new(FakeTransport::default());
        let store = FileLeadStore::new(&path)
            .with_mail_webhook("https://mail.example/hook", transport.clone());
        store
            .store(&lead(), 1_700_000_000_000)
            .await
            .expect("store");
        store
            .store(&lead(), 1_700_000_001_000)
            .await
            .expect("store");

        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(
            contents.lines().count(),
            2,
            "append-only, one line per lead"
        );
        assert!(contents.contains("\"publisher\":\"pub1\""));
        {
            let calls = transport.seen.lock().unwrap();
            assert_eq!(calls.len(), 2, "each lead is forwarded once");
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
}
