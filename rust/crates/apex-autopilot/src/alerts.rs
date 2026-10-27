//! Customer-impact alerts.
//!
//! Four conditions reach a customer (they need to act): a rollback that failed
//! to revert, a guardrail that stopped a test, two days of missing reporting,
//! and an expired platform token. The runner emits one [`Alert`] for each; the
//! sink decides delivery (email, webhook, pager). The library ships a no-op
//! sink and a recording sink for tests; a deployment wires its own.

use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// One condition worth paging the customer about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Alert {
    /// A rollback was required but the platform call failed: the change is
    /// still live and the customer must act.
    FailedRollback {
        experiment_id: String,
        detail: String,
    },
    /// A guardrail (fast harm, platform incident, publisher edit, split
    /// mismatch, reporting gap) stopped the test.
    GuardrailStop {
        experiment_id: String,
        detail: String,
    },
    /// Two consecutive days without reporting: the test cannot be measured.
    PullFailure {
        experiment_id: String,
        detail: String,
    },
    /// A platform token expired: management or reporting calls are refused.
    ExpiredToken { platform: String, detail: String },
    /// An account metric fell more than the weekday-adjusted baseline allows.
    AccountMetricDrop {
        publisher: String,
        ad_unit: String,
        metric: String,
        observed: f64,
        baseline: f64,
    },
    /// app-ads.txt lines disappeared from a watched publisher's file.
    AppAdsLinesLost {
        publisher: String,
        domain: String,
        lost: u32,
    },
}

/// A3 M1: compare the latest account metric with the weekday-adjusted
/// 28-day baseline. A drop is reported when the observed value is below
/// `baseline * (1 - threshold)`.
#[must_use]
pub fn metric_dropped(baseline: f64, observed: f64, threshold: f64) -> bool {
    baseline.is_finite()
        && observed.is_finite()
        && baseline > 0.0
        && threshold.is_finite()
        && threshold > 0.0
        && observed < baseline * (1.0 - threshold)
}

/// A3 M1: count authority lines lost between two parsed app-ads.txt files.
/// This is a cardinality helper; the identity-aware check is
/// [`app_ads_line_identities_lost`].
#[must_use]
pub fn app_ads_lines_lost(previous: usize, current: usize) -> u32 {
    u32::try_from(previous.saturating_sub(current)).unwrap_or(u32::MAX)
}

/// A3 M1: count canonical line identities that disappeared. Replacing five
/// records with five unrelated records loses five, not zero.
#[must_use]
pub fn app_ads_line_identities_lost(previous: &[String], current: &[String]) -> u32 {
    let current: std::collections::BTreeSet<&str> = current.iter().map(String::as_str).collect();
    u32::try_from(
        previous
            .iter()
            .filter(|line| !current.contains(line.as_str()))
            .count(),
    )
    .unwrap_or(u32::MAX)
}

/// Why an alert could not be delivered. Delivery is fail-closed: a caller
/// that advances a baseline must treat `Err` as "not delivered" and retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertError(pub String);

impl std::fmt::Display for AlertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AlertError {}

/// Where alerts go.
#[async_trait]
pub trait AlertSink: Send + Sync + std::fmt::Debug {
    /// Deliver one alert. `Err` means the alert was not durably delivered and
    /// the caller must not treat it as handled.
    async fn emit(&self, alert: Alert) -> Result<(), AlertError>;
}

/// Drops alerts; a deployment that has not configured delivery yet.
#[derive(Debug, Default)]
pub struct NoopAlertSink;

#[async_trait]
impl AlertSink for NoopAlertSink {
    async fn emit(&self, _alert: Alert) -> Result<(), AlertError> {
        Ok(())
    }
}

/// Records alerts in memory (tests, and the API's recent-alerts view).
#[derive(Debug, Default)]
pub struct RecordingAlertSink {
    alerts: Mutex<Vec<Alert>>,
}

impl RecordingAlertSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// The alerts emitted so far, in order.
    pub fn alerts(&self) -> Vec<Alert> {
        self.alerts.lock().map(|a| a.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl AlertSink for RecordingAlertSink {
    async fn emit(&self, alert: Alert) -> Result<(), AlertError> {
        let mut alerts = self
            .alerts
            .lock()
            .map_err(|_| AlertError("recording sink lock poisoned".into()))?;
        alerts.push(alert);
        Ok(())
    }
}

/// Appends each alert as one JSON line, inspectable beside the ledger.
#[derive(Debug)]
pub struct FileAlertSink {
    path: std::path::PathBuf,
    lock: Mutex<()>,
}

impl FileAlertSink {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }
}

#[async_trait]
impl AlertSink for FileAlertSink {
    async fn emit(&self, alert: Alert) -> Result<(), AlertError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| AlertError("alert file lock poisoned".into()))?;
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    AlertError(format!(
                        "{}: parent directory: {error}",
                        self.path.display()
                    ))
                })?;
            }
        }
        let line = serde_json::to_string(&alert)
            .map_err(|error| AlertError(format!("alert could not be serialized: {error}")))?;
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| {
                AlertError(format!(
                    "{} could not be opened: {error}",
                    self.path.display()
                ))
            })?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .map_err(|error| {
                AlertError(format!(
                    "{} could not be written: {error}",
                    self.path.display()
                ))
            })?;
        file.sync_data().map_err(|error| {
            AlertError(format!(
                "{} could not be fsynced: {error}",
                self.path.display()
            ))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[tokio::test]
    async fn the_recording_sink_keeps_order() {
        let sink = RecordingAlertSink::new();
        sink.emit(Alert::GuardrailStop {
            experiment_id: "e1".into(),
            detail: "fast harm".into(),
        })
        .await
        .unwrap();
        sink.emit(Alert::ExpiredToken {
            platform: "applovin_max".into(),
            detail: "401".into(),
        })
        .await
        .unwrap();
        let alerts = sink.alerts();
        assert_eq!(alerts.len(), 2);
        assert!(matches!(alerts[0], Alert::GuardrailStop { .. }));
        assert!(matches!(alerts[1], Alert::ExpiredToken { .. }));
    }

    #[tokio::test]
    async fn the_file_sink_appends_one_json_line_per_alert() {
        let dir = std::env::temp_dir().join(format!("apex-alerts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("alerts.jsonl");
        let sink = FileAlertSink::new(&path);
        sink.emit(Alert::GuardrailStop {
            experiment_id: "e1".into(),
            detail: "fast harm".into(),
        })
        .await
        .unwrap();
        sink.emit(Alert::ExpiredToken {
            platform: "applovin_max".into(),
            detail: "401".into(),
        })
        .await
        .unwrap();
        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(contents.lines().count(), 2);
        assert!(contents.contains("guardrailStop"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_file_sink_fails_closed_when_it_cannot_write() {
        let dir = std::env::temp_dir().join(format!("apex-alerts-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The target is a directory, so the append open must fail.
        let sink = FileAlertSink::new(&dir);
        let error = sink
            .emit(Alert::GuardrailStop {
                experiment_id: "e1".into(),
                detail: "fast harm".into(),
            })
            .await
            .expect_err("an unwritable alert file is not accepted");
        assert!(!error.0.is_empty(), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_noop_sink_swallows_alerts() {
        NoopAlertSink
            .emit(Alert::PullFailure {
                experiment_id: "e".into(),
                detail: "two days".into(),
            })
            .await
            .unwrap();
    }
}
