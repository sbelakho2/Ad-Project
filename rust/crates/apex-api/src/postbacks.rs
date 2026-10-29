//! MAX S2S impression postbacks (A3 D1). AppLovin does not retry, so the
//! handler validates, appends one durable line and answers immediately. The
//! raw payload is kept; the lift runner reduces it later.
//!
//! Two wire forms exist: the operator JSON batch (`POST /v1/postbacks/max`)
//! and the impression-level S2S callback (`GET /v1/postbacks/max/{publisher}`,
//! E1) where AppLovin replaces its macros in the query string. The GET form
//! authenticates with a per-publisher event key — AppLovin sends
//! `{EVENT_TOKEN}` as the SHA-1 digest of the event id and the key, never the
//! key itself — deduplicates on the event id and spools one line per
//! impression into a per-publisher file.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::Deserialize;
use sha1::{Digest, Sha1};

/// The environment variable naming the per-publisher MAX S2S event keys, as
/// comma-separated `publisher=event-key` entries (the `APEX_LEDGER_KEYS`
/// shape; an AppLovin event key is an opaque secret, not a hex key).
pub const MAX_EVENT_KEYS_ENV: &str = "APEX_MAX_EVENT_KEYS";
/// The environment variable naming the directory the per-publisher S2S GET
/// spool files are appended into.
pub const MAX_S2S_SPOOL_DIR_ENV: &str = "APEX_POSTBACK_SPOOL_DIR";
/// The spool directory used when [`MAX_S2S_SPOOL_DIR_ENV`] is unset.
pub const DEFAULT_SPOOL_DIR: &str = ".apex/postbacks";
/// How many event ids the process-local dedupe of `GET /v1/postbacks/max`
/// keeps. Bounded so a hostile or broken caller cannot grow the server's
/// memory without limit; AppLovin does not retry, so this guards against
/// duplicate deliveries, not against a durable replay log.
pub const MAX_S2S_DEDUPE_CAPACITY: usize = 65_536;

/// The spool file of one publisher inside the spool directory. Publisher
/// names are validated (`^[A-Za-z0-9_-]{1,64}$`) before they ever reach
/// this path.
pub fn max_s2s_spool_path(dir: &Path, publisher: &str) -> PathBuf {
    dir.join(format!("{publisher}.max.jsonl"))
}

/// Parse the per-publisher event-key map. A malformed entry is a typed
/// configuration error naming [`MAX_EVENT_KEYS_ENV`], never a silently
/// dropped publisher.
pub fn parse_event_keys(raw: &str) -> Result<HashMap<String, String>, String> {
    let mut keys = HashMap::new();
    for entry in raw.split(',').filter(|entry| !entry.trim().is_empty()) {
        let (publisher, key) = entry.split_once('=').ok_or_else(|| {
            format!("{MAX_EVENT_KEYS_ENV} entry {entry:?} is not publisher=event-key")
        })?;
        let publisher = publisher.trim();
        if !crate::sources::is_valid_publisher(publisher) {
            return Err(format!(
                "{MAX_EVENT_KEYS_ENV} publisher {publisher:?} must match \
                 ^[A-Za-z0-9_-]{{1,64}}$"
            ));
        }
        let key = key.trim();
        if key.is_empty() {
            return Err(format!(
                "{MAX_EVENT_KEYS_ENV} event key for {publisher:?} is empty"
            ));
        }
        if keys
            .insert(publisher.to_string(), key.to_string())
            .is_some()
        {
            return Err(format!(
                "{MAX_EVENT_KEYS_ENV} lists publisher {publisher:?} twice"
            ));
        }
    }
    Ok(keys)
}

/// The S2S event token AppLovin sends for one impression: the lowercase hex
/// SHA-1 of the event id concatenated with the publisher's event key (E1).
/// The raw key never travels the wire, so the receiver verifies this digest,
/// never the key itself.
pub fn event_token_digest(event_id: &str, event_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(event_id.as_bytes());
    hasher.update(event_key.as_bytes());
    hex::encode(hasher.finalize())
}

/// One impression-level S2S postback, with AppLovin's macro names accepted
/// as alias query keys next to the operator's configured spellings.
#[derive(Debug, Deserialize)]
pub struct MaxS2sPostback {
    #[serde(alias = "EVENT_TOKEN")]
    event_token: String,
    #[serde(alias = "EVENT_ID")]
    event_id: String,
    #[serde(alias = "USER_ID")]
    user_id: String,
    #[serde(alias = "AD_UNIT_ID")]
    ad_unit_id: String,
    #[serde(default, alias = "AD_UNIT_TEST_NAME")]
    ad_unit_test_name: Option<String>,
    revenue: f64,
}

/// The largest accepted length of one query-string field.
const MAX_S2S_FIELD_BYTES: usize = 256;

impl MaxS2sPostback {
    /// The impression's event id (the dedupe key).
    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    /// The event key the request presented (the `{EVENT_TOKEN}` macro).
    pub fn event_token(&self) -> &str {
        &self.event_token
    }
    /// Refuse an empty or oversized field set and a revenue that is not a
    /// finite non-negative number. AppLovin sends its macros replaced; an
    /// unreplaced `{MACRO}` or an empty value must never reach the spool.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("event_token", &self.event_token),
            ("event_id", &self.event_id),
            ("user_id", &self.user_id),
            ("ad_unit_id", &self.ad_unit_id),
        ] {
            let value = value.trim();
            if value.is_empty() {
                return Err(format!("postback needs a non-empty {name}"));
            }
            if value.len() > MAX_S2S_FIELD_BYTES {
                return Err(format!(
                    "postback {name} exceeds {MAX_S2S_FIELD_BYTES} bytes"
                ));
            }
        }
        if let Some(test_name) = self.ad_unit_test_name.as_deref().map(str::trim) {
            if test_name.len() > MAX_S2S_FIELD_BYTES {
                return Err(format!(
                    "postback ad_unit_test_name exceeds {MAX_S2S_FIELD_BYTES} bytes"
                ));
            }
        }
        if !self.revenue.is_finite() || self.revenue < 0.0 {
            return Err("postback needs a finite non-negative revenue".into());
        }
        Ok(())
    }
}

/// The bounded, process-local set of already-spoolled event ids behind
/// `GET /v1/postbacks/max`. Insertion-ordered: at capacity the oldest id is
/// evicted first so memory stays bounded.
#[derive(Debug)]
pub struct PostbackDedupe {
    inner: Mutex<DedupeInner>,
    capacity: usize,
}

#[derive(Debug, Default)]
struct DedupeInner {
    seen: HashSet<String>,
    order: VecDeque<String>,
}

impl PostbackDedupe {
    /// A dedupe with an explicit id bound.
    pub fn bounded(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(DedupeInner::default()),
            capacity: capacity.max(1),
        }
    }

    fn lock(&self) -> MutexGuard<'_, DedupeInner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Claim `event_id`. Returns `false` (and records nothing) when the id
    /// is already claimed, so a duplicate delivery is not spooled twice.
    pub fn claim(&self, event_id: &str) -> bool {
        let mut inner = self.lock();
        if !inner.seen.insert(event_id.to_string()) {
            return false;
        }
        inner.order.push_back(event_id.to_string());
        while inner.order.len() > self.capacity {
            match inner.order.pop_front() {
                Some(oldest) => {
                    inner.seen.remove(&oldest);
                }
                None => break,
            }
        }
        true
    }

    /// Give a claim back after its spool write failed, so the provider's
    /// retry can record the event instead of being answered as a duplicate
    /// of an impression that never reached the file.
    pub fn release(&self, event_id: &str) {
        let mut inner = self.lock();
        inner.seen.remove(event_id);
        inner.order.retain(|id| id != event_id);
    }

    /// Number of claimed ids (tests).
    pub fn len(&self) -> usize {
        self.lock().seen.len()
    }

    /// True when no id is claimed (tests).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Deserialize)]
struct Postback {
    #[serde(alias = "userId", alias = "user_id")]
    user_id: String,
    #[serde(alias = "adUnitId", alias = "ad_unit", alias = "ad_unit_id")]
    ad_unit: String,
    revenue: f64,
    #[serde(default, alias = "testName", alias = "adUnitTestName")]
    test_name: Option<String>,
    #[serde(default)]
    time: Option<String>,
}

/// Append one postback to the spool. The path is deployment configuration.
pub fn append(path: &Path, body: &[u8]) -> Result<usize, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|error| format!("postback JSON: {error}"))?;
    let rows = match value {
        serde_json::Value::Array(rows) => rows,
        value => vec![value],
    };
    if rows.is_empty() || rows.len() > 500 {
        return Err("postback batch must contain 1..=500 rows".into());
    }
    let mut lines = String::new();
    for row in rows {
        let postback: Postback =
            serde_json::from_value(row).map_err(|error| format!("postback shape: {error}"))?;
        if postback.user_id.is_empty()
            || postback.ad_unit.is_empty()
            || !postback.revenue.is_finite()
        {
            return Err("postback needs user id, ad unit and finite revenue".into());
        }
        let line = serde_json::json!({
            "user_id": postback.user_id,
            "ad_unit": postback.ad_unit,
            "revenue_micros": (postback.revenue * 1_000_000.0).round() as i64,
            "test_name": postback.test_name,
            "time": postback.time,
        });
        lines.push_str(&line.to_string());
        lines.push('\n');
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("postback dir: {error}"))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("postback spool: {error}"))?;
    file.write_all(lines.as_bytes())
        .map_err(|error| format!("postback write: {error}"))?;
    Ok(lines.lines().count())
}

/// Append one impression-level S2S postback to `publisher`'s own spool file
/// inside `dir` (append-only, one JSON line, a single write so concurrent
/// deliveries cannot interleave). The event key is never spooled.
pub fn append_max_s2s(
    dir: &Path,
    publisher: &str,
    postback: &MaxS2sPostback,
    received_at_unix_ms: i64,
) -> Result<(), String> {
    let line = serde_json::json!({
        "event_id": postback.event_id,
        "user_id": postback.user_id,
        "ad_unit": postback.ad_unit_id,
        "ad_unit_test_name": postback.ad_unit_test_name,
        "revenue_micros": (postback.revenue * 1_000_000.0).round() as i64,
        "received_at_unix_ms": received_at_unix_ms,
    })
    .to_string();
    let path = max_s2s_spool_path(dir, publisher);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("postback dir: {error}"))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("postback spool: {error}"))?;
    let mut record = String::with_capacity(line.len() + 1);
    record.push_str(&line);
    record.push('\n');
    file.write_all(record.as_bytes())
        .map_err(|error| format!("postback write: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_one_line_per_impression() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("max.jsonl");
        let body = br#"[{"userId":"u1","adUnitId":"rw","revenue":0.25,"testName":"a"},{"userId":"u1","adUnitId":"rw","revenue":0.5}]"#;
        assert_eq!(append(&path, body).unwrap(), 2);
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("\"revenue_micros\":250000"));
    }

    #[test]
    fn refuses_a_bad_shape() {
        let dir = tempfile::tempdir().unwrap();
        assert!(append(&dir.path().join("x"), br#"{"user_id":""}"#).is_err());
    }

    fn s2s(event_id: &str, token: &str, revenue: f64) -> MaxS2sPostback {
        serde_json::from_value(serde_json::json!({
            "event_token": token,
            "event_id": event_id,
            "user_id": "u1",
            "ad_unit_id": "au1",
            "revenue": revenue,
        }))
        .unwrap()
    }

    #[test]
    fn the_event_token_digest_is_sha1_of_event_id_then_key() {
        // Known SHA-1 vectors pin the concatenation order (event id first)
        // and the lowercase hex encoding.
        assert_eq!(
            event_token_digest("", ""),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            event_token_digest("abc", ""),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        // Order matters: id+key and key+id digest differently.
        assert_ne!(
            event_token_digest("ev-1", "key-1"),
            event_token_digest("key-1", "ev-1")
        );
        assert_eq!(
            event_token_digest("ev-1", "key-1"),
            event_token_digest("ev-1", "key-1")
        );
    }

    #[test]
    fn event_key_maps_are_parsed_strictly() {
        let keys = parse_event_keys("northstar=key-1, second=key-2,").unwrap();
        assert_eq!(keys.get("northstar").map(String::as_str), Some("key-1"));
        assert_eq!(keys.get("second").map(String::as_str), Some("key-2"));
        assert!(parse_event_keys("no-equals").is_err());
        assert!(parse_event_keys("../evil=key").is_err());
        assert!(parse_event_keys("northstar=a,northstar=b")
            .unwrap_err()
            .contains("twice"));
        assert!(parse_event_keys("northstar=")
            .unwrap_err()
            .contains("empty"));
        assert!(parse_event_keys("   ").unwrap().is_empty());
    }

    #[test]
    fn s2s_postbacks_validate_fields_and_revenue() {
        assert!(s2s("ev-1", "key", 0.25).validate().is_ok());
        assert!(s2s("", "key", 0.25).validate().is_err());
        assert!(s2s("ev-1", "", 0.25).validate().is_err());
        assert!(s2s("ev-1", "key", -0.25).validate().is_err());
        // NaN and infinity cannot even travel in JSON, but the struct is
        // built directly here to pin the guard.
        for revenue in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let postback = MaxS2sPostback {
                event_token: "key".into(),
                event_id: "ev-1".into(),
                user_id: "u1".into(),
                ad_unit_id: "au1".into(),
                ad_unit_test_name: None,
                revenue,
            };
            assert!(postback.validate().is_err());
        }
        let long = "x".repeat(257);
        let oversized = MaxS2sPostback {
            event_token: "key".into(),
            event_id: "ev-1".into(),
            user_id: long.clone(),
            ad_unit_id: "au1".into(),
            ad_unit_test_name: None,
            revenue: 0.25,
        };
        assert!(oversized.validate().is_err());
        // The macro names are accepted as alias keys.
        let macro_form: MaxS2sPostback = serde_json::from_value(serde_json::json!({
            "EVENT_TOKEN": "key",
            "EVENT_ID": "ev-2",
            "USER_ID": "u1",
            "AD_UNIT_ID": "au1",
            "AD_UNIT_TEST_NAME": "holdout-a",
            "revenue": 1.5,
        }))
        .unwrap();
        assert!(macro_form.validate().is_ok());
    }

    #[test]
    fn s2s_postbacks_spool_per_publisher_and_dedupe_on_the_event_id() {
        let dir = tempfile::tempdir().unwrap();
        let dedupe = PostbackDedupe::bounded(MAX_S2S_DEDUPE_CAPACITY);
        let postback = s2s("ev-1", "key", 0.25);
        assert!(dedupe.claim(postback.event_id()), "first delivery is new");
        append_max_s2s(dir.path(), "northstar", &postback, 1_792_055_400_000).unwrap();
        append_max_s2s(dir.path(), "second", &postback, 1_792_055_400_000).unwrap();
        assert!(!dedupe.claim(postback.event_id()), "the replay is claimed");
        let second = s2s("ev-2", "key", 1.0);
        assert!(dedupe.claim(second.event_id()));
        append_max_s2s(dir.path(), "northstar", &second, 1_792_055_400_001).unwrap();

        let text = std::fs::read_to_string(max_s2s_spool_path(dir.path(), "northstar")).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "the duplicate was never spooled: {text}");
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["event_id"], "ev-1");
        assert_eq!(first["user_id"], "u1");
        assert_eq!(first["ad_unit"], "au1");
        assert_eq!(first["revenue_micros"], 250_000);
        assert_eq!(first["received_at_unix_ms"], 1_792_055_400_000i64);
        // Each publisher's file only ever carries its own impressions.
        let other = std::fs::read_to_string(max_s2s_spool_path(dir.path(), "second")).unwrap();
        assert_eq!(other.lines().count(), 1);
    }

    #[test]
    fn dedupe_claims_evict_the_oldest_id_and_releases_are_re_claimable() {
        let dedupe = PostbackDedupe::bounded(2);
        assert!(dedupe.claim("a"));
        assert!(dedupe.claim("b"));
        assert!(
            dedupe.claim("c"),
            "the claim succeeds and evicts the oldest"
        );
        assert_eq!(dedupe.len(), 2);
        assert!(dedupe.claim("a"), "the evicted id may arrive again");
        assert!(!dedupe.claim("c"), "a still-claimed id stays a duplicate");
        dedupe.release("c");
        assert!(dedupe.claim("c"), "a released id can be re-claimed");
        assert!(dedupe.claim("d"));
        assert_eq!(dedupe.len(), 2);
    }
}
