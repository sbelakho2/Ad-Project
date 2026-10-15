//! Operator events into the analytics store.
//!
//! Every action attempt and every session lifecycle event is emitted as an
//! `operator_action` domain event through [`apex_analytics::AnalyticsClient`]
//! when an analytics store is configured. Restricted values are redacted
//! before emission ([`crate::redact`]) and the actor and target identities
//! stay bounded strings; the envelope (schema version, environment, service
//! version, occurred/received times) is the section 05 contract and the
//! client stamps it. The environment is the deployment's configured chip
//! (`APEX_ENVIRONMENT`), mapped to the analytics vocabulary by
//! [`analytics_environment`], never a hardcoded value.
//!
//! The bounded `apex_operator_actions_total` series live in
//! [`crate::telemetry`]: the action engine records every outcome through the
//! shared [`apex_metrics::Registry`] with `capability_class`, `action_class`
//! and `outcome` labels, so the actor and target identities here never
//! become metric dimensions.

use std::sync::Arc;

use apex_analytics::{
    AnalyticsClient, ClickHouseConfig, ClickHouseStore, DomainEvent, EventSource, EventType,
};

use crate::redact::redact_text;

/// The event emitter bound to the analytics store, when one is configured.
#[derive(Debug, Clone)]
pub struct EventEmitter {
    client: AnalyticsClient,
}

/// Map the deployment chip from [`crate::config`] (`PRODUCTION`, `TEST`,
/// `STAGING`, or a bounded deployment label) to the analytics environment
/// vocabulary `apex_metrics::vocab::Environment::VALUES` (`production`,
/// `staging`, `dev`).
///
/// - `PRODUCTION` maps to `production` and `STAGING` to `staging`;
/// - `TEST` maps to `dev`: the vocabulary has no `test` value, and a test
///   deployment is the development boundary, not the pre-production
///   staging mirror (the runner's non-production events are stamped `dev`
///   the same way);
/// - any other bounded label maps to `dev` too: an unrecognized label must
///   never stamp events as `production`, so a mislabelled deployment
///   cannot contaminate the production analytics partition.
pub fn analytics_environment(chip: &str) -> &'static str {
    use apex_metrics::vocab::Environment;
    match chip.trim().to_ascii_lowercase().as_str() {
        "production" => Environment::Production.as_str(),
        "staging" => Environment::Staging.as_str(),
        _ => Environment::Dev.as_str(),
    }
}

impl EventEmitter {
    /// Build an emitter over the configured analytics store, stamping the
    /// deployment's configured environment (mapped through
    /// [`analytics_environment`]). `None` when no store is configured; the
    /// journal remains the durable record.
    pub fn build(
        config: Option<&ClickHouseConfig>,
        environment: &str,
        service_version: &str,
    ) -> Result<Option<Self>, String> {
        let Some(config) = config else {
            return Ok(None);
        };
        let store = ClickHouseStore::new(config.clone()).map_err(|error| error.to_string())?;
        let client = AnalyticsClient::new(
            Arc::new(store),
            analytics_environment(environment),
            service_version,
        );
        Ok(Some(Self { client }))
    }

    /// Emit one operator action event. Redaction runs here, immediately
    /// before the payload is built, so no caller can bypass it.
    pub async fn emit_action(
        &self,
        actor: &str,
        capability: &str,
        target: &str,
        outcome: &str,
        correlation_id: &str,
    ) {
        let payload = apex_analytics::EventPayload {
            action_actor: Some(bounded(actor, 64)),
            authorization: Some(bounded(capability, 64)),
            target: Some(redact_text(target)),
            result: Some(outcome.to_string()),
            ..apex_analytics::EventPayload::new()
        };
        self.emit(payload, outcome, correlation_id).await;
    }

    /// Emit one session lifecycle event (sign-in, step-up, revoke, logout).
    /// The session handle itself never appears; the target is the operator
    /// identity or a session prefix.
    pub async fn emit_session(&self, actor: &str, event: &str, correlation_id: &str) {
        let payload = apex_analytics::EventPayload {
            action_actor: Some(bounded(actor, 64)),
            authorization: Some("session".to_string()),
            target: Some(format!("session:{event}")),
            result: Some(event.to_string()),
            ..apex_analytics::EventPayload::new()
        };
        self.emit(payload, event, correlation_id).await;
    }

    async fn emit(
        &self,
        payload: apex_analytics::EventPayload,
        outcome: &str,
        correlation_id: &str,
    ) {
        // `operator_action` events require the envelope `outcome` field as
        // well as the payload result; both carry the same bounded label.
        let event: Result<DomainEvent, apex_analytics::ValidationError> = self
            .client
            .builder(EventType::OperatorAction, EventSource::Operator)
            .with_event_id(format!("control-{correlation_id}"))
            .with_correlation_id(bounded(correlation_id, 64))
            .with_outcome(bounded(outcome, 64))
            .with_payload(payload)
            .build();
        match event {
            Ok(event) => {
                if let Err(error) = self.client.emit(&event).await {
                    // The journal stays the durable record; a failed emission
                    // is logged with its bounded reason and never blocks the
                    // operator action.
                    tracing::warn!(%error, "operator action event emission failed");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "operator action event was refused by validation");
            }
        }
    }
}

/// Truncate a bounded label to `max` characters.
fn bounded(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clickhouse_config() -> ClickHouseConfig {
        ClickHouseConfig {
            http_url: "http://127.0.0.1:8123".to_string(),
            database: "apex_test".to_string(),
            request_timeout_ms: 1,
        }
    }

    #[test]
    fn emitter_is_absent_without_a_store_configuration() {
        assert!(EventEmitter::build(None, "PRODUCTION", "test")
            .expect("none")
            .is_none());
        let config = clickhouse_config();
        assert!(EventEmitter::build(Some(&config), "PRODUCTION", "test")
            .expect("builds")
            .is_some());
    }

    #[test]
    fn the_chip_maps_to_the_analytics_environment_vocabulary() {
        for (chip, expected) in [
            ("PRODUCTION", "production"),
            ("production", "production"),
            ("TEST", "dev"),
            ("test", "dev"),
            ("STAGING", "staging"),
            (" staging ", "staging"),
            ("CANARY", "dev"),
        ] {
            let mapped = analytics_environment(chip);
            assert_eq!(mapped, expected, "{chip}");
            assert!(
                apex_metrics::vocab::Environment::VALUES.contains(&mapped),
                "{chip} maps to a declared vocabulary value"
            );
        }
    }

    #[tokio::test]
    async fn an_emitted_event_carries_the_configured_environment() {
        use apex_analytics::{EventStore, MemoryStore, ProjectionStore};
        let store = Arc::new(MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let emitter = EventEmitter {
            client: AnalyticsClient::new(store.clone(), analytics_environment("TEST"), "test"),
        };
        emitter
            .emit_action(
                "maya",
                "retry_read",
                "acct_northstar_max_01",
                "requested",
                "corr-1",
            )
            .await;
        let events = store.load_events().await.expect("load");
        assert_eq!(events.len(), 1, "the event validates and is stored");
        assert_eq!(events[0].environment, "dev", "the TEST chip maps to dev");
        assert_eq!(events[0].service_version, "test");
        assert_eq!(events[0].outcome.as_deref(), Some("requested"));
        assert_eq!(events[0].payload.result.as_deref(), Some("requested"));
    }

    #[tokio::test]
    async fn production_shaped_action_ids_are_never_refused_as_tokens() {
        use apex_analytics::{EventStore, MemoryStore, ProjectionStore};
        let store = Arc::new(MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let emitter = EventEmitter {
            client: AnalyticsClient::new(store.clone(), "production", "test"),
        };
        // The longest capability name previously produced a 40+ character
        // non-hex word that the envelope's token-like guard refused, dropping
        // every operator action event.
        for capability in crate::capabilities::Capability::ALL {
            let correlation = crate::actions::correlation_id();
            assert!(
                correlation.len() < 40,
                "{correlation:?} stays below the token-like word length"
            );
            emitter
                .emit_action(
                    "maya",
                    capability.as_str(),
                    "acct_1",
                    "requested",
                    &correlation,
                )
                .await;
        }
        let events = store.load_events().await.expect("load");
        assert_eq!(
            events.len(),
            crate::capabilities::Capability::ALL.len(),
            "every production-shaped action id emits a stored event"
        );
        for event in &events {
            assert_eq!(event.event_type, EventType::OperatorAction);
            assert!(event.event_id.starts_with("control-ctl-"));
        }
    }

    #[test]
    fn the_emitter_stamps_the_configured_environment_not_a_constant() {
        let config = clickhouse_config();
        let emitter = EventEmitter::build(Some(&config), "PRODUCTION", "test")
            .expect("builds")
            .expect("configured");
        assert_eq!(emitter.client.environment(), "production");
        let emitter = EventEmitter::build(Some(&config), "TEST", "test")
            .expect("builds")
            .expect("configured");
        assert_eq!(emitter.client.environment(), "dev");
        let emitter = EventEmitter::build(Some(&config), "staging", "test")
            .expect("builds")
            .expect("configured");
        assert_eq!(emitter.client.environment(), "staging");
    }

    #[test]
    fn bounded_truncates_without_splitting_characters() {
        assert_eq!(bounded("abcdef", 3), "abc");
        assert_eq!(bounded("ab", 3), "ab");
        assert_eq!(bounded("aébc", 2), "aé");
    }
}
