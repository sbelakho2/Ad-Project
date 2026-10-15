//! Live read-only audits: platform management reads with the publisher's own
//! key.
//!
//! This is the free-audit path of the product. A publisher onboards read-only
//! platform credentials once; [`LiveAuditor::audit`] then builds the snapshot
//! through `apex-connectors`, runs the audit engine and returns the report.
//! Nothing here writes to a platform.
//!
//! Credentials are stored encrypted with the crate vault. [`VaultedFileAccountStore`]
//! reads `<root>/<publisher>.blob`, decrypts it and parses [`LiveAccount`].
//! Missing files are a `404`; undecryptable or malformed files are logged
//! internal errors, never client-visible detail.
//!
//! Failure mapping: a credential or platform-validation refusal is
//! `400 invalid_credentials` (the publisher can act on it), every other
//! platform failure is `502 platform_unavailable` with the detail logged only.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;

use apex_audit::{AuditReport, BidderAccount};
use apex_connectors::admob::AdmobClient;
use apex_connectors::app_ads::AppAdsTxtFetcher;
use apex_connectors::levelplay::LevelPlayClient;
use apex_connectors::max::{MaxClient, MaxReportClient};
use apex_connectors::snapshot::{
    admob_snapshot, levelplay_snapshot, max_snapshot, PublisherContext,
};
use apex_connectors::transport::Transport;
use apex_core::{PlatformError, PlatformId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::clock::Clock;
use crate::error::ApiError;
use crate::vault::Vault;

/// Read cap for one encrypted credential bundle.
pub const MAX_CREDENTIAL_BYTES: u64 = 1024 * 1024;

/// One platform key set, as stored per publisher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveCredentials {
    /// MAX management key, LevelPlay key, or AdMob access token.
    pub api_key: String,
    /// MAX account id, LevelPlay app key, or `accounts/pub-<digits>`.
    pub account_id: String,
    /// The platform API base url, no trailing slash. Required so live calls
    /// never silently target a guessed host.
    pub base_url: String,
    /// MAX Revenue Reporting key (a separate credential from the management
    /// key). When present, the audit feeds 30-day country revenue into the
    /// floor rules instead of scoring with an empty reporting context.
    #[serde(default)]
    pub report_key: Option<String>,
    /// MAX reporting base url; defaults to the documented host when absent.
    #[serde(default)]
    pub reporting_base_url: Option<String>,
    /// LevelPlay refresh token (the secret key travels in `api_key`).
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// The publisher's onboarding record: context plus one key set per platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveAccount {
    /// The developer website hosting `app-ads.txt`.
    pub developer_site: String,
    /// Networks the publisher should authorize in app-ads.txt.
    #[serde(default)]
    pub expected_networks: Vec<String>,
    /// Ad unit ids to audit on platforms with a per-unit waterfall.
    #[serde(default)]
    pub ad_unit_ids: Vec<String>,
    /// Bidder accounts as apex knows them.
    #[serde(default)]
    pub bidder_accounts: Vec<BidderAccount>,
    /// Key sets keyed by platform wire name (`applovin_max`, `levelplay`,
    /// `admob`).
    pub platforms: BTreeMap<String, LiveCredentials>,
}

impl LiveAccount {
    /// The key set for `platform`, when onboarded.
    pub fn credentials(&self, platform: PlatformId) -> Option<&LiveCredentials> {
        self.platforms.get(platform.as_str())
    }
}

/// Where onboarded publisher accounts come from.
#[async_trait]
pub trait LiveAccountStore: Send + Sync + std::fmt::Debug {
    /// The account for `publisher`, or [`ApiError::NotFound`] when the
    /// publisher is not onboarded. Validation is re-checked here so the store
    /// is safe even when called directly.
    async fn account(&self, publisher: &str) -> Result<LiveAccount, ApiError>;
}

/// Vault-encrypted account files: `<root>/<publisher>.blob`.
pub struct VaultedFileAccountStore {
    root: PathBuf,
    vault: Vault,
}

impl std::fmt::Debug for VaultedFileAccountStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultedFileAccountStore")
            .field("root", &self.root)
            .finish()
    }
}

impl VaultedFileAccountStore {
    pub fn new(root: impl Into<PathBuf>, vault: Vault) -> Self {
        let root = root.into();
        // Canonicalize once; every read resolves inside this root or fails
        // closed, exactly like the snapshot and ledger sources.
        let root = crate::sources::canonical_root(&root);
        Self { root, vault }
    }

    /// Build a store whose vault key is 64 hex characters.
    pub fn with_vault_key_hex(
        root: impl Into<PathBuf>,
        key_hex: &str,
    ) -> Result<Self, crate::vault::VaultError> {
        Ok(Self::new(root, Vault::from_hex_key(key_hex)?))
    }

    fn path_for(&self, publisher: &str) -> PathBuf {
        self.root.join(format!("{publisher}.blob"))
    }
}

#[async_trait]
impl LiveAccountStore for VaultedFileAccountStore {
    async fn account(&self, publisher: &str) -> Result<LiveAccount, ApiError> {
        if !crate::sources::is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let root = self.root.clone();
        let path: PathBuf = self.path_for(publisher);
        let blob = tokio::task::spawn_blocking(move || -> Result<String, ApiError> {
            let resolved = crate::sources::contained_in(&root, path)?;
            let metadata = std::fs::metadata(&resolved).map_err(|error| match error.kind() {
                ErrorKind::NotFound => ApiError::NotFound,
                _ => ApiError::Internal(format!("credential file unreadable: {error}")),
            })?;
            crate::sources::ensure_within_cap(metadata.len(), MAX_CREDENTIAL_BYTES)?;
            std::fs::read_to_string(&resolved)
                .map_err(|error| ApiError::Internal(format!("credential file unreadable: {error}")))
        })
        .await
        .map_err(|join| ApiError::Internal(format!("credential read task failed: {join}")))??;
        let plaintext = self
            .vault
            .decrypt(&blob)
            .map_err(|error| ApiError::Internal(format!("credential decrypt failed: {error}")))?;
        serde_json::from_str(&plaintext)
            .map_err(|error| ApiError::Internal(format!("credential bundle malformed: {error}")))
    }
}

/// Runs live audits over an account store, transport and clock.
pub struct LiveAuditor {
    accounts: Arc<dyn LiveAccountStore>,
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    token: Option<Vec<u8>>,
}

impl std::fmt::Debug for LiveAuditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveAuditor").finish()
    }
}

impl LiveAuditor {
    pub fn new(
        accounts: Arc<dyn LiveAccountStore>,
        transport: Arc<dyn Transport>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            accounts,
            transport,
            clock,
            token: None,
        }
    }

    /// Require `Authorization: Bearer <token>` on every live audit. The
    /// binary enables this path only with a token; embedding services that
    /// front the API with their own auth may leave it unset.
    #[must_use]
    pub fn with_token(mut self, token: impl Into<Vec<u8>>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Check the bearer token. Fails closed: a configured-but-empty token
    /// refuses every request rather than opening the route.
    pub fn authorize(&self, authorization: Option<&str>) -> Result<(), ApiError> {
        let Some(expected) = &self.token else {
            return Ok(());
        };
        if expected.is_empty() {
            return Err(ApiError::Unauthorized);
        }
        let presented = authorization
            .and_then(|value| value.strip_prefix("Bearer "))
            .unwrap_or_default();
        if constant_time_eq(presented.as_bytes(), expected.as_slice()) {
            Ok(())
        } else {
            Err(ApiError::Unauthorized)
        }
    }

    /// Run the read-only audit for one publisher on one platform.
    pub async fn audit(
        &self,
        publisher: &str,
        platform: PlatformId,
    ) -> Result<AuditReport, ApiError> {
        Ok(self.audit_with_snapshot(publisher, platform).await?.1)
    }

    /// Run a live audit and return both the platform snapshot and the report,
    /// so the caller can persist the snapshot for the read path and the write
    /// cycle to consume.
    pub async fn audit_with_snapshot(
        &self,
        publisher: &str,
        platform: PlatformId,
    ) -> Result<(apex_audit::Snapshot, AuditReport), ApiError> {
        let account = self.accounts.account(publisher).await?;
        let credentials = account
            .credentials(platform)
            .ok_or(ApiError::BadRequest("platform_not_onboarded"))?;
        let mut context = PublisherContext {
            expected_networks: account.expected_networks.clone(),
            developer_site: Some(account.developer_site.clone()),
            bidder_accounts: account.bidder_accounts.clone(),
            ..PublisherContext::default()
        };
        let fetcher = AppAdsTxtFetcher::new(Arc::clone(&self.transport));
        let now_unix_ms = self.clock.now_unix_s().saturating_mul(1000);

        // With a MAX report key, the audit is scored against real 30-day
        // country revenue: a free audit that cannot see revenue cannot find a
        // stale floor's cost.
        if platform == PlatformId::AppLovinMax {
            if let Some(report_key) = credentials.report_key.as_deref() {
                let reporting_base = credentials
                    .reporting_base_url
                    .clone()
                    .unwrap_or_else(|| apex_connectors::max::MAX_REPORTING_BASE.to_string());
                let report =
                    MaxReportClient::new(report_key, &reporting_base, Arc::clone(&self.transport))?;
                let rows = report.country_revenue_30d(now_unix_ms).await?;
                context.country_revenue = rows
                    .into_iter()
                    .filter_map(|row| {
                        let country = row.country?;
                        let ad_unit_id = row.ad_unit_id?;
                        Some(apex_audit::CountryRevenueRow {
                            country,
                            ad_unit_id,
                            revenue_30d: row.revenue,
                            impressions_30d: row.impressions,
                        })
                    })
                    .collect();
            }
        }
        let snapshot = match platform {
            PlatformId::AppLovinMax => {
                let client = MaxClient::new(
                    &credentials.api_key,
                    &credentials.base_url,
                    Arc::clone(&self.transport),
                )?;
                max_snapshot(&client, &fetcher, &context, now_unix_ms).await?
            }
            PlatformId::LevelPlay => {
                let refresh_token =
                    credentials
                        .refresh_token
                        .as_deref()
                        .ok_or(ApiError::BadRequest(
                            "levelplay_credentials_need_refresh_token",
                        ))?;
                let client = LevelPlayClient::new(
                    &credentials.api_key,
                    refresh_token,
                    &credentials.account_id,
                    &credentials.base_url,
                    Arc::clone(&self.transport),
                )?;
                levelplay_snapshot(
                    &client,
                    &fetcher,
                    &context,
                    &account.ad_unit_ids,
                    now_unix_ms,
                )
                .await?
            }
            PlatformId::Admob => {
                let client = AdmobClient::new(
                    &credentials.api_key,
                    &credentials.account_id,
                    &credentials.base_url,
                    Arc::clone(&self.transport),
                )?;
                admob_snapshot(&client, &fetcher, &context, now_unix_ms).await?
            }
        };
        let report = apex_audit::run(&snapshot);
        Ok((snapshot, report))
    }
}

/// Length-checked byte comparison that does not short-circuit on the first
/// differing byte.
pub fn constant_time_eq(lhs: &[u8], rhs: &[u8]) -> bool {
    if lhs.len() != rhs.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in lhs.iter().zip(rhs.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Credential and validation refusals are the publisher's to resolve; everything
/// else is an upstream/platform condition. An auth failure on the app-ads.txt
/// fetch is the website refusing our request, not a platform key problem, so
/// it maps to the upstream condition even when a retry wrapper classified it
/// as an auth error by status.
pub fn map_platform_error(error: PlatformError) -> ApiError {
    match error {
        PlatformError::Validation(_) => ApiError::BadRequest("invalid_credentials"),
        PlatformError::Auth { ref operation, .. } if operation == "fetchAppAdsTxt" => {
            ApiError::PlatformUnavailable(error.to_string())
        }
        PlatformError::Auth { .. } => ApiError::BadRequest("invalid_credentials"),
        other => ApiError::PlatformUnavailable(other.to_string()),
    }
}

impl From<PlatformError> for ApiError {
    fn from(error: PlatformError) -> Self {
        map_platform_error(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{StaticAuditSource, StaticLedgerSource};
    use crate::state::AppState;
    use crate::vault::Vault;
    use apex_connectors::transport::{
        HttpRequest, HttpResponse, RecordingSleeper, RetryTransport, TransportError,
    };
    use std::sync::Mutex;
    use tower::ServiceExt;

    const VAULT_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    struct ScriptedTransport {
        script: Mutex<std::collections::VecDeque<Result<HttpResponse, TransportError>>>,
        calls: Mutex<Vec<HttpRequest>>,
    }

    impl ScriptedTransport {
        fn new(items: Vec<Result<HttpResponse, TransportError>>) -> Arc<Self> {
            Arc::new(Self {
                script: Mutex::new(items.into()),
                calls: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl Transport for ScriptedTransport {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.calls
                .lock()
                .map_err(|_| TransportError::Network("poisoned".into()))?
                .push(req);
            self.script
                .lock()
                .map_err(|_| TransportError::Network("poisoned".into()))?
                .pop_front()
                .unwrap_or(Err(TransportError::Network("script exhausted".into())))
        }
    }

    fn ok(status: u16, body: &str) -> Result<HttpResponse, TransportError> {
        Ok(HttpResponse {
            status,
            headers: vec![],
            body: body.as_bytes().to_vec(),
        })
    }

    #[derive(Debug, Default)]
    struct StaticAccountStore {
        account: Option<LiveAccount>,
    }

    #[async_trait]
    impl LiveAccountStore for StaticAccountStore {
        async fn account(&self, _publisher: &str) -> Result<LiveAccount, ApiError> {
            self.account.clone().ok_or(ApiError::NotFound)
        }
    }

    fn max_account(developer_site: &str) -> LiveAccount {
        let mut platforms = BTreeMap::new();
        platforms.insert(
            "applovin_max".to_string(),
            LiveCredentials {
                api_key: "key".to_string(),
                account_id: "acct".to_string(),
                base_url: "https://api.example.test".to_string(),
                report_key: None,
                reporting_base_url: None,
                refresh_token: None,
            },
        );
        LiveAccount {
            developer_site: developer_site.to_string(),
            expected_networks: vec!["google.com".to_string(), "unityads.com".to_string()],
            ad_unit_ids: vec![],
            bidder_accounts: vec![],
            platforms,
        }
    }

    /// A MAX account that also carries the Revenue Reporting key.
    fn max_account_with_report_key(developer_site: &str) -> LiveAccount {
        let mut account = max_account(developer_site);
        if let Some(credentials) = account.platforms.get_mut("applovin_max") {
            credentials.report_key = Some("report-key".to_string());
            credentials.reporting_base_url = Some("https://r.example.test".to_string());
        }
        account
    }

    fn auditor_with(
        account: Option<LiveAccount>,
        script: Vec<Result<HttpResponse, TransportError>>,
    ) -> (Arc<LiveAuditor>, Arc<ScriptedTransport>) {
        let transport = ScriptedTransport::new(script);
        // Mirror production wiring: classification and retries live in the
        // retry wrapper, exactly as the binary builds it.
        let retry: Arc<dyn Transport> = Arc::new(RetryTransport::new(
            transport.clone(),
            0,
            Arc::new(RecordingSleeper::default()),
        ));
        let auditor = Arc::new(LiveAuditor::new(
            Arc::new(StaticAccountStore { account }),
            retry,
            Arc::new(crate::clock::FixedClock::new(1_700_000_000)),
        ));
        (auditor, transport)
    }

    fn live_state(auditor: Option<Arc<LiveAuditor>>) -> AppState {
        let mut state = AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        );
        if let Some(auditor) = auditor {
            state = state.with_live_auditor(auditor);
        }
        state
    }

    async fn post_audit(
        state: AppState,
        platform: &str,
        body: &str,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let response = crate::router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/v1/audit/{platform}"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, value)
    }

    #[tokio::test]
    async fn live_audit_reports_missing_expected_network() {
        let (auditor, transport) = auditor_with(
            Some(max_account("studio.example.com")),
            vec![
                ok(
                    200,
                    r#"[{"id":"u1","name":"Banner","platform":"ios","package_name":"com.x","ad_format":"BANNER"}]"#,
                ),
                ok(200, "google.com, pub-1, DIRECT\n"),
            ],
        );
        let (status, report) = post_audit(
            live_state(Some(auditor)),
            "applovin_max",
            r#"{"publisher":"pub1"}"#,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{report}");
        let findings = report["findings"].as_array().expect("findings");
        assert!(
            findings.iter().any(|finding| {
                finding["rule"] == "app_ads_txt" && finding["platform"] == "applovin_max"
            }),
            "{report}"
        );
        let calls = transport.calls.lock().map(|c| c.len()).unwrap_or(0);
        assert_eq!(calls, 2);
    }

    #[tokio::test]
    async fn live_audit_fetches_country_revenue_when_a_report_key_is_set() {
        let (auditor, transport) = auditor_with(
            Some(max_account_with_report_key("studio.example.com")),
            // Reporting is pulled first (it fills the audit context), then the
            // management calls.
            vec![
                ok(
                    200,
                    r#"{"code":200,"count":1,"results":[{"day":"2026-10-01","country":"EE","max_ad_unit_id":"u1","estimated_revenue":"12.5","impressions":"100000"}]}"#,
                ),
                ok(
                    200,
                    r#"[{"id":"u1","name":"Banner","platform":"ios","package_name":"com.x","ad_format":"BANNER"}]"#,
                ),
                ok(
                    200,
                    "google.com, pub-1, DIRECT\nunityads.com, pub-2, DIRECT\n",
                ),
            ],
        );
        let (status, report) = post_audit(
            live_state(Some(auditor)),
            "applovin_max",
            r#"{"publisher":"pub1"}"#,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{report}");
        let calls = transport
            .calls
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default();
        assert_eq!(calls.len(), 3, "reporting + management + app-ads.txt");
        let report_call = calls
            .iter()
            .find(|call| call.url.contains("/maxReport?"))
            .expect("a MAX reporting call");
        assert!(report_call.url.contains("api_key=report-key"));
        assert!(report_call.url.contains("country"));
    }

    #[tokio::test]
    async fn live_audit_answers_503_without_an_auditor() {
        let (status, body) = post_audit(live_state(None), "admob", r#"{"publisher":"pub1"}"#).await;
        assert_eq!(status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "live_audit_not_configured");
    }

    #[tokio::test]
    async fn live_audit_maps_platform_auth_to_invalid_credentials() {
        let (auditor, _) = auditor_with(Some(max_account("studio.example.com")), vec![ok(401, "")]);
        let (status, body) = post_audit(
            live_state(Some(auditor)),
            "applovin_max",
            r#"{"publisher":"pub1"}"#,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_credentials");
    }

    #[tokio::test]
    async fn live_audit_404s_unknown_publishers_and_400s_missing_platforms() {
        let (auditor, _) = auditor_with(None, vec![]);
        let (status, body) = post_audit(
            live_state(Some(auditor)),
            "applovin_max",
            r#"{"publisher":"pub1"}"#,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "not_found");

        let (auditor, _) = auditor_with(Some(max_account("studio.example.com")), vec![]);
        let (status, body) = post_audit(
            live_state(Some(auditor)),
            "admob",
            r#"{"publisher":"pub1"}"#,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "platform_not_onboarded");

        let (auditor, _) = auditor_with(Some(max_account("studio.example.com")), vec![]);
        let (status, body) = post_audit(
            live_state(Some(auditor)),
            "applovin_max",
            r#"{"publisher":"a/b"}"#,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_publisher");
    }

    #[tokio::test]
    async fn vaulted_store_roundtrips_and_fails_closed() {
        let dir = std::env::temp_dir().join(format!("apex-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let vault = Vault::from_hex_key(VAULT_KEY).expect("vault");
        let store = VaultedFileAccountStore::new(dir.clone(), vault);
        let account = max_account("studio.example.com");
        let blob = Vault::from_hex_key(VAULT_KEY)
            .expect("vault")
            .encrypt(&serde_json::to_string(&account).expect("json"));
        std::fs::write(dir.join("pub1.blob"), blob).expect("write blob");

        let loaded = store.account("pub1").await.expect("account");
        assert_eq!(loaded.developer_site, "studio.example.com");
        assert!(loaded.credentials(PlatformId::AppLovinMax).is_some());

        assert!(matches!(
            store.account("missing").await,
            Err(ApiError::NotFound)
        ));
        assert!(matches!(
            store.account("../escape").await,
            Err(ApiError::BadRequest("invalid_publisher"))
        ));

        let other = VaultedFileAccountStore::new(
            dir.clone(),
            Vault::from_hex_key("1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100")
                .expect("vault"),
        );
        assert!(matches!(
            other.account("pub1").await,
            Err(ApiError::Internal(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn vaulted_store_refuses_symlink_escapes_and_oversized_blobs() {
        let dir = std::env::temp_dir().join(format!("apex-live-esc-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!("apex-live-out-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("root");
        std::fs::write(&outside, "not encrypted").expect("outside file");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, dir.join("pub1.blob")).expect("symlink");

        let vault = Vault::from_hex_key(VAULT_KEY).expect("vault");
        let store = VaultedFileAccountStore::new(dir.clone(), vault);
        #[cfg(unix)]
        assert!(matches!(
            store.account("pub1").await,
            Err(ApiError::NotFound)
        ));

        let big = dir.join("pub2.blob");
        std::fs::write(
            &big,
            vec![b'a'; usize::try_from(MAX_CREDENTIAL_BYTES).unwrap() + 1],
        )
        .expect("big blob");
        assert!(matches!(
            store.account("pub2").await,
            Err(ApiError::Internal(_))
        ));

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn constant_time_eq_is_length_checked_and_exact() {
        assert!(constant_time_eq(b"token-1", b"token-1"));
        assert!(!constant_time_eq(b"token-1", b"token-2"));
        assert!(!constant_time_eq(b"token-1", b"token-1-longer"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[tokio::test]
    async fn live_audit_enforces_the_bearer_token_when_configured() {
        let raw = ScriptedTransport::new(vec![
            ok(200, r#"[]"#),
            ok(
                200,
                "google.com, pub-1, DIRECT\nunityads.com, pub-2, DIRECT\n",
            ),
        ]);
        let retry: Arc<dyn Transport> = Arc::new(RetryTransport::new(
            raw,
            0,
            Arc::new(RecordingSleeper::default()),
        ));
        let auditor = Arc::new(
            LiveAuditor::new(
                Arc::new(StaticAccountStore {
                    account: Some(max_account("studio.example.com")),
                }),
                retry,
                Arc::new(crate::clock::FixedClock::new(1_700_000_000)),
            )
            .with_token("s3cret-token"),
        );
        let state = live_state(Some(auditor));

        let response = crate::router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/audit/applovin_max")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"publisher":"pub1"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);

        let wrong = crate::router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/audit/applovin_max")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer nope")
                    .body(axum::body::Body::from(r#"{"publisher":"pub1"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(wrong.status(), axum::http::StatusCode::UNAUTHORIZED);

        let allowed = crate::router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/audit/applovin_max")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer s3cret-token")
                    .body(axum::body::Body::from(r#"{"publisher":"pub1"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(allowed.status(), axum::http::StatusCode::OK);
    }

    #[test]
    fn an_empty_configured_token_fails_closed() {
        let auditor = LiveAuditor::new(
            Arc::new(StaticAccountStore::default()),
            ScriptedTransport::new(vec![]),
            Arc::new(crate::clock::FixedClock::new(0)),
        )
        .with_token("");
        assert!(matches!(
            auditor.authorize(Some("Bearer ")),
            Err(ApiError::Unauthorized)
        ));
        assert!(matches!(
            auditor.authorize(None),
            Err(ApiError::Unauthorized)
        ));
    }

    #[tokio::test]
    async fn levelplay_audit_reads_every_configured_unit() {
        let mut account = max_account("studio.example.com");
        account.ad_unit_ids = vec!["u1".to_string(), "u2".to_string()];
        account.platforms.clear();
        account.platforms.insert(
            "levelplay".to_string(),
            LiveCredentials {
                api_key: "key".to_string(),
                account_id: "app-key".to_string(),
                base_url: "https://api.example.test".to_string(),
                report_key: None,
                reporting_base_url: None,
                refresh_token: Some("refresh".to_string()),
            },
        );
        let (auditor, transport) = auditor_with(
            Some(account),
            vec![
                // The bearer is minted once, then cached: exactly one auth call.
                ok(200, r#""bearer-token""#),
                ok(
                    200,
                    r#"{"instances":[{"instanceId":"e1","adapterNetwork":"admob","instanceName":"ADM_1","price":1.0,"position":1}]}"#,
                ),
                ok(
                    200,
                    r#"{"instances":[{"instanceId":"e2","adapterNetwork":"meta","instanceName":"META_1","price":2.0,"position":1}]}"#,
                ),
                ok(
                    200,
                    "google.com, pub-1, DIRECT\nunityads.com, pub-2, DIRECT\n",
                ),
            ],
        );
        let (status, report) = post_audit(
            live_state(Some(auditor)),
            "levelplay",
            r#"{"publisher":"pub1"}"#,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{report}");
        let calls = transport
            .calls
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default();
        assert_eq!(calls.len(), 4, "auth + two units + app-ads.txt");
        assert!(calls[0].url.contains("/partners/publisher/auth"));
        assert!(calls[1].url.contains("/mediation/instances/v4"));
    }

    #[tokio::test]
    async fn admob_audit_runs_against_the_mediation_api() {
        let mut account = max_account("studio.example.com");
        account.platforms.clear();
        account.platforms.insert(
            "admob".to_string(),
            LiveCredentials {
                api_key: "token".to_string(),
                account_id: "accounts/pub-1234567890".to_string(),
                base_url: "https://api.example.test".to_string(),
                report_key: None,
                reporting_base_url: None,
                refresh_token: None,
            },
        );
        let (auditor, _) = auditor_with(
            Some(account),
            vec![
                ok(200, r#"{"mediationGroups":[]}"#),
                ok(
                    200,
                    "google.com, pub-1, DIRECT\nunityads.com, pub-2, DIRECT\n",
                ),
            ],
        );
        let (status, report) = post_audit(
            live_state(Some(auditor)),
            "admob",
            r#"{"publisher":"pub1"}"#,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{report}");
    }

    #[tokio::test]
    async fn website_refusals_are_platform_unavailable_not_credential_errors() {
        let (auditor, _) = auditor_with(
            Some(max_account("studio.example.com")),
            vec![ok(200, r#"[]"#), ok(403, "")],
        );
        let (status, body) = post_audit(
            live_state(Some(auditor)),
            "applovin_max",
            r#"{"publisher":"pub1"}"#,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"], "platform_unavailable");
    }
}
