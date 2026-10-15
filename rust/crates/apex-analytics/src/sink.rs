//! Event sinks: the client handle and the spooling wrapper.
//!
//! - [`AnalyticsClient`] is a cheap-clone handle carrying the environment and
//!   service version defaults; producers build events through it so no event
//!   ever lacks its deployment labels.
//! - [`SpoolSink`] wraps a primary sink with a JSONL spool. A failed batch is
//!   appended to the spool instead of being lost; [`SpoolSink::replay_spool`]
//!   re-emits spooled batches and truncates only after they are delivered.
//!   Corrupt spool lines are preserved to a `.dead` sibling file, never
//!   dropped silently. Rewrites (truncation and keeping undelivered lines)
//!   go through a `<spool>.tmp` sibling and a rename, so a crash mid-rewrite
//!   leaves the old complete file or the new complete file, never a
//!   truncated one.
//!
//! Delivery is at-least-once by construction: a crash between a successful
//! insert and the spool truncation replays the batch, and the store's
//! `(source, event_id)` dedup key absorbs the replay.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::envelope::{DomainEvent, DomainEventBuilder, EventSource, EventType};
use crate::store::{EventStore, StoreError};

/// Store failures convert into sink failures with their message intact.
impl From<crate::store::StoreError> for SinkError {
    fn from(error: crate::store::StoreError) -> Self {
        SinkError::Store(error.to_string())
    }
}

/// Sink failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SinkError {
    /// The store refused the batch.
    #[error("store refused the batch: {0}")]
    Store(String),
    /// The spool could not be written or read.
    #[error("spool operation failed at {path}: {reason}")]
    Spool {
        /// Spool file involved.
        path: String,
        /// Underlying reason.
        reason: String,
    },
    /// An event does not belong to this client's deployment labels.
    #[error("event {event_id} targets environment '{event_environment}', client is '{client_environment}'")]
    EnvironmentMismatch {
        /// The offending event.
        event_id: String,
        /// The event's environment label.
        event_environment: String,
        /// The client's environment label.
        client_environment: String,
    },
}

/// What an emit call did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EmitOutcome {
    /// Events delivered to the primary sink.
    pub delivered: usize,
    /// Events spooled for a later replay.
    pub spooled: usize,
    /// The primary sink's error message when the batch was spooled for it;
    /// the reason a spooled batch is not delivered yet is never silent.
    pub last_error: Option<String>,
}

/// A sink accepts validated events.
#[async_trait]
pub trait EventSink: Send + Sync + std::fmt::Debug {
    /// Emit a batch. Batches are atomic per call.
    async fn emit(&self, events: &[DomainEvent]) -> Result<EmitOutcome, SinkError>;
}

/// A cheap-clone handle that stamps events with the deployment labels and
/// writes them through the store.
#[derive(Debug, Clone)]
pub struct AnalyticsClient {
    store: Arc<dyn EventStore>,
    environment: String,
    service_version: String,
}

impl AnalyticsClient {
    /// A client writing into `store`.
    pub fn new(
        store: Arc<dyn EventStore>,
        environment: impl Into<String>,
        service_version: impl Into<String>,
    ) -> Self {
        Self {
            store,
            environment: environment.into(),
            service_version: service_version.into(),
        }
    }

    /// The environment label this client stamps.
    pub fn environment(&self) -> &str {
        &self.environment
    }

    /// The service version label this client stamps.
    pub fn service_version(&self) -> &str {
        &self.service_version
    }

    /// Create the database and tables when they are absent (idempotent).
    /// Boot calls this once so a fresh deployment starts writing instead of
    /// spooling against a missing schema.
    pub async fn ensure_schema(&self) -> Result<(), StoreError> {
        self.store.ensure_schema().await
    }

    /// A builder pre-filled with the client's labels.
    pub fn builder(&self, event_type: EventType, source: EventSource) -> DomainEventBuilder {
        DomainEvent::builder(event_type, source)
            .with_environment(self.environment.clone())
            .with_service_version(self.service_version.clone())
    }

    /// Emit one event.
    pub async fn emit(&self, event: &DomainEvent) -> Result<EmitOutcome, SinkError> {
        self.emit_batch(std::slice::from_ref(event)).await
    }

    /// Emit a batch; refuses events stamped for another environment.
    pub async fn emit_batch(&self, events: &[DomainEvent]) -> Result<EmitOutcome, SinkError> {
        for event in events {
            if event.environment != self.environment {
                return Err(SinkError::EnvironmentMismatch {
                    event_id: event.event_id.clone(),
                    event_environment: event.environment.clone(),
                    client_environment: self.environment.clone(),
                });
            }
        }
        self.store
            .insert(events)
            .await
            .map_err(|error| SinkError::Store(error.to_string()))?;
        Ok(EmitOutcome {
            delivered: events.len(),
            spooled: 0,
            last_error: None,
        })
    }
}

#[async_trait]
impl EventSink for AnalyticsClient {
    async fn emit(&self, events: &[DomainEvent]) -> Result<EmitOutcome, SinkError> {
        self.emit_batch(events).await
    }
}

/// Outcome of a spool replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReplayOutcome {
    /// Events re-delivered from the spool.
    pub replayed: usize,
    /// Corrupt lines preserved to the `.dead` file.
    pub corrupt: usize,
    /// Lines still spooled after the replay (a delivery failure stopped it).
    pub remaining: usize,
}

/// A sink that spools failed batches to a JSONL file and replays them later.
#[derive(Debug)]
pub struct SpoolSink<S: EventSink> {
    primary: S,
    spool_path: PathBuf,
}

impl<S: EventSink> SpoolSink<S> {
    /// Wrap `primary`, spooling failed batches to `spool_path`.
    pub fn new(primary: S, spool_path: impl Into<PathBuf>) -> Self {
        Self {
            primary,
            spool_path: spool_path.into(),
        }
    }

    /// The spool file path.
    pub fn spool_path(&self) -> &Path {
        &self.spool_path
    }

    /// The primary sink (for health checks and direct access).
    pub fn primary(&self) -> &S {
        &self.primary
    }

    fn dead_path(&self) -> PathBuf {
        let mut name = self
            .spool_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "spool.jsonl".to_string());
        name.push_str(".dead");
        self.spool_path.with_file_name(name)
    }

    async fn append_lines(&self, lines: &str) -> Result<(), SinkError> {
        if let Some(parent) = self.spool_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| SinkError::Spool {
                    path: parent.display().to_string(),
                    reason: error.to_string(),
                })?;
        }
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.spool_path)
            .await
            .map_err(|error| SinkError::Spool {
                path: self.spool_path.display().to_string(),
                reason: error.to_string(),
            })?;
        file.write_all(lines.as_bytes())
            .await
            .map_err(|error| SinkError::Spool {
                path: self.spool_path.display().to_string(),
                reason: error.to_string(),
            })
    }

    /// Re-emit every spooled batch, then truncate. Corrupt lines move to
    /// `<spool>.dead`. A failed delivery keeps the undelivered lines spooled
    /// and reports the error.
    pub async fn replay_spool(&self) -> Result<ReplayOutcome, SinkError> {
        let text = match tokio::fs::read_to_string(&self.spool_path).await {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ReplayOutcome::default())
            }
            Err(error) => {
                return Err(SinkError::Spool {
                    path: self.spool_path.display().to_string(),
                    reason: error.to_string(),
                })
            }
        };
        let mut deliverable_lines: Vec<&str> = Vec::new();
        let mut events: Vec<DomainEvent> = Vec::new();
        let mut corrupt_lines: Vec<&str> = Vec::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            match parse_spool_line(line) {
                Ok(event) => {
                    events.push(event);
                    deliverable_lines.push(line);
                }
                Err(_) => corrupt_lines.push(line),
            }
        }
        let corrupt = corrupt_lines.len();
        if !corrupt_lines.is_empty() {
            let mut dead = corrupt_lines.join("\n");
            dead.push('\n');
            let dead_path = self.dead_path();
            if let Some(parent) = dead_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|error| SinkError::Spool {
                        path: parent.display().to_string(),
                        reason: error.to_string(),
                    })?;
            }
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&dead_path)
                .await
                .map_err(|error| SinkError::Spool {
                    path: dead_path.display().to_string(),
                    reason: error.to_string(),
                })?;
            file.write_all(dead.as_bytes())
                .await
                .map_err(|error| SinkError::Spool {
                    path: dead_path.display().to_string(),
                    reason: error.to_string(),
                })?;
        }
        let replayed = match self.primary.emit(&events).await {
            Ok(outcome) => outcome.delivered,
            Err(error) => {
                // Keep every undelivered line spooled; nothing is lost.
                let mut remaining_text = deliverable_lines.join("\n");
                if !remaining_text.is_empty() {
                    remaining_text.push('\n');
                }
                replace_file(&self.spool_path, &remaining_text).await?;
                return Err(error);
            }
        };
        replace_file(&self.spool_path, "").await?;
        Ok(ReplayOutcome {
            replayed,
            corrupt,
            remaining: 0,
        })
    }
}

/// Replace `path` with `contents` through a `<path>.tmp` sibling and a
/// rename (write, flush, rename): a crash mid-rewrite leaves either the old
/// complete file or the new complete file, never a truncated spool.
async fn replace_file(path: &Path, contents: &str) -> Result<(), SinkError> {
    let mut name = path.as_os_str().to_os_string();
    name.push(".tmp");
    let tmp_path = PathBuf::from(name);
    use tokio::io::AsyncWriteExt;
    let write_error = |error: std::io::Error| SinkError::Spool {
        path: tmp_path.display().to_string(),
        reason: error.to_string(),
    };
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(write_error)?;
    file.write_all(contents.as_bytes())
        .await
        .map_err(write_error)?;
    file.flush().await.map_err(write_error)?;
    drop(file);
    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|error| SinkError::Spool {
            path: path.display().to_string(),
            reason: error.to_string(),
        })
}

#[async_trait]
impl<S: EventSink> EventSink for SpoolSink<S> {
    async fn emit(&self, events: &[DomainEvent]) -> Result<EmitOutcome, SinkError> {
        match self.primary.emit(events).await {
            Ok(outcome) => Ok(outcome),
            Err(store_error) => {
                let mut lines = String::new();
                for event in events {
                    let row = event.to_row();
                    let line = serde_json::to_string(&row).map_err(|error| SinkError::Spool {
                        path: self.spool_path.display().to_string(),
                        reason: error.to_string(),
                    })?;
                    lines.push_str(&line);
                    lines.push('\n');
                }
                self.append_lines(&lines).await?;
                // The batch is durable in the spool; report it as spooled,
                // not delivered, with the primary's reason attached.
                Ok(EmitOutcome {
                    delivered: 0,
                    spooled: events.len(),
                    last_error: Some(store_error.to_string()),
                })
            }
        }
    }
}

/// Parse one spool line back into an event (the `EventRow` JSON shape).
fn parse_spool_line(line: &str) -> Result<DomainEvent, String> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
    crate::store::domain_event_from_row(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::envelope::{EventType, ValidationError};
    use crate::store::{MemoryStore, ProjectionStore, StoreError};
    use crate::test_support::{occurred, valid_event};

    #[tokio::test]
    async fn client_stamps_labels_and_refuses_other_environments() {
        let store = Arc::new(MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let client = AnalyticsClient::new(store.clone(), "production", "apex-0.1.0");
        assert_eq!(client.environment(), "production");
        assert_eq!(client.service_version(), "apex-0.1.0");

        let event = client
            .builder(EventType::JobRun, EventSource::Worker)
            .with_event_id("job_1")
            .with_occurred_at(occurred())
            .with_received_at(occurred())
            .with_outcome("completed")
            .with_payload(crate::test_support::payload_for(EventType::JobRun))
            .build()
            .expect("build through client");
        assert_eq!(event.environment, "production");
        assert_eq!(event.service_version, "apex-0.1.0");
        let outcome = client.emit(&event).await.expect("emit");
        assert_eq!(outcome.delivered, 1);

        let foreign = crate::test_support::valid_event_builder(EventType::ReportPullCompleted)
            .with_environment("staging")
            .build()
            .expect("foreign event");
        let error = client
            .emit(&foreign)
            .await
            .expect_err("staging event into production client");
        assert!(matches!(error, SinkError::EnvironmentMismatch { .. }));
    }

    #[tokio::test]
    async fn a_client_built_event_always_has_its_labels() {
        let store = Arc::new(MemoryStore::new());
        let client = AnalyticsClient::new(store, "staging", "apex-0.2.0");
        let event_type = EventType::JobRun;
        let built = client
            .builder(event_type, EventSource::Worker)
            .with_occurred_at(occurred())
            .with_received_at(occurred())
            .with_outcome("completed")
            .with_payload(crate::test_support::payload_for(event_type))
            .build();
        assert!(matches!(built, Ok(event) if event.environment == "staging"));
        // Missing labels stay missing: the builder refuses to invent them.
        let missing = DomainEvent::builder(event_type, EventSource::Worker)
            .with_occurred_at(occurred())
            .with_received_at(occurred())
            .with_payload(crate::test_support::payload_for(event_type))
            .with_outcome("completed")
            .build();
        assert!(matches!(
            missing,
            Err(ValidationError::MissingField {
                field: "environment",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn spool_sink_spools_failed_batches_and_replays_them() {
        let directory =
            std::env::temp_dir().join(format!("apex-analytics-spool-{}", std::process::id()));
        let spool = directory.join("ingest.jsonl");
        let _ = std::fs::remove_file(&spool);

        // A store without a schema fails every insert.
        let cold = Arc::new(MemoryStore::new());
        let client = AnalyticsClient::new(cold.clone(), "production", "apex-0.1.0");
        let sink = SpoolSink::new(client, spool.clone());
        let event = valid_event(EventType::ReportPullCompleted);
        let outcome = sink
            .emit(std::slice::from_ref(&event))
            .await
            .expect("spooled");
        assert_eq!(outcome.delivered, 0);
        assert_eq!(outcome.spooled, 1);
        let spooled = tokio::fs::read_to_string(&spool).await.expect("spool read");
        assert_eq!(spooled.lines().count(), 1);
        assert!(spooled.contains("\"event_type\":\"report_pull_completed\""));

        // Replaying against the cold store fails and keeps the spool.
        let replay = sink.replay_spool().await;
        assert!(replay.is_err());
        let still = tokio::fs::read_to_string(&spool).await.expect("spool kept");
        assert_eq!(still.lines().count(), 1);

        // Warm the store; the replay delivers and truncates.
        cold.ensure_schema().await.expect("schema");
        let replay = sink.replay_spool().await.expect("replay");
        assert_eq!(replay.replayed, 1);
        assert_eq!(replay.corrupt, 0);
        assert_eq!(replay.remaining, 0);
        let truncated = tokio::fs::read_to_string(&spool)
            .await
            .expect("empty spool");
        assert!(truncated.is_empty());
        assert_eq!(cold.load_events().await.expect("load").len(), 1);

        // Replaying an absent spool is a no-op.
        let replay = sink.replay_spool().await.expect("no spool");
        assert_eq!(replay.replayed, 0);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[tokio::test]
    async fn corrupt_spool_lines_are_preserved_to_dead() {
        let directory =
            std::env::temp_dir().join(format!("apex-analytics-dead-{}", std::process::id()));
        let spool = directory.join("ingest.jsonl");
        tokio::fs::create_dir_all(&directory).await.expect("dir");
        let good = serde_json::to_string(&valid_event(EventType::JobRun).to_row()).expect("row");
        tokio::fs::write(&spool, format!("{good}\nnot json at all\n"))
            .await
            .expect("write");

        let store = Arc::new(MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let client = AnalyticsClient::new(store.clone(), "production", "apex-0.1.0");
        let sink = SpoolSink::new(client, spool.clone());
        let replay = sink.replay_spool().await.expect("replay");
        assert_eq!(replay.replayed, 1);
        assert_eq!(replay.corrupt, 1);
        let dead = tokio::fs::read_to_string(directory.join("ingest.jsonl.dead"))
            .await
            .expect("dead file");
        assert_eq!(dead.trim(), "not json at all");

        // A spool line naming an unknown environment is corrupt, not lost.
        tokio::fs::write(&spool, "{}\n").await.expect("write");
        let replay = sink.replay_spool().await.expect("replay");
        assert_eq!(replay.corrupt, 1);
        let dead = tokio::fs::read_to_string(directory.join("ingest.jsonl.dead"))
            .await
            .expect("dead file");
        assert!(dead.contains("{}"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[tokio::test]
    async fn delivered_batches_do_not_touch_the_spool() {
        let directory =
            std::env::temp_dir().join(format!("apex-analytics-direct-{}", std::process::id()));
        let spool = directory.join("ingest.jsonl");
        let store = Arc::new(MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let client = AnalyticsClient::new(store.clone(), "production", "apex-0.1.0");
        let sink = SpoolSink::new(client, spool.clone());
        let outcome = sink
            .emit(&[valid_event(EventType::JobRun)])
            .await
            .expect("delivered");
        assert_eq!(outcome.delivered, 1);
        assert_eq!(outcome.spooled, 0);
        assert!(tokio::fs::metadata(&spool).await.is_err());
        assert_eq!(store.load_events().await.expect("load").len(), 1);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[tokio::test]
    async fn spool_write_failures_surface_as_spool_errors() {
        let directory =
            std::env::temp_dir().join(format!("apex-analytics-blocked-{}", std::process::id()));
        // The spool path is a directory: opening it for append must fail
        // with a typed error, never a panic and never a silent drop.
        tokio::fs::create_dir_all(&directory).await.expect("dir");
        let cold = Arc::new(MemoryStore::new());
        let client = AnalyticsClient::new(cold, "production", "apex-0.1.0");
        let sink = SpoolSink::new(client, directory.clone());
        assert_eq!(sink.spool_path(), directory.as_path());
        let error = sink
            .emit(&[valid_event(EventType::JobRun)])
            .await
            .expect_err("the spool path is not writable");
        assert!(matches!(error, SinkError::Spool { .. }), "{error:?}");

        // Reading a directory as a spool file fails the same way.
        let error = sink.replay_spool().await.expect_err("unreadable spool");
        assert!(matches!(error, SinkError::Spool { .. }), "{error:?}");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[tokio::test]
    async fn sinks_also_work_behind_the_trait() {
        let store = Arc::new(MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let client = AnalyticsClient::new(store.clone(), "production", "apex-0.1.0");
        let direct: &dyn EventSink = &client;
        assert_eq!(direct.emit(&[]).await.expect("empty batch").delivered, 0);
        let outcome = direct
            .emit(&[valid_event(EventType::JobRun)])
            .await
            .expect("through the trait");
        assert_eq!(outcome.delivered, 1);

        let spool =
            std::env::temp_dir().join(format!("apex-analytics-trait-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&spool);
        let wrapped = SpoolSink::new(client, spool.clone());
        let trait_sink: &dyn EventSink = &wrapped;
        let outcome = trait_sink
            .emit(&[valid_event(EventType::ReportPullCompleted)])
            .await
            .expect("delivered through the trait");
        assert_eq!(outcome.delivered, 1);
        assert_eq!(wrapped.primary().environment(), "production");
        let _ = std::fs::remove_file(&spool);
    }

    #[test]
    fn store_errors_surface_their_message() {
        let error: SinkError = StoreError::SchemaNotReady.into();
        assert!(matches!(error, SinkError::Store(_)));
        assert!(error.to_string().contains("schema is not ready"));
    }

    #[tokio::test]
    async fn spool_rewrites_are_atomic_replacements() {
        let directory =
            std::env::temp_dir().join(format!("apex-analytics-atomic-{}", std::process::id()));
        tokio::fs::create_dir_all(&directory).await.expect("dir");
        let spool = directory.join("ingest.jsonl");
        let mut tmp_name = spool.as_os_str().to_os_string();
        tmp_name.push(".tmp");
        let tmp = PathBuf::from(tmp_name);

        // A pre-existing spool with one deliverable line and one dead line:
        // after the rewrite the file holds exactly the intended content and
        // no temp sibling remains, so the rewrite was a replacement rather
        // than a truncate-and-write.
        let good = serde_json::to_string(&valid_event(EventType::JobRun).to_row()).expect("row");
        tokio::fs::write(&spool, format!("{good}\nnot json at all\n"))
            .await
            .expect("write spool");
        let store = Arc::new(MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let client = AnalyticsClient::new(store.clone(), "production", "apex-0.1.0");
        let sink = SpoolSink::new(client, spool.clone());
        let replay = sink.replay_spool().await.expect("replay");
        assert_eq!(replay.replayed, 1);
        assert_eq!(replay.corrupt, 1);
        let after = tokio::fs::read_to_string(&spool)
            .await
            .expect("empty spool");
        assert_eq!(after, "", "the truncation is the intended complete content");
        assert!(!tmp.exists(), "no temp file remains");

        // A failed delivery keeps the undelivered lines: the kept file is
        // exactly the intended content, again with no temp sibling.
        let cold = Arc::new(MemoryStore::new());
        let client = AnalyticsClient::new(cold, "production", "apex-0.1.0");
        let sink = SpoolSink::new(client, spool.clone());
        tokio::fs::write(&spool, format!("{good}\n"))
            .await
            .expect("write spool");
        assert!(sink.replay_spool().await.is_err());
        let kept = tokio::fs::read_to_string(&spool).await.expect("kept spool");
        assert_eq!(
            kept,
            format!("{good}\n"),
            "the undelivered line stays complete"
        );
        assert!(!tmp.exists(), "no temp file remains");
        let _ = std::fs::remove_dir_all(&directory);
    }
}
