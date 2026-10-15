//! Notification sinks: where a rendered message goes.
//!
//! - [`NotificationSink`] is the async delivery contract. A sink reports what
//!   the provider supplied: acceptance (the provider took the message) or
//!   delivery (the provider confirmed arrival). Acceptance is not delivery.
//! - [`WebhookSink`] POSTs `{to, subject, html, text, template_class,
//!   dedup_key, reference}` as JSON to `APEX_NOTIFY_WEBHOOK` with an optional
//!   bearer `APEX_NOTIFY_WEBHOOK_TOKEN`.
//! - [`FileSink`] appends the same JSON object per line to a local file
//!   (development and tests).
//!
//! A non-2xx response is a typed failure, never a silent success.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use apex_metrics::vocab::TemplateClass;

/// The environment variable naming the webhook URL.
pub const WEBHOOK_ENV: &str = "APEX_NOTIFY_WEBHOOK";
/// The environment variable naming the optional webhook bearer token.
pub const WEBHOOK_TOKEN_ENV: &str = "APEX_NOTIFY_WEBHOOK_TOKEN";

/// What a sink reports about one send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    /// The provider accepted the message.
    Accepted,
    /// The provider confirmed delivery.
    Delivered,
}

impl DeliveryState {
    /// The stable state name.
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryState::Accepted => "accepted",
            DeliveryState::Delivered => "delivered",
        }
    }

    /// Parse a provider-supplied state name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "accepted" => Some(DeliveryState::Accepted),
            "delivered" => Some(DeliveryState::Delivered),
            _ => None,
        }
    }
}

/// The provider's answer to one send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    /// What the provider supplied.
    pub state: DeliveryState,
    /// The provider's own reference for the message, when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reference: Option<String>,
}

/// One delivery attempt's payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryMessage {
    /// The validated recipient address.
    pub to: String,
    /// The subject line.
    pub subject: String,
    /// The HTML part.
    pub html: String,
    /// The plain-text part.
    pub text: String,
    /// The template that produced the message.
    #[serde(with = "crate::vocab_serde")]
    pub template_class: TemplateClass,
    /// The deduplication identity.
    pub dedup_key: String,
    /// The durable reference, when present.
    pub reference: Option<String>,
}

impl DeliveryMessage {
    /// Build the sink payload for `message` and `to`.
    pub fn from_message(message: &crate::template::Message, to: &str) -> Self {
        Self {
            to: to.to_string(),
            subject: message.subject.clone(),
            html: message.html.clone(),
            text: message.text.clone(),
            template_class: message.template_class,
            dedup_key: message.dedup_key.clone(),
            reference: message.reference.clone(),
        }
    }
}

/// Why a send failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SinkError {
    /// The transport could not reach the provider.
    #[error("webhook transport failure: {0}")]
    Transport(String),
    /// The provider refused the message.
    #[error("webhook refused the message with status {status}: {detail}")]
    Rejected {
        /// The HTTP status.
        status: u16,
        /// The response detail (bounded).
        detail: String,
    },
    /// The sink is misconfigured.
    #[error("notification sink configuration: {0}")]
    Config(String),
    /// The local file sink could not write.
    #[error("file sink write at {path}: {reason}")]
    File {
        /// The file involved.
        path: String,
        /// The reason.
        reason: String,
    },
    /// The provider's answer could not be understood.
    #[error("webhook response was not the documented JSON shape: {0}")]
    BadResponse(String),
}

/// Where rendered messages are delivered.
#[async_trait]
pub trait NotificationSink: Send + Sync + std::fmt::Debug {
    /// Deliver one message; report what the provider supplied.
    async fn send(&self, message: &DeliveryMessage) -> Result<DeliveryReceipt, SinkError>;
}

/// The webhook sink: JSON POST to `APEX_NOTIFY_WEBHOOK` with an optional
/// bearer token from `APEX_NOTIFY_WEBHOOK_TOKEN`.
#[derive(Debug, Clone)]
pub struct WebhookSink {
    url: String,
    token: Option<String>,
    client: reqwest::Client,
}

/// The documented webhook response body: `{"state": "accepted"|"delivered",
/// "provider_reference": "..."}`. A 2xx without a body is acceptance.
#[derive(Debug, Deserialize)]
struct WebhookReply {
    state: String,
    #[serde(default)]
    provider_reference: Option<String>,
}

impl WebhookSink {
    /// Build a webhook sink. The URL must be an absolute http(s) URL.
    pub fn new(url: &str, token: Option<String>) -> Result<Self, SinkError> {
        let url = url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() < 12 {
            return Err(SinkError::Config(format!(
                "{WEBHOOK_ENV} must be an absolute http(s) URL"
            )));
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|error| SinkError::Config(error.to_string()))?;
        Ok(Self {
            url: url.to_string(),
            token: token.filter(|token| !token.trim().is_empty()),
            client,
        })
    }

    /// Build from `APEX_NOTIFY_WEBHOOK` / `APEX_NOTIFY_WEBHOOK_TOKEN`;
    /// unset URL disables notifications.
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, SinkError> {
        let Some(url) = lookup(WEBHOOK_ENV).filter(|url| !url.trim().is_empty()) else {
            return Ok(None);
        };
        Self::new(&url, lookup(WEBHOOK_TOKEN_ENV)).map(Some)
    }
}

#[async_trait]
impl NotificationSink for WebhookSink {
    async fn send(&self, message: &DeliveryMessage) -> Result<DeliveryReceipt, SinkError> {
        let mut request = self.client.post(&self.url).json(message);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| SinkError::Transport(error.to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| SinkError::Transport(error.to_string()))?;
        if !status.is_success() {
            // Bound the provider's detail by characters, never bytes: a
            // multibyte body at the boundary must not panic.
            let detail = crate::text::bounded_chars(&body, 200);
            return Err(SinkError::Rejected {
                status: status.as_u16(),
                detail,
            });
        }
        if body.trim().is_empty() {
            return Ok(DeliveryReceipt {
                state: DeliveryState::Accepted,
                provider_reference: None,
            });
        }
        let reply: WebhookReply = serde_json::from_str(&body)
            .map_err(|error| SinkError::BadResponse(error.to_string()))?;
        let state = DeliveryState::parse(&reply.state)
            .ok_or_else(|| SinkError::BadResponse(format!("unknown state '{}'", reply.state)))?;
        Ok(DeliveryReceipt {
            state,
            provider_reference: reply.provider_reference,
        })
    }
}

/// The file sink: one JSON object per line, inspectable beside the runner's
/// other records. Reports acceptance (the file is the provider).
#[derive(Debug, Clone)]
pub struct FileSink {
    path: PathBuf,
}

impl FileSink {
    /// Append lines to `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The file path.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[async_trait]
impl NotificationSink for FileSink {
    async fn send(&self, message: &DeliveryMessage) -> Result<DeliveryReceipt, SinkError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| SinkError::File {
                    path: parent.display().to_string(),
                    reason: error.to_string(),
                })?;
            }
        }
        let line = serde_json::to_string(message).map_err(|error| SinkError::File {
            path: self.path.display().to_string(),
            reason: error.to_string(),
        })?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| SinkError::File {
                path: self.path.display().to_string(),
                reason: error.to_string(),
            })?;
        writeln!(file, "{line}").map_err(|error| SinkError::File {
            path: self.path.display().to_string(),
            reason: error.to_string(),
        })?;
        Ok(DeliveryReceipt {
            state: DeliveryState::Accepted,
            provider_reference: None,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::template::{guardrail_stop, GuardrailStop};

    fn sample() -> DeliveryMessage {
        let message = guardrail_stop(&GuardrailStop {
            experiment_id: "exp_0242".into(),
            action_url: "https://console.apexmediation.ee/experiments/exp_0242".into(),
        })
        .expect("message");
        DeliveryMessage::from_message(&message, "ops@example.com")
    }

    #[test]
    fn delivery_states_round_trip() {
        assert_eq!(
            DeliveryState::parse("accepted"),
            Some(DeliveryState::Accepted)
        );
        assert_eq!(
            DeliveryState::parse("delivered"),
            Some(DeliveryState::Delivered)
        );
        assert_eq!(DeliveryState::parse("queued"), None);
        assert_eq!(DeliveryState::Accepted.as_str(), "accepted");
        assert_eq!(DeliveryState::Delivered.as_str(), "delivered");
    }

    #[test]
    fn provider_details_are_bounded_by_characters_not_bytes() {
        // A multibyte character straddling the byte bound must not panic and
        // must never split a character.
        let text = "é".repeat(500);
        let bounded = crate::text::bounded_chars(&text, 200);
        assert_eq!(bounded.chars().count(), 200);
        assert_eq!(bounded, "é".repeat(200));
        // A boundary exactly at 200 bytes with 2-byte characters is the
        // classic `String::truncate(200)` panic case; the bounded copy is
        // whole.
        let text = format!("{}€", "a".repeat(199));
        let bounded = crate::text::bounded_chars(&text, 200);
        assert_eq!(bounded, text.to_string());
        // Short text is returned unchanged.
        assert_eq!(crate::text::bounded_chars("ok", 200), "ok");
    }

    #[test]
    fn webhook_config_is_validated() {
        assert!(WebhookSink::new("https://hooks.example.test/notify", None).is_ok());
        assert!(WebhookSink::new("http://127.0.0.1:9999/notify", None).is_ok());
        assert!(WebhookSink::new("hooks.example.test", None).is_err());
        assert!(WebhookSink::new("", None).is_err());
        assert!(WebhookSink::from_env(|_| None).expect("unset").is_none());
        let sink = WebhookSink::from_env(|name| match name {
            WEBHOOK_ENV => Some("https://hooks.example.test/notify".into()),
            _ => None,
        })
        .expect("configured")
        .expect("some");
        assert!(format!("{sink:?}").contains("WebhookSink"));
        let error = WebhookSink::from_env(|name| match name {
            WEBHOOK_ENV => Some("not-a-url".into()),
            _ => None,
        })
        .expect_err("bad url");
        assert!(matches!(error, SinkError::Config(_)));
    }

    #[tokio::test]
    async fn the_file_sink_appends_one_json_object_per_message() {
        let dir = std::env::temp_dir().join(format!("apex-notify-sink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sink = FileSink::new(dir.join("outbox.jsonl"));
        let receipt = sink.send(&sample()).await.expect("send");
        assert_eq!(receipt.state, DeliveryState::Accepted);
        sink.send(&sample()).await.expect("send again");
        let contents = std::fs::read_to_string(sink.path()).expect("read");
        assert_eq!(contents.lines().count(), 2);
        let row: serde_json::Value =
            serde_json::from_str(contents.lines().next().unwrap()).unwrap();
        assert_eq!(row["to"], "ops@example.com");
        assert_eq!(row["template_class"], "guardrail_stop");
        assert_eq!(row["dedup_key"], "guardrail_stop:exp_0242");
        assert!(row["html"].as_str().unwrap().contains("width=\"600\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_file_sink_reports_write_failures() {
        let dir = std::env::temp_dir().join(format!("apex-notify-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        // The path is a directory: appending to it must fail typed.
        let error = FileSink::new(&dir).send(&sample()).await.expect_err("dir");
        assert!(matches!(error, SinkError::File { .. }), "{error:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A one-shot HTTP listener that answers with `response_body` and records
    /// the raw request bytes.
    fn spawn_server(
        status_line: &'static str,
        response_body: &'static str,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 4096];
                let mut header_end = None;
                let mut content_length = 0usize;
                loop {
                    let read = match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(read) => read,
                        Err(_) => break,
                    };
                    buffer.extend_from_slice(&chunk[..read]);
                    if header_end.is_none() {
                        if let Some(position) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = Some(position + 4);
                            let header_text =
                                String::from_utf8_lossy(&buffer[..position]).to_lowercase();
                            content_length = header_text
                                .lines()
                                .find_map(|line| line.strip_prefix("content-length:"))
                                .and_then(|value| value.trim().parse().ok())
                                .unwrap_or(0);
                        }
                    }
                    if let Some(header_end) = header_end {
                        if buffer.len() >= header_end + content_length {
                            break;
                        }
                    }
                }
                let _ = sender.send(String::from_utf8_lossy(&buffer).into_owned());
                let response = format!(
                    "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (format!("http://{addr}/notify"), receiver)
    }

    #[tokio::test]
    async fn the_webhook_sink_posts_the_documented_body_with_bearer_auth() {
        let (url, requests) = spawn_server(
            "HTTP/1.1 200 OK",
            r#"{"state":"delivered","provider_reference":"msg_1"}"#,
        );
        let sink = WebhookSink::new(&url, Some("secret-token".into())).expect("sink");
        let receipt = sink.send(&sample()).await.expect("send");
        assert_eq!(receipt.state, DeliveryState::Delivered);
        assert_eq!(receipt.provider_reference.as_deref(), Some("msg_1"));
        let raw = requests.recv().expect("request");
        assert!(raw.starts_with("POST /notify"));
        assert!(
            raw.contains("authorization: Bearer secret-token")
                || raw.contains("Authorization: Bearer secret-token")
        );
        assert!(raw.contains(r#""to":"ops@example.com""#));
        assert!(raw.contains(r#""template_class":"guardrail_stop""#));
        assert!(raw.contains(r#""dedup_key":"guardrail_stop:exp_0242""#));
        assert!(raw.contains(r#""reference":"exp_0242""#));
        assert!(raw.contains(r#""html":"<!doctype html>"#));
    }

    #[tokio::test]
    async fn the_webhook_sink_treats_empty_2xx_as_acceptance() {
        let (url, requests) = spawn_server("HTTP/1.1 202 Accepted", "");
        let sink = WebhookSink::new(&url, None).expect("sink");
        let receipt = sink.send(&sample()).await.expect("send");
        assert_eq!(receipt.state, DeliveryState::Accepted);
        assert!(receipt.provider_reference.is_none());
        let raw = requests.recv().expect("request");
        assert!(!raw.to_lowercase().contains("authorization:"));
    }

    #[tokio::test]
    async fn webhook_refusals_and_bad_bodies_are_typed_failures() {
        let (url, requests) = spawn_server("HTTP/1.1 500 Internal Server Error", "boom");
        let sink = WebhookSink::new(&url, None).expect("sink");
        let error = sink.send(&sample()).await.expect_err("refused");
        assert!(
            matches!(error, SinkError::Rejected { status: 500, .. }),
            "{error:?}"
        );
        let _ = requests.recv();

        let (url, requests) = spawn_server("HTTP/1.1 200 OK", r#"{"state":"queued"}"#);
        let sink = WebhookSink::new(&url, None).expect("sink");
        let error = sink.send(&sample()).await.expect_err("unknown state");
        assert!(matches!(error, SinkError::BadResponse(_)), "{error:?}");
        let _ = requests.recv();

        // An unreachable endpoint is a transport failure, not a silent drop.
        let sink = WebhookSink::new("http://127.0.0.1:1/notify", None).expect("sink");
        let error = sink.send(&sample()).await.expect_err("unreachable");
        assert!(matches!(error, SinkError::Transport(_)), "{error:?}");
    }
}
