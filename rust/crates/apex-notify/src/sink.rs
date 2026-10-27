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
//! - [`EmailSink`] pipes the composed RFC 5322 message to the local MTA's
//!   `sendmail` interface, the same transport the sales engine uses: the
//!   message (a `multipart/alternative` pair of the frozen HTML part and its
//!   plain-text twin) arrives on stdin, the envelope is explicit
//!   (`-i -f <envelope-from> -- <recipient>`), and a nonzero exit status is
//!   the failure. The executable comes from `APEX_NOTIFY_SENDMAIL`
//!   (default `/usr/sbin/sendmail`, the sales engine's default). One sink is
//!   selected by [`select_sink`].
//!
//! A non-2xx response or a nonzero MTA exit is a typed failure, never a
//! silent success.
//!
//! # The email sink's contract
//!
//! - Sender identity: `APEX_NOTIFY_FROM`, exactly one address, either
//!   `hello@apex-transactional.example` or with a display name
//!   (`Apex <hello@apex-transactional.example>`). It is validated at
//!   construction and an absent or malformed value is a typed
//!   [`SinkError::Config`] refusal, never a fallback sender.
//! - Domain separation: the transactional domain must be kept apart from
//!   the sales engine's prospecting domain, so transactional deliverability
//!   and reputation are not coupled to cold outreach. DNS (SPF, DKIM,
//!   DMARC) lives outside this process; nothing in this crate can enforce
//!   the domain choice, and this configuration contract is the
//!   documentation.
//! - Delivery states: `sendmail` exit 0 is acceptance, recorded as
//!   `accepted` after the `requested` row; a nonzero exit or a timeout is
//!   `failed` with the MTA's stderr excerpt bounded by characters, never
//!   bytes. A local MTA never supplies delivery confirmation, so
//!   `accepted` is terminal for this sink. The states remain distinct rows
//!   per the design, and an `accepted` row deduplicates retries exactly like
//!   the other sinks.
//!
//! # Selection
//!
//! [`select_sink`] builds exactly one sink. `APEX_NOTIFY_SINK` names the
//! kind (`email`, `webhook` or `file`); unset keeps the pre-existing order
//! (webhook when `APEX_NOTIFY_WEBHOOK` is set, else file when
//! `APEX_NOTIFY_FILE` is set, else none configured), so existing selections
//! are unchanged.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use apex_metrics::vocab::TemplateClass;

/// The environment variable naming the webhook URL.
pub const WEBHOOK_ENV: &str = "APEX_NOTIFY_WEBHOOK";
/// The environment variable naming the optional webhook bearer token.
pub const WEBHOOK_TOKEN_ENV: &str = "APEX_NOTIFY_WEBHOOK_TOKEN";
/// The environment variable selecting the sink kind (`email`, `webhook` or
/// `file`).
pub const SINK_ENV: &str = "APEX_NOTIFY_SINK";
/// The environment variable naming the file sink's output file.
pub const FILE_ENV: &str = "APEX_NOTIFY_FILE";
/// The environment variable naming the transactional sender identity.
pub const FROM_ENV: &str = "APEX_NOTIFY_FROM";
/// The environment variable overriding the `sendmail` executable.
pub const SENDMAIL_ENV: &str = "APEX_NOTIFY_SENDMAIL";
/// The system `sendmail` path, the same default the sales engine uses.
pub const DEFAULT_SENDMAIL: &str = "/usr/sbin/sendmail";
/// The only search path the child MTA sees.
pub const SENDMAIL_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";
/// A harmless home for MTAs that insist on one.
pub const SENDMAIL_HOME: &str = "/nonexistent";

/// How long the email sink waits for `sendmail` before killing it and
/// failing: a wedged MTA must not wedge the runner.
const SENDMAIL_TIMEOUT: Duration = Duration::from_secs(30);

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
    /// The absolute one-click unsubscribe URL, when the message has one.
    /// Rendered as both `List-Unsubscribe` and the RFC 8058 one-click post
    /// hint; absent on messages with no cancellation surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_unsubscribe: Option<String>,
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
            list_unsubscribe: None,
        }
    }
}

/// Why a send failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SinkError {
    /// The transport could not reach the provider.
    #[error("webhook transport failure: {0}")]
    Transport(String),
    /// The local MTA failed, or the sink refused the message before handing
    /// it over. The detail is the MTA's stderr excerpt, bounded by
    /// characters.
    #[error("mail transport failure: {0}")]
    Mail(String),
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

    /// Deliver one recipient batch, returning one result per message in the
    /// same order. The default preserves per-message transport semantics; a
    /// sink whose transport can batch (the local file) overrides it.
    async fn send_batch(
        &self,
        messages: &[DeliveryMessage],
    ) -> Vec<Result<DeliveryReceipt, SinkError>> {
        let mut results = Vec::with_capacity(messages.len());
        for message in messages {
            results.push(self.send(message).await);
        }
        results
    }
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

// How many times `FileSink` opened its output file on this test thread.
// Tests use it to prove the batch path opens once; the counter is compiled
// only for tests and is thread-local so parallel tests cannot skew it.
#[cfg(test)]
thread_local! {
    pub(crate) static FILE_SINK_OPENS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
        let mut results = self.send_batch(std::slice::from_ref(message)).await;
        results.pop().unwrap_or_else(|| {
            Err(SinkError::File {
                path: self.path.display().to_string(),
                reason: "the batch returned no result".to_string(),
            })
        })
    }

    /// Open, append and flush once for the whole recipient batch. A message
    /// that cannot be encoded, or a write that fails, is reported at its own
    /// index; the other recipients are still attempted.
    async fn send_batch(
        &self,
        messages: &[DeliveryMessage],
    ) -> Vec<Result<DeliveryReceipt, SinkError>> {
        if messages.is_empty() {
            return Vec::new();
        }
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    let error = SinkError::File {
                        path: parent.display().to_string(),
                        reason: error.to_string(),
                    };
                    return messages.iter().map(|_| Err(error.clone())).collect();
                }
            }
        }
        let mut slots: Vec<Option<Result<DeliveryReceipt, SinkError>>> =
            (0..messages.len()).map(|_| None).collect();
        let mut encoded: Vec<Option<String>> = Vec::with_capacity(messages.len());
        for (index, message) in messages.iter().enumerate() {
            match serde_json::to_string(message) {
                Ok(line) => encoded.push(Some(line)),
                Err(error) => {
                    slots[index] = Some(Err(SinkError::File {
                        path: self.path.display().to_string(),
                        reason: error.to_string(),
                    }));
                    encoded.push(None);
                }
            }
        }
        if encoded.iter().all(Option::is_none) {
            return slots
                .into_iter()
                .map(|slot| {
                    slot.unwrap_or_else(|| {
                        Err(SinkError::File {
                            path: self.path.display().to_string(),
                            reason: "the batch returned no result".to_string(),
                        })
                    })
                })
                .collect();
        }
        #[cfg(test)]
        FILE_SINK_OPENS.with(|opens| opens.set(opens.get() + 1));
        use std::io::Write;
        let file = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(error) => {
                let error = SinkError::File {
                    path: self.path.display().to_string(),
                    reason: error.to_string(),
                };
                return messages.iter().map(|_| Err(error.clone())).collect();
            }
        };
        let mut writer = std::io::BufWriter::new(file);
        for (index, line) in encoded.iter().enumerate() {
            let Some(line) = line else { continue };
            if let Err(error) = writeln!(writer, "{line}") {
                slots[index] = Some(Err(SinkError::File {
                    path: self.path.display().to_string(),
                    reason: error.to_string(),
                }));
            } else {
                slots[index] = Some(Ok(DeliveryReceipt {
                    state: DeliveryState::Accepted,
                    provider_reference: None,
                }));
            }
        }
        if let Err(error) = writer.flush() {
            let error = SinkError::File {
                path: self.path.display().to_string(),
                reason: error.to_string(),
            };
            for slot in slots.iter_mut().filter(|slot| slot.is_none()) {
                *slot = Some(Err(error.clone()));
            }
        }
        // Financial alerts (payment failures, invoice reminders) must survive
        // a crash: flush alone only reaches the page cache. A failed fsync is
        // a failed delivery for every message in the batch, never an accepted
        // receipt.
        if let Err(error) = writer.get_ref().sync_all() {
            let error = SinkError::File {
                path: self.path.display().to_string(),
                reason: format!("fsync failed: {error}"),
            };
            for slot in &mut slots {
                *slot = Some(Err(error.clone()));
            }
        }
        slots
            .into_iter()
            .map(|slot| {
                slot.unwrap_or_else(|| {
                    Err(SinkError::File {
                        path: self.path.display().to_string(),
                        reason: "the batch returned no result".to_string(),
                    })
                })
            })
            .collect()
    }
}

/// The email sink: the composed RFC 5322 message is piped to the local
/// MTA's `sendmail` interface, the same transport (and invocation semantics)
/// the sales engine uses: `-i -f <envelope-from> -- <recipient>` with the
/// recipient given explicitly, so header contents can never add recipients.
/// Delivery, signing, retries and bounces are the MTA's job.
///
/// A `sendmail` exit of 0 reports [`DeliveryState::Accepted`]; a nonzero
/// exit (or a wait beyond 30 seconds) is a [`SinkError::Mail`] failure with
/// the MTA's stderr excerpt. A local MTA supplies no delivery confirmation,
/// so `accepted` is terminal for this sink and `delivered` is never
/// reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailSink {
    from_header: String,
    from_address: String,
    command: String,
}

impl EmailSink {
    /// Build an email sink. `from` is the `APEX_NOTIFY_FROM` value: exactly
    /// one address, either a bare address or `Display Name <address>`. An
    /// absent or malformed value is a typed refusal. `command` names the
    /// `sendmail` executable (never a shell string).
    ///
    /// The transactional domain must be kept apart from the sales engine's
    /// domain; no code here can enforce DNS, so the configuration is the
    /// contract (see the module documentation).
    pub fn new(from: Option<&str>, command: impl Into<String>) -> Result<Self, SinkError> {
        let (from_header, from_address) = parse_sender(from)?;
        let command = command.into();
        let command = command.trim().to_string();
        if command.is_empty() {
            return Err(SinkError::Config(format!(
                "{SENDMAIL_ENV} must name the sendmail executable"
            )));
        }
        Ok(Self {
            from_header,
            from_address,
            command,
        })
    }

    /// Build from `APEX_NOTIFY_FROM` (required) and `APEX_NOTIFY_SENDMAIL`
    /// (optional; defaults to [`DEFAULT_SENDMAIL`], the sales engine's
    /// default).
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, SinkError> {
        let from = lookup(FROM_ENV).filter(|value| !value.trim().is_empty());
        let command = lookup(SENDMAIL_ENV)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SENDMAIL.to_string());
        Self::new(from.as_deref(), command)
    }

    /// The `From:` header value as configured.
    pub fn from_header(&self) -> &str {
        &self.from_header
    }

    /// The envelope sender (the bare address from the configured sender).
    pub fn from_address(&self) -> &str {
        &self.from_address
    }

    /// The `sendmail` executable.
    pub fn command(&self) -> &str {
        &self.command
    }
}

/// Refuse anything that is not one public dot-atom mailbox. The recipient is
/// both a header value and a `sendmail` envelope argument; pipes, commas,
/// quotes and local-only domains must never reach the MTA.
fn check_recipient(to: &str) -> Result<(), SinkError> {
    let valid = to.len() <= 254
        && !to.starts_with('-')
        && to.matches('@').count() == 1
        && !to.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(
                    c,
                    '|' | '/' | ',' | '<' | '>' | '"' | '(' | ')' | '[' | ']' | ':' | ';' | '\\'
                )
        })
        && to.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && local.len() <= 64
                && !local.starts_with('.')
                && !local.ends_with('.')
                && !local.contains("..")
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !domain.contains("..")
        });
    if !valid {
        return Err(SinkError::Mail(
            "refused a recipient that is not one public mailbox".into(),
        ));
    }
    Ok(())
}

/// Refuse a header value that could smuggle a line.
fn check_header(name: &str, value: &str) -> Result<(), SinkError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(SinkError::Mail(format!(
            "refused an empty {name} or one with control characters"
        )));
    }
    Ok(())
}

#[async_trait]
impl NotificationSink for EmailSink {
    async fn send(&self, message: &DeliveryMessage) -> Result<DeliveryReceipt, SinkError> {
        check_recipient(&message.to)?;
        check_header("Subject", &message.subject)?;
        if message.to.starts_with('-') {
            return Err(SinkError::Mail(
                "refused a To that looks like a sendmail option".into(),
            ));
        }
        let now_unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        let domain = self.from_address.rsplit('@').next().unwrap_or_default();
        let raw = crate::mail::compose(
            &self.from_header,
            &message.to,
            &message.subject,
            &message.text,
            &message.html,
            &message.dedup_key,
            domain,
            message.list_unsubscribe.as_deref(),
            now_unix_secs,
        );
        let mut child = Command::new(&self.command)
            .args(["-i", "-f", &self.from_address, "--", &message.to])
            // Never hand the service environment (vault keys, provider
            // tokens) to a local executable. The MTA needs only a search path and a
            // harmless home directory; everything else is cleared.
            .env_clear()
            .env("PATH", SENDMAIL_PATH)
            .env("HOME", SENDMAIL_HOME)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| SinkError::Mail(error.to_string()))?;
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write as _;
            if let Err(error) = stdin.write_all(raw.as_bytes()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SinkError::Mail(error.to_string()));
            }
        }
        // Drain stderr while waiting so a verbose MTA cannot deadlock on a
        // full pipe; the wait is bounded so a wedged MTA cannot wedge the
        // runner.
        let stderr_reader = child.stderr.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                use std::io::Read as _;
                let _ = pipe.read_to_end(&mut bytes);
                bytes
            })
        });
        let deadline = std::time::Instant::now() + SENDMAIL_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if std::time::Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(SinkError::Mail(format!(
                        "sendmail did not exit within {}s and was killed",
                        SENDMAIL_TIMEOUT.as_secs()
                    )));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(SinkError::Mail(error.to_string()));
                }
            }
        };
        let stderr = stderr_reader
            .map(|reader| reader.join().unwrap_or_default())
            .unwrap_or_default();
        if status.success() {
            return Ok(DeliveryReceipt {
                state: DeliveryState::Accepted,
                provider_reference: None,
            });
        }
        // Bound the MTA's detail by characters, never bytes: a multibyte
        // character at the boundary must not panic and must survive whole.
        let mut detail: String = String::from_utf8_lossy(&stderr)
            .trim()
            .chars()
            .take(300)
            .collect();
        if detail.is_empty() {
            detail = format!("sendmail failed with no stderr ({status})");
        }
        Err(SinkError::Mail(detail))
    }
}

/// Build exactly one sink from the environment, keeping the pre-existing
/// selection when [`SINK_ENV`] is unset:
///
/// | `APEX_NOTIFY_SINK` | requires | result |
/// |--------------------|----------|--------|
/// | `email` | `APEX_NOTIFY_FROM` | [`EmailSink`] |
/// | `webhook` | `APEX_NOTIFY_WEBHOOK` | [`WebhookSink`] |
/// | `file` | `APEX_NOTIFY_FILE` | [`FileSink`] |
/// | unset | — | the legacy order: webhook, else file, else `Ok(None)` |
///
/// An unknown value or a kind whose required variable is missing is a typed
/// configuration error, never a silent fallback to a different provider.
pub fn select_sink(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Option<Arc<dyn NotificationSink>>, SinkError> {
    let selected = lookup(SINK_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let Some(selected) = selected else {
        // The pre-existing selection order, unchanged.
        if let Some(webhook) = WebhookSink::from_env(&lookup)? {
            return Ok(Some(Arc::new(webhook)));
        }
        if let Some(path) = lookup(FILE_ENV).filter(|value| !value.trim().is_empty()) {
            return Ok(Some(Arc::new(FileSink::new(path.trim()))));
        }
        return Ok(None);
    };
    match selected.as_str() {
        "email" => Ok(Some(Arc::new(EmailSink::from_env(lookup)?))),
        "webhook" => match WebhookSink::from_env(&lookup)? {
            Some(webhook) => Ok(Some(Arc::new(webhook))),
            None => Err(SinkError::Config(format!(
                "{WEBHOOK_ENV} is required when {SINK_ENV}=webhook"
            ))),
        },
        "file" => match lookup(FILE_ENV).filter(|value| !value.trim().is_empty()) {
            Some(path) => Ok(Some(Arc::new(FileSink::new(path.trim())))),
            None => Err(SinkError::Config(format!(
                "{FILE_ENV} is required when {SINK_ENV}=file"
            ))),
        },
        other => Err(SinkError::Config(format!(
            "{SINK_ENV} must be 'email', 'webhook' or 'file'; got '{other}'"
        ))),
    }
}

/// Parse `APEX_NOTIFY_FROM`: exactly one address, optionally with a display
/// name. Returns `(From: header value, bare address)`.
fn parse_sender(from: Option<&str>) -> Result<(String, String), SinkError> {
    let refuse = |why: &str| {
        SinkError::Config(format!(
            "{FROM_ENV} must be one email address such as \"Apex <hello@apex-transactional.example>\" ({why})"
        ))
    };
    let Some(raw) = from else {
        return Err(refuse("it is not set"));
    };
    let value = raw.trim();
    if value.is_empty() {
        return Err(refuse("it is empty"));
    }
    if value.len() > 320 {
        return Err(refuse("it is too long"));
    }
    if value.chars().any(|c| !c.is_ascii() || c.is_control()) {
        return Err(refuse("only printable ASCII is accepted"));
    }
    if value.contains(',') || value.contains(';') {
        return Err(refuse("it lists more than one address"));
    }
    let (header, address) = split_display(value).map_err(refuse)?;
    validate_address(&address).map_err(refuse)?;
    Ok((header, address))
}

/// Split `Name <address>` into its header form and bare address; a value
/// without angle brackets must already be the bare address.
fn split_display(value: &str) -> Result<(String, String), &'static str> {
    if !value.contains('<') && !value.contains('>') {
        return Ok((value.to_string(), value.to_string()));
    }
    if !value.ends_with('>') || value.matches('<').count() != 1 || value.matches('>').count() != 1 {
        return Err("the display-name form is \"Name <address>\"");
    }
    let open = value
        .find('<')
        .ok_or("the display-name form is \"Name <address>\"")?;
    let display = value[..open].trim();
    let address = value[open + 1..value.len() - 1].trim();
    if display.is_empty() || display.chars().any(|c| c == '"' || c == '<' || c == '>') {
        return Err("the display name is empty or not plain");
    }
    Ok((format!("{display} <{address}>"), address.to_string()))
}

/// Minimal single-address validation: a local part, one `@`, and a dotted
/// hostname domain. Never a zero-length or option-shaped envelope.
fn validate_address(address: &str) -> Result<(), &'static str> {
    const LOCAL: &str = "!#$%&'*+-/=?^_`{|}~.";
    if address.is_empty() {
        return Err("the address is empty");
    }
    if address.len() > 254 {
        return Err("the address is too long");
    }
    if address.starts_with('-') {
        return Err("the address must not start with '-'");
    }
    let Some((local, domain)) = address.split_once('@') else {
        return Err("the address has no '@'");
    };
    if local.is_empty()
        || !local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || LOCAL.contains(c))
    {
        return Err("the local part is empty or carries unsupported characters");
    }
    if domain.is_empty()
        || domain.contains("..")
        || domain.starts_with('.')
        || domain.ends_with('.')
        || !domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return Err("the domain part is not a hostname");
    }
    if !domain.contains('.') {
        return Err("the domain needs a dot (a bare hostname is not accepted)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::template::{guardrail_stop, GuardrailStop};

    #[test]
    fn recipient_boundaries_are_enforced() {
        assert!(check_recipient("owner@example.org").is_ok());
        for bad in [
            "-dash@example.org",
            "user@nodot",
            "root,victim@example.org",
            "|/bin/sh@localhost.localdomain",
            "Display <user@example.org>",
            "user@localhost",
        ] {
            assert!(check_recipient(bad).is_err(), "{bad}");
        }
        let local_65 = format!("{}@example.org", "a".repeat(65));
        assert!(check_recipient(&local_65).is_err());
        let domain = format!("{}.example.org", "b".repeat(177));
        let exactly_254 = format!("{}@{domain}", "a".repeat(64));
        assert_eq!(exactly_254.len(), 254);
        assert!(check_recipient(&exactly_254).is_ok());
        let one_more = format!("x{exactly_254}");
        assert!(check_recipient(&one_more).is_err());
    }

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
    async fn the_file_sink_batch_opens_once_while_single_sends_open_each_time() {
        FILE_SINK_OPENS.with(|opens| opens.set(0));
        let dir =
            std::env::temp_dir().join(format!("apex-notify-sink-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sink = FileSink::new(dir.join("outbox.jsonl"));
        let mut second = sample();
        second.to = "fin@example.com".to_string();
        let results = sink.send_batch(&[sample(), second]).await;
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.is_ok()), "{results:?}");
        assert_eq!(
            FILE_SINK_OPENS.with(std::cell::Cell::get),
            1,
            "one open for the whole recipient batch"
        );
        let contents = std::fs::read_to_string(sink.path()).expect("read");
        assert_eq!(contents.lines().count(), 2);

        // Measurement of the previous per-message path: two sends open twice.
        FILE_SINK_OPENS.with(|opens| opens.set(0));
        sink.send(&sample()).await.expect("send 1");
        sink.send(&sample()).await.expect("send 2");
        assert_eq!(
            FILE_SINK_OPENS.with(std::cell::Cell::get),
            2,
            "the single-send path opens once per message"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_email_sink_still_refuses_header_injection() {
        let dir = email_temp_dir("inject");
        let sink = EmailSink::new(
            Some("Apex <hello@apex-transactional.example>"),
            fake_sendmail(&dir, "ok.sh", "cat > /dev/null\nexit 0"),
        )
        .expect("sink");
        let mut message = sample();
        message.subject = "Hello\r\nBcc: evil@example.com".to_string();
        let error = sink.send(&message).await.expect_err("header injection");
        assert!(matches!(error, SinkError::Mail(_)), "{error:?}");
        assert!(error.to_string().contains("control characters"), "{error}");
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

    use crate::contacts::Contact;
    use crate::notifier::{Notifier, NotifyReport};
    use crate::state::DeliveryLog;
    use apex_metrics::Registry;

    /// The template message behind [`sample`], for driving the notifier.
    fn template_message() -> crate::template::Message {
        guardrail_stop(&GuardrailStop {
            experiment_id: "exp_0242".into(),
            action_url: "https://console.apexmediation.ee/experiments/exp_0242".into(),
        })
        .unwrap()
    }

    #[test]
    fn email_config_requires_one_valid_sender_address() {
        for good in [
            "Apex <hello@apex-transactional.example>",
            "hello@apex-transactional.example",
            "a.b+c@x.example",
            "Billing <billing@tx.example>",
        ] {
            assert!(EmailSink::new(Some(good), "sendmail").is_ok(), "{good}");
        }
        let sink = EmailSink::new(
            Some("  Apex <hello@apex-transactional.example>  "),
            " /usr/sbin/sendmail ",
        )
        .expect("trimmed");
        assert_eq!(
            sink.from_header(),
            "Apex <hello@apex-transactional.example>"
        );
        assert_eq!(sink.from_address(), "hello@apex-transactional.example");
        assert_eq!(sink.command(), "/usr/sbin/sendmail");
        assert!(format!("{sink:?}").contains("EmailSink"));

        for bad in [
            "not an address",
            "a@b",
            "a@x.",
            "a@.x",
            "a@x..y",
            "he llo@x.y",
            "-oQ/tmp@x.example",
            "A <a@x.y>, B <b@x.y>",
            "A <a@x.y> trailing",
            "A <a@x.y",
            "A a@x.y>",
            "A <a@x.y> <b@x.y>",
            "a@x.y\r\nBcc: leak@x.y",
            "Äpex <a@x.y>",
        ] {
            let error = EmailSink::new(Some(bad), "sendmail").expect_err(bad);
            assert!(matches!(error, SinkError::Config(_)), "{bad}: {error:?}");
            assert!(error.to_string().contains(FROM_ENV), "{bad}: {error}");
        }
        assert!(EmailSink::new(None, "sendmail").is_err(), "absent sender");
        assert!(
            EmailSink::new(Some(""), "sendmail").is_err(),
            "empty sender"
        );
        assert!(
            EmailSink::new(Some("a@x.y"), "   ").is_err(),
            "empty command is refused"
        );

        // from_env: APEX_NOTIFY_FROM is required; the command defaults to the
        // sales engine's sendmail path and takes the override when set.
        let error = EmailSink::from_env(|_| None).expect_err("absent");
        assert!(error.to_string().contains(FROM_ENV), "{error}");
        let sink = EmailSink::from_env(|name| match name {
            FROM_ENV => Some("Apex <hello@apex-transactional.example>".into()),
            _ => None,
        })
        .expect("from");
        assert_eq!(sink.command(), DEFAULT_SENDMAIL);
        let sink = EmailSink::from_env(|name| match name {
            FROM_ENV => Some("hello@tx.example".into()),
            SENDMAIL_ENV => Some("  /opt/fake/sendmail  ".into()),
            _ => None,
        })
        .expect("override");
        assert_eq!(sink.command(), "/opt/fake/sendmail");
        assert_eq!(sink.from_header(), "hello@tx.example");
    }

    /// A stand-in `sendmail`: captures argv and the piped message to files.
    #[cfg(unix)]
    fn fake_sendmail(dir: &std::path::Path, name: &str, script: &str) -> String {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[cfg(unix)]
    fn email_temp_dir(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("apex-notify-email-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Executing a just-written script can race a concurrent fork in another
    /// test thread that still holds its write descriptor (ETXTBSY); retry.
    #[cfg(unix)]
    async fn send_settled(
        sink: &EmailSink,
        message: &DeliveryMessage,
    ) -> Result<DeliveryReceipt, SinkError> {
        let mut last = sink.send(message).await;
        for _ in 0..20 {
            match &last {
                Err(SinkError::Mail(detail)) if detail.contains("busy") => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    last = sink.send(message).await;
                }
                _ => break,
            }
        }
        last
    }

    /// The service environment must not reach the child MTA: a local
    /// executable could otherwise read vault keys or provider tokens.
    #[cfg(unix)]
    #[tokio::test]
    async fn sendmail_never_inherits_the_service_environment() {
        let dir = email_temp_dir("env-clear");
        let env_path = dir.join("env.txt");
        let script = format!("env > '{}'\nexit 0", env_path.display());
        let sink = EmailSink::new(
            Some("Apex <hello@apex-transactional.example>"),
            fake_sendmail(&dir, "env.sh", &script),
        )
        .unwrap();

        let secret_name = format!("APEX_TEST_VAULT_SECRET_{}", std::process::id());
        let secret_value = "vault-material-that-must-not-leak";
        std::env::set_var(&secret_name, secret_value);
        let sent = send_settled(&sink, &sample()).await;
        std::env::remove_var(&secret_name);
        sent.expect("send");

        let environment = std::fs::read_to_string(&env_path).expect("environment dump");
        assert!(
            !environment.contains(&secret_name),
            "the secret name reached sendmail: {environment}"
        );
        assert!(
            !environment.contains(secret_value),
            "the secret value reached sendmail: {environment}"
        );
        assert!(
            environment
                .lines()
                .any(|line| line == format!("PATH={SENDMAIL_PATH}")),
            "only the fixed PATH is passed: {environment}"
        );
        assert!(
            environment
                .lines()
                .any(|line| line == format!("HOME={SENDMAIL_HOME}")),
            "the harmless HOME is passed: {environment}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_email_sink_pipes_argv_and_message_intact_to_sendmail() {
        let dir = email_temp_dir("intact");
        let argv_path = dir.join("argv.txt");
        let body_path = dir.join("body.eml");
        let script = format!(
            "printf '%s\\n' \"$@\" > '{}'\ncat > '{}'\nexit 0",
            argv_path.display(),
            body_path.display()
        );
        let sink = EmailSink::new(
            Some("Apex <hello@apex-transactional.example>"),
            fake_sendmail(&dir, "ok.sh", &script),
        )
        .unwrap();
        let message = sample();
        let receipt = send_settled(&sink, &message).await.expect("send");
        assert_eq!(receipt.state, DeliveryState::Accepted);
        assert!(
            receipt.provider_reference.is_none(),
            "a local MTA supplies no reference"
        );

        // The envelope is explicit, sales-engine style: `-i -f <from> -- <to>`.
        let argv = std::fs::read_to_string(&argv_path).expect("argv");
        assert_eq!(
            argv,
            "-i\n-f\nhello@apex-transactional.example\n--\nops@example.com\n"
        );

        // The message arrives whole: the documented headers, the transport
        // headers the sink adds, and both body parts.
        let body = std::fs::read_to_string(&body_path).expect("body");
        assert!(
            body.starts_with("From: Apex <hello@apex-transactional.example>\r\n"),
            "{body}"
        );
        assert!(body.contains("\r\nTo: ops@example.com\r\n"));
        assert!(body.contains("\r\nSubject: An Apex test was stopped\r\n"));
        assert!(body.contains("\r\nDate: "));
        assert!(body.contains("\r\nMessage-ID: <apex."));
        assert!(body.contains("@apex-transactional.example>\r\n"));
        assert!(body.contains("\r\nMIME-Version: 1.0\r\n"));
        assert!(body.contains("\r\nContent-Type: multipart/alternative; boundary=\"apex-notify-"));
        let plain = body
            .find("Content-Type: text/plain; charset=utf-8")
            .expect("plain part");
        let html = body
            .find("Content-Type: text/html; charset=utf-8")
            .expect("html part");
        assert!(plain < html, "the plain-text twin comes first");
        assert!(body.contains(&message.text.replace('\n', "\r\n").trim_end().to_string()));
        assert!(body.contains(&message.html.replace('\n', "\r\n").trim_end().to_string()));
        assert!(body.ends_with("--\r\n"));
        assert!(
            !body.replace("\r\n", "").contains('\n'),
            "CRLF line endings only"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn email_exit_codes_map_to_states_and_a_missing_mta_is_typed() {
        let sink_for = |command: &str| EmailSink::new(Some("a@x.example"), command).unwrap();
        // A missing executable is a typed failure, never a panic.
        let missing = sink_for("/nonexistent/sendmail");
        let error = send_settled(&missing, &sample())
            .await
            .expect_err("missing binary");
        assert!(matches!(error, SinkError::Mail(_)), "{error:?}");

        // Exit 0 is acceptance.
        let ok = sink_for("true");
        assert_eq!(
            send_settled(&ok, &sample()).await.unwrap().state,
            DeliveryState::Accepted
        );

        // A nonzero exit is a typed failure with a usable display.
        let fails = sink_for("false");
        let error = send_settled(&fails, &sample()).await.expect_err("exit 1");
        assert!(matches!(error, SinkError::Mail(_)), "{error:?}");
        assert!(
            error.to_string().contains("mail transport failure"),
            "{error}"
        );
        assert!(error.to_string().contains("exit status"), "{error}");

        // `true` exits without reading: a message larger than the pipe buffer
        // cannot be handed over, and that is an error, never a silent drop.
        let big = DeliveryMessage {
            text: "x".repeat(4 << 20),
            ..sample()
        };
        assert!(send_settled(&ok, &big).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn email_failure_records_requested_then_failed_with_the_mta_excerpt() {
        let dir = email_temp_dir("failed");
        let stderr_path = dir.join("mta-error.txt");
        std::fs::write(&stderr_path, "smtp: 550 5.1.1 relay denied\n").unwrap();
        let script = format!("cat '{}' >&2\nexit 75", stderr_path.display());
        let sink = Arc::new(
            EmailSink::new(Some("a@x.example"), fake_sendmail(&dir, "fail.sh", &script)).unwrap(),
        );
        // Direct: a nonzero exit carries the MTA's stderr excerpt.
        let direct = send_settled(&sink, &sample()).await.expect_err("exit 75");
        match &direct {
            SinkError::Mail(detail) => assert_eq!(detail, "smtp: 550 5.1.1 relay denied"),
            other => panic!("unexpected {other:?}"),
        }

        // Through the notifier: `requested` then `failed`, exactly.
        let log_path = dir.join("delivery.jsonl");
        let log = DeliveryLog::open(Some(&log_path)).expect("log");
        let notifier = Notifier::new(sink.clone(), log, Arc::new(Registry::new()));
        let contacts = [Contact::new("ops@example.com").unwrap()];
        let report = notifier
            .send_message(&contacts, &template_message(), 9_000)
            .await;
        assert_eq!(report.failed, 1);
        assert_eq!(report.accepted, 0);
        assert_eq!(report.attempted, 1);
        let text = std::fs::read_to_string(&log_path).expect("rows");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            vec![
                r#"{"at_unix_ms":9000,"template_class":"guardrail_stop","dedup_key":"guardrail_stop:exp_0242","to":"ops@example.com","state":"requested"}"#,
                r#"{"at_unix_ms":9000,"template_class":"guardrail_stop","dedup_key":"guardrail_stop:exp_0242","to":"ops@example.com","state":"failed","detail":"mail transport failure: smtp: 550 5.1.1 relay denied"}"#,
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn email_acceptance_records_requested_then_accepted_and_dedupes() {
        let dir = email_temp_dir("accepted");
        let calls_path = dir.join("calls.txt");
        let script = format!(
            "echo CALL >> '{}'\ncat > /dev/null\nexit 0",
            calls_path.display()
        );
        let sink = Arc::new(
            EmailSink::new(
                Some("Apex <hello@apex-transactional.example>"),
                fake_sendmail(&dir, "ok.sh", &script),
            )
            .unwrap(),
        );
        let log_path = dir.join("delivery.jsonl");
        let log = DeliveryLog::open(Some(&log_path)).expect("log");
        let notifier = Notifier::new(sink.clone(), log, Arc::new(Registry::new()));
        let contacts = [Contact::new("ops@example.com").unwrap()];
        let report = notifier
            .send_message(&contacts, &template_message(), 7_000)
            .await;
        assert_eq!(
            report,
            NotifyReport {
                attempted: 1,
                accepted: 1,
                delivered: 0,
                failed: 0,
                duplicates: 0,
                state_errors: 0,
            }
        );
        // Exactly two rows: `requested` then `accepted`. A local MTA never
        // supplies `delivered`, so `accepted` is the terminal state here.
        let text = std::fs::read_to_string(&log_path).expect("rows");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            vec![
                r#"{"at_unix_ms":7000,"template_class":"guardrail_stop","dedup_key":"guardrail_stop:exp_0242","to":"ops@example.com","state":"requested"}"#,
                r#"{"at_unix_ms":7000,"template_class":"guardrail_stop","dedup_key":"guardrail_stop:exp_0242","to":"ops@example.com","state":"accepted"}"#,
            ]
        );
        // The same dedup_key never sends twice: no new MTA invocation.
        let again = notifier
            .send_message(&contacts, &template_message(), 7_001)
            .await;
        assert_eq!(again.duplicates, 1);
        assert_eq!(again.attempted, 0);
        let calls = std::fs::read_to_string(&calls_path).expect("calls");
        assert_eq!(calls.lines().filter(|line| *line == "CALL").count(), 1);
        assert_eq!(
            std::fs::read_to_string(&log_path)
                .expect("rows")
                .lines()
                .count(),
            2,
            "a duplicate appends nothing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_email_sink_keeps_one_sendmail_process_per_recipient() {
        // Measurement for the batch work: email stays as-is because one
        // sendmail invocation with several recipients returns one exit
        // status, so a per-recipient failure would not be attributable.
        let dir = email_temp_dir("spawns");
        let calls_path = dir.join("calls.txt");
        let script = format!(
            "printf '%s\\n' \"$@\" >> '{}'\ncat > /dev/null\nexit 0",
            calls_path.display()
        );
        let sink = Arc::new(
            EmailSink::new(
                Some("Apex <hello@apex-transactional.example>"),
                fake_sendmail(&dir, "ok.sh", &script),
            )
            .unwrap(),
        );
        let log = DeliveryLog::open(Some(&dir.join("delivery.jsonl"))).expect("log");
        let notifier = Notifier::new(sink, log, Arc::new(Registry::new()));
        let contacts = [
            Contact::new("ops@example.com").unwrap(),
            Contact::new("fin@example.com").unwrap(),
        ];
        let report = notifier
            .send_message(&contacts, &template_message(), 7_000)
            .await;
        assert_eq!(report.accepted, 2);
        let calls = std::fs::read_to_string(&calls_path).expect("calls");
        // Each spawn writes three argv lines (`-i`, `-f <from>`, `--`) plus
        // the recipient; two recipients therefore mean two processes.
        let recipients = calls
            .lines()
            .filter(|line| *line == "ops@example.com" || *line == "fin@example.com")
            .collect::<Vec<_>>();
        assert_eq!(
            recipients,
            vec!["ops@example.com", "fin@example.com"],
            "one sendmail process per recipient keeps failures attributable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_mta_stderr_excerpt_is_truncated_on_character_boundaries() {
        let dir = email_temp_dir("trunc");
        let stderr_path = dir.join("stderr.txt");
        // Character 300 is the multibyte 'é': a byte truncation would split
        // it; the excerpt must end with it whole.
        std::fs::write(
            &stderr_path,
            format!("{}é{}", "a".repeat(299), "z".repeat(10)),
        )
        .unwrap();
        let script = format!("cat '{}' >&2\nexit 1", stderr_path.display());
        let sink = Arc::new(
            EmailSink::new(
                Some("a@x.example"),
                fake_sendmail(&dir, "trunc.sh", &script),
            )
            .unwrap(),
        );
        let error = send_settled(&sink, &sample()).await.expect_err("exit 1");
        match error {
            SinkError::Mail(detail) => {
                assert_eq!(detail.chars().count(), 300);
                assert_eq!(detail, format!("{}é", "a".repeat(299)));
            }
            other => panic!("unexpected {other:?}"),
        }
        // Through the notifier the row detail is bounded again (200 chars)
        // without panicking on the multibyte boundary.
        let log_path = dir.join("delivery.jsonl");
        let log = DeliveryLog::open(Some(&log_path)).expect("log");
        let notifier = Notifier::new(sink.clone(), log, Arc::new(Registry::new()));
        let contacts = [Contact::new("ops@example.com").unwrap()];
        let report = notifier
            .send_message(&contacts, &template_message(), 11)
            .await;
        assert_eq!(report.failed, 1);
        let rows = DeliveryLog::read_rows(&log_path).expect("rows");
        assert_eq!(rows[0].state, "requested");
        assert_eq!(rows[1].state, "failed");
        let detail = rows[1].detail.as_deref().unwrap();
        assert_eq!(detail.chars().count(), 200);
        assert!(detail.starts_with("mail transport failure: "));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn no_secrets_ride_in_the_email_message() {
        let dir = email_temp_dir("secrets");
        let argv_path = dir.join("argv.txt");
        let body_path = dir.join("body.eml");
        let script = format!(
            "printf '%s\\n' \"$@\" > '{}'\ncat > '{}'\nexit 0",
            argv_path.display(),
            body_path.display()
        );
        let sink = EmailSink::new(
            Some("Apex <hello@apex-transactional.example>"),
            fake_sendmail(&dir, "ok.sh", &script),
        )
        .unwrap();
        let template = crate::template::payment_failed(&crate::template::PaymentFailed {
            invoice_reference: "INV-2026-104".into(),
            amount_micros: 300_000_000,
            amount_currency: Some("USD".into()),
            provider_event_key: "psp_evt_88_secret".into(),
            action_url: "https://console.apexmediation.ee/invoices/INV-2026-104".into(),
        })
        .unwrap();
        let message = DeliveryMessage::from_message(&template, "ops@example.com");
        send_settled(&sink, &message).await.expect("send");
        let body = std::fs::read_to_string(&body_path).expect("body");
        let argv = std::fs::read_to_string(&argv_path).expect("argv");
        assert!(body.contains("Payment needs attention for invoice INV-2026-104"));
        for secret in [
            "psp_evt_88_secret",
            "s3cr3t-token",
            "APEX_NOTIFY_WEBHOOK_TOKEN",
        ] {
            assert!(!body.contains(secret), "{secret} leaked into the message");
            assert!(!argv.contains(secret), "{secret} leaked into the argv");
        }
        // The provider event key rides only in the dedup identity, and the
        // message carries only its hash (the Message-ID), never the raw key.
        assert!(
            !body.contains(&message.dedup_key),
            "the raw dedup identity never rides"
        );
        assert!(body.contains("Message-ID: <apex."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn select_sink_adds_email_without_changing_existing_selections() {
        let from = "Apex <hello@apex-transactional.example>";
        // email: selected explicitly, with the executable override honored.
        let sink = select_sink(|name| match name {
            SINK_ENV => Some("email".into()),
            FROM_ENV => Some(from.into()),
            SENDMAIL_ENV => Some("/opt/fake/sendmail".into()),
            _ => None,
        })
        .expect("configured")
        .expect("some");
        let debug = format!("{sink:?}");
        assert!(debug.contains("EmailSink"), "{debug}");
        assert!(debug.contains("/opt/fake/sendmail"), "{debug}");

        // email without a sender is a typed refusal, never a fallback.
        let error = select_sink(|name| match name {
            SINK_ENV => Some("email".into()),
            _ => None,
        })
        .expect_err("no from");
        assert!(matches!(error, SinkError::Config(_)), "{error:?}");
        assert!(error.to_string().contains(FROM_ENV), "{error}");

        // Unknown values fail closed even when a legacy sink is configured.
        let error = select_sink(|name| match name {
            SINK_ENV => Some("carrier-pigeon".into()),
            WEBHOOK_ENV => Some("https://hooks.example.test/notify".into()),
            _ => None,
        })
        .expect_err("unknown");
        assert!(error.to_string().contains("carrier-pigeon"), "{error}");

        // Explicit webhook/file require their variables.
        let error = select_sink(|name| match name {
            SINK_ENV => Some("webhook".into()),
            _ => None,
        })
        .expect_err("no webhook url");
        assert!(matches!(error, SinkError::Config(_)), "{error:?}");
        let error = select_sink(|name| match name {
            SINK_ENV => Some("file".into()),
            _ => None,
        })
        .expect_err("no file");
        assert!(matches!(error, SinkError::Config(_)), "{error:?}");
        let sink = select_sink(|name| match name {
            SINK_ENV => Some("file".into()),
            FILE_ENV => Some("/tmp/apex-notify-selection.jsonl".into()),
            _ => None,
        })
        .expect("file")
        .expect("some");
        assert!(format!("{sink:?}").contains("FileSink"));

        // Unset: the legacy order is untouched — webhook wins over file.
        let sink = select_sink(|name| match name {
            WEBHOOK_ENV => Some("https://hooks.example.test/notify".into()),
            FILE_ENV => Some("/tmp/apex-notify-selection.jsonl".into()),
            _ => None,
        })
        .expect("legacy")
        .expect("some");
        assert!(format!("{sink:?}").contains("WebhookSink"));
        let sink = select_sink(|name| match name {
            FILE_ENV => Some("/tmp/apex-notify-selection.jsonl".into()),
            _ => None,
        })
        .expect("file only")
        .expect("some");
        assert!(format!("{sink:?}").contains("FileSink"));
        assert!(select_sink(|_| None).expect("unset").is_none());
    }
}
