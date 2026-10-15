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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// Where alerts go.
#[async_trait]
pub trait AlertSink: Send + Sync + std::fmt::Debug {
    async fn emit(&self, alert: Alert);
}

/// Drops alerts; a deployment that has not configured delivery yet.
#[derive(Debug, Default)]
pub struct NoopAlertSink;

#[async_trait]
impl AlertSink for NoopAlertSink {
    async fn emit(&self, _alert: Alert) {}
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
    async fn emit(&self, alert: Alert) {
        if let Ok(mut alerts) = self.alerts.lock() {
            alerts.push(alert);
        }
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
        .await;
        sink.emit(Alert::ExpiredToken {
            platform: "applovin_max".into(),
            detail: "401".into(),
        })
        .await;
        let alerts = sink.alerts();
        assert_eq!(alerts.len(), 2);
        assert!(matches!(alerts[0], Alert::GuardrailStop { .. }));
        assert!(matches!(alerts[1], Alert::ExpiredToken { .. }));
    }

    #[tokio::test]
    async fn the_noop_sink_swallows_alerts() {
        NoopAlertSink
            .emit(Alert::PullFailure {
                experiment_id: "e".into(),
                detail: "two days".into(),
            })
            .await;
    }
}
