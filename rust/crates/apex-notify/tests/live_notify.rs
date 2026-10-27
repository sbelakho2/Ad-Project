//! LIVE notification verification against the real local MTA.
//!
//! Run with `APEX_LIVE_NOTIFY=1` on a host where `/usr/sbin/sendmail` delivers
//! to `/var/mail/<user>` (postfix on this Mac). The test sends every frozen
//! template through the real `EmailSink`, then reads the delivered mailbox and
//! matches each message by the real `Message-ID` the composer derives from the
//! dedup identity. It also proves the header-injection refusals, duplicate
//! suppression, retry after a real transport failure, the real file sink and a
//! real external webhook.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use apex_metrics::Registry;
use apex_notify::contacts::Contact;
use apex_notify::sink::{
    DeliveryMessage, EmailSink, FileSink, NotificationSink, SinkError, WebhookSink,
};
use apex_notify::state::DeliveryLog;
use apex_notify::template::{
    self, approval_requested, audit_ready, fee_preview, guardrail_stop, payment_failed,
    payment_received, payment_retry, proof_hold, reconnection_requested, reporting_access,
    restore_failed, ApprovalRequested, AuditReady, FeePreview, GuardrailStop, IncidentBlock,
    PaymentFailed, PaymentReceived, PaymentRetry, ProofHold, ReconnectionRequested,
    ReportingAccess, RestoreFailed,
};
use apex_notify::{Message, Notifier};

const FROM: &str = "Apex Live <apex-live@apex-transactional.example>";
const FROM_DOMAIN: &str = "apex-transactional.example";

fn live_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!(
        "apex-notify-live-{}-{nanos}-{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("live dir");
    dir
}

fn current_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .expect("USER")
}

fn mailbox_path() -> PathBuf {
    Path::new("/var/mail").join(current_user())
}

fn recipient() -> Contact {
    Contact::new(&format!("{}@localhost.localdomain", current_user())).expect("local recipient")
}

/// The exact FNV-1a 64-bit hash the composer uses for its Message-ID and MIME
/// boundary (pinned in `mail.rs`).
fn fnv1a64(data: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn message_id(message: &Message) -> String {
    format!("<apex.{:016x}@{FROM_DOMAIN}>", fnv1a64(&message.dedup_key))
}

fn mailbox_count(message: &Message) -> usize {
    std::fs::read_to_string(mailbox_path())
        .unwrap_or_default()
        .matches(&message_id(message))
        .count()
}

fn wait_for_more(message: &Message, before: usize) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if mailbox_count(message) > before {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

fn every_template() -> Vec<Message> {
    let url = "https://console.apexmediation.ee/live";
    let mut messages = vec![fee_preview(&FeePreview {
        month: "2026-10".into(),
        amount_micros: 12_345_000,
        draft_reference: "draft-live-fee-preview".into(),
        action_url: url.into(),
    })
    .expect("fee preview")];
    messages.push(
        payment_retry(&PaymentRetry {
            invoice_reference: "INV-2026-10-live-retry".into(),
            balance_micros: 3_000_000,
            balance_currency: Some("EUR".into()),
            retry_day: Some("2026-10-28".into()),
            action_url: url.into(),
        })
        .expect("payment retry"),
    );
    messages.push(
        payment_failed(&PaymentFailed {
            invoice_reference: "INV-2026-10-live-failed".into(),
            amount_micros: 4_500_000,
            amount_currency: Some("USD".into()),
            provider_event_key: "psp-live-failed".into(),
            action_url: url.into(),
        })
        .expect("payment failed"),
    );
    messages.push(
        payment_received(&PaymentReceived {
            invoice_reference: "INV-2026-10-live-received".into(),
            received_micros: 1_500_000,
            remaining_micros: 1_500_000,
            amount_currency: Some("USD".into()),
            applied_day: "2026-10-19".into(),
            provider_event_key: "psp-live-received".into(),
            action_url: url.into(),
        })
        .expect("payment received"),
    );
    messages.push(
        audit_ready(&AuditReady {
            platform_label: "MAX".into(),
            snapshot_day: "2026-10-19".into(),
            action_url: url.into(),
        })
        .expect("audit ready"),
    );
    messages.push(
        reporting_access(&ReportingAccess {
            platform_label: "MAX".into(),
            publisher_label: "Northstar".into(),
            day: "2026-10-19".into(),
            action_url: url.into(),
        })
        .expect("reporting access"),
    );
    messages.push(
        reconnection_requested(&ReconnectionRequested {
            platform_label: "LevelPlay".into(),
            publisher_label: "Northstar".into(),
            access_label: "reporting access".into(),
            day: "2026-10-19".into(),
            action_url: url.into(),
        })
        .expect("reconnection"),
    );
    messages.push(
        guardrail_stop(&GuardrailStop {
            experiment_id: "exp_live_guardrail".into(),
            action_url: url.into(),
        })
        .expect("guardrail stop"),
    );
    messages.push(
        restore_failed(&RestoreFailed {
            experiment_id: "exp_live_restore".into(),
            incident_id: "inc_live_restore".into(),
            incident: IncidentBlock {
                what_is_known: "The stop request has not been confirmed.".into(),
                who_is_affected: "One experiment on the live account.".into(),
                what_we_are_doing: "Reconciling the platform state.".into(),
                next_update: "Today, 18:00 UTC.".into(),
            },
            action_url: url.into(),
        })
        .expect("restore failed"),
    );
    messages.push(
        proof_hold(&ProofHold {
            draft_reference: "draft-live-proof-hold".into(),
            action_url: url.into(),
        })
        .expect("proof hold"),
    );
    messages.push(
        approval_requested(&ApprovalRequested {
            proposal_id: "proposal-live-approval".into(),
            test_label: "Rewarded video".into(),
            country: "Germany".into(),
            action_url: url.into(),
        })
        .expect("approval requested"),
    );
    messages
}

#[tokio::test(flavor = "multi_thread")]
async fn every_template_is_delivered_by_the_real_local_mta() {
    if std::env::var("APEX_LIVE_NOTIFY").ok().as_deref() != Some("1") {
        println!("SKIP: set APEX_LIVE_NOTIFY=1 to deliver through /usr/sbin/sendmail");
        return;
    }
    let dir = live_dir("templates");
    let state = dir.join("delivery.jsonl");
    let sink = Arc::new(EmailSink::new(Some(FROM), "/usr/sbin/sendmail").expect("sink"));
    let notifier = Notifier::new(
        sink,
        DeliveryLog::open(Some(&state)).expect("state"),
        Arc::new(Registry::new()),
    );
    let contacts = [recipient()];
    let messages = every_template();
    assert_eq!(messages.len(), 11, "every frozen template is exercised");
    let mut delivered = Vec::new();
    for message in &messages {
        let before = mailbox_count(message);
        let report = notifier
            .send_message(&contacts, message, 1_792_400_000_000)
            .await;
        assert_eq!(
            report.failed,
            0,
            "{}: {report:?}",
            message.template_class.as_str()
        );
        assert_eq!(
            report.accepted,
            1,
            "{}: {report:?}",
            message.template_class.as_str()
        );
        assert!(
            wait_for_more(message, before),
            "mailbox never received {message:?}"
        );
        assert_eq!(
            mailbox_count(message),
            before + 1,
            "expected exactly one new delivered copy of {}",
            message_id(message)
        );
        delivered.push(message.template_class.as_str().to_string());
    }
    let rows = DeliveryLog::read_rows(&state).expect("rows");
    assert_eq!(rows.len(), 22, "requested + accepted per message");
    println!(
        "LIVE notify: {} templates delivered to {}; Message-IDs verified in {}",
        delivered.len(),
        recipient().email(),
        mailbox_path().display()
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&state)
        .expect("state metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "delivery state mode {mode:o}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn dedup_retry_and_state_rows_are_real() {
    if std::env::var("APEX_LIVE_NOTIFY").ok().as_deref() != Some("1") {
        println!("SKIP: set APEX_LIVE_NOTIFY=1 for the real MTA");
        return;
    }
    let dir = live_dir("dedup");
    let state = dir.join("delivery.jsonl");
    let message = payment_failed(&PaymentFailed {
        invoice_reference: "INV-2026-10-live-dedup".into(),
        amount_micros: 1_000_000,
        amount_currency: Some("USD".into()),
        provider_event_key: "psp-live-dedup".into(),
        action_url: "https://console.apexmediation.ee/live".into(),
    })
    .expect("message");
    let contacts = [recipient()];
    let registry = Arc::new(Registry::new());

    // A real transport failure first: the executable does not exist. The
    // notifier records requested + failed and keeps the message eligible.
    let broken = Notifier::new(
        Arc::new(
            EmailSink::new(Some(FROM), "/nonexistent/sendmail-live")
                .expect("sink construction does not stat the binary"),
        ),
        DeliveryLog::open(Some(&state)).expect("state"),
        Arc::clone(&registry),
    );
    let failed = broken
        .send_message(&contacts, &message, 1_792_400_000_000)
        .await;
    assert_eq!(failed.failed, 1, "{failed:?}");
    assert_eq!(failed.accepted, 0);

    // Retry with the real MTA and the same state file: a failed attempt does
    // not deduplicate, so the message is sent.
    let good = Notifier::new(
        Arc::new(EmailSink::new(Some(FROM), "/usr/sbin/sendmail").expect("sink")),
        DeliveryLog::open(Some(&state)).expect("state"),
        Arc::clone(&registry),
    );
    let before = mailbox_count(&message);
    let accepted = good
        .send_message(&contacts, &message, 1_792_400_000_001)
        .await;
    assert_eq!(accepted.accepted, 1, "{accepted:?}");
    assert!(wait_for_more(&message, before), "retry was not delivered");

    // The same (template, dedup key, recipient) is now a duplicate: nothing
    // is sent and the mailbox holds exactly the one new copy.
    let duplicate = good
        .send_message(&contacts, &message, 1_792_400_000_002)
        .await;
    assert_eq!(duplicate.duplicates, 1, "{duplicate:?}");
    assert_eq!(duplicate.attempted, 0, "{duplicate:?}");
    assert_eq!(mailbox_count(&message), before + 1);

    let rows = DeliveryLog::read_rows(&state).expect("rows");
    let states: Vec<&str> = rows.iter().map(|row| row.state.as_str()).collect();
    assert_eq!(
        states,
        vec!["requested", "failed", "requested", "accepted"],
        "unexpected delivery rows: {rows:?}"
    );
    println!("LIVE notify dedup/retry: {states:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn header_injection_is_refused_and_never_delivered() {
    if std::env::var("APEX_LIVE_NOTIFY").ok().as_deref() != Some("1") {
        println!("SKIP: set APEX_LIVE_NOTIFY=1 for the real MTA");
        return;
    }
    // Addresses carrying CRLF or a leading option dash never reach sendmail.
    assert!(Contact::new("ops@example.com\r\nBcc: attacker@example.com").is_err());
    assert!(Contact::new("ops@example.com\nBcc: attacker@example.com").is_err());
    let option_like =
        Contact::new("-fapex@example.com").expect("the contact parser accepts the address shape");
    let sink = EmailSink::new(Some(FROM), "/usr/sbin/sendmail").expect("sink");
    let option_payload = DeliveryMessage {
        to: option_like.email().to_string(),
        subject: "Live option refusal".into(),
        html: "<p>live</p>".into(),
        text: "live".into(),
        template_class: apex_metrics::vocab::TemplateClass::PaymentFailed,
        dedup_key: "option-like-live".into(),
        reference: None,
        list_unsubscribe: None,
    };
    assert!(matches!(
        sink.send(&option_payload).await,
        Err(SinkError::Mail(_))
    ));

    // Templates that interpolate an identity into the subject/header refuse
    // control characters at the template boundary where the field is a header
    // field, and the email sink refuses any rendered subject that still
    // carries one.
    let injected = "MAX\r\nBcc: attacker@example.com";
    assert!(reporting_access(&ReportingAccess {
        platform_label: injected.into(),
        publisher_label: "Northstar".into(),
        day: "2026-10-19".into(),
        action_url: "https://console.apexmediation.ee/live".into(),
    })
    .is_err());
    assert!(reconnection_requested(&ReconnectionRequested {
        platform_label: "MAX".into(),
        publisher_label: injected.into(),
        access_label: "reporting access".into(),
        day: "2026-10-19".into(),
        action_url: "https://console.apexmediation.ee/live".into(),
    })
    .is_err());
    assert!(EmailSink::new(Some("Apex\r\nBcc: x@example.com"), "/usr/sbin/sendmail").is_err());

    // A rendered message whose subject carries CRLF is refused by the sink
    // before any child process is spawned, and the sentinel never reaches the
    // mailbox.
    let sink = EmailSink::new(Some(FROM), "/usr/sbin/sendmail").expect("sink");
    let payload = DeliveryMessage {
        to: recipient().email().to_string(),
        subject: "Live\r\nBcc: attacker@example.com".into(),
        html: "<p>live</p>".into(),
        text: "live".into(),
        template_class: apex_metrics::vocab::TemplateClass::PaymentFailed,
        dedup_key: "header-injection-live".into(),
        reference: None,
        list_unsubscribe: None,
    };
    let error = sink.send(&payload).await.expect_err("refused");
    assert!(matches!(error, SinkError::Mail(_)), "{error:?}");
    assert!(!std::fs::read_to_string(mailbox_path())
        .unwrap_or_default()
        .contains("Bcc: attacker@example.com"));

    // The subprocess environment is cleared: a secret in the runner's
    // environment never appears in the delivered message.
    std::env::set_var("APEX_LIVE_SECRET_SENTINEL", "vault-secret-live-sentinel");
    let message = payment_received(&PaymentReceived {
        invoice_reference: "INV-2026-10-live-env".into(),
        received_micros: 1_000_000,
        remaining_micros: 0,
        amount_currency: Some("USD".into()),
        applied_day: "2026-10-19".into(),
        provider_event_key: "psp-live-env".into(),
        action_url: "https://console.apexmediation.ee/live".into(),
    })
    .expect("message");
    let notifier = Notifier::new(
        Arc::new(EmailSink::new(Some(FROM), "/usr/sbin/sendmail").expect("sink")),
        DeliveryLog::open(Some(&live_dir("env").join("delivery.jsonl"))).expect("state"),
        Arc::new(Registry::new()),
    );
    let report = notifier
        .send_message(&[recipient()], &message, 1_792_400_000_000)
        .await;
    assert_eq!(report.accepted, 1);
    let before = 0usize;
    assert!(wait_for_more(&message, before));
    let body = std::fs::read_to_string(mailbox_path()).expect("mailbox");
    assert!(
        !body.contains("vault-secret-live-sentinel"),
        "a child MTA inherited the service environment"
    );
    std::env::remove_var("APEX_LIVE_SECRET_SENTINEL");
    println!("LIVE notify injection refusals and subprocess env isolation verified");
}

#[tokio::test(flavor = "multi_thread")]
async fn file_sink_writes_real_json_lines() {
    let dir = live_dir("file");
    let path = dir.join("sink.jsonl");
    let message = template::audit_ready(&AuditReady {
        platform_label: "MAX".into(),
        snapshot_day: "2026-10-19".into(),
        action_url: "https://console.apexmediation.ee/live".into(),
    })
    .expect("message");
    let sink = FileSink::new(&path);
    let contacts = [
        Contact::new("one@example.com").expect("one"),
        Contact::new("two@example.com").expect("two"),
    ];
    let payloads: Vec<DeliveryMessage> = contacts
        .iter()
        .map(|contact| DeliveryMessage::from_message(&message, contact.email()))
        .collect();
    let receipts = sink.send_batch(&payloads).await;
    assert_eq!(receipts.len(), 2);
    assert!(receipts.iter().all(Result::is_ok));
    let raw = std::fs::read_to_string(&path).expect("sink file");
    let rows: Vec<serde_json::Value> = raw
        .lines()
        .map(|line| serde_json::from_str(line).expect("json line"))
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["to"], "one@example.com");
    assert_eq!(rows[1]["to"], "two@example.com");
    assert_eq!(rows[0]["template_class"], "audit_ready");
    assert_eq!(rows[0]["subject"], "Your Apex audit is ready");
    println!(
        "LIVE notify file sink: 2 real JSONL rows at {}",
        path.display()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn webhook_sink_posts_to_the_real_external_httpbin() {
    if std::env::var("APEX_LIVE_NETWORK").ok().as_deref() != Some("1") {
        println!("SKIP: set APEX_LIVE_NETWORK=1 for the real external webhook");
        return;
    }
    let message = template::fee_preview(&FeePreview {
        month: "2026-10".into(),
        amount_micros: 12_345_000,
        draft_reference: "draft-live-webhook".into(),
        action_url: "https://console.apexmediation.ee/live".into(),
    })
    .expect("message");
    let payload = DeliveryMessage::from_message(&message, "ops@example.com");

    // A real POST to https://httpbin.org/post. httpbin is not a notification
    // provider and does not answer with the documented state body, so the
    // sink must report that honestly rather than pretend acceptance.
    let post = WebhookSink::new("https://httpbin.org/post", None).expect("post sink");
    let error = post
        .send(&payload)
        .await
        .expect_err("httpbin /post is not contract-shaped");
    assert!(matches!(error, SinkError::BadResponse(_)), "{error:?}");
    println!("LIVE notify webhook https://httpbin.org/post refused honestly: {error}");

    // The same real external service answering the documented contract:
    // httpbin's response-headers endpoint returns {state, provider_reference}.
    let accepted = WebhookSink::new(
        "https://httpbin.org/response-headers?state=accepted&provider_reference=httpbin-live",
        None,
    )
    .expect("accepted sink");
    let receipt = accepted.send(&payload).await.expect("accepted");
    assert_eq!(receipt.state.as_str(), "accepted");
    assert_eq!(receipt.provider_reference.as_deref(), Some("httpbin-live"));
    println!("LIVE notify webhook accepted by real external httpbin: {receipt:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn state_retention_prunes_only_rows_outside_the_window() {
    let dir = live_dir("retention");
    let state = dir.join("delivery.jsonl");
    let sink = Arc::new(FileSink::new(dir.join("outbox.jsonl")));
    let notifier = Notifier::new(
        sink,
        DeliveryLog::open(Some(&state)).expect("state"),
        Arc::new(Registry::new()),
    );
    let message = payment_failed(&PaymentFailed {
        invoice_reference: "INV-2026-10-live-retention".into(),
        amount_micros: 1_000_000,
        amount_currency: Some("USD".into()),
        provider_event_key: "psp-live-retention".into(),
        action_url: "https://console.apexmediation.ee/live".into(),
    })
    .expect("message");
    let at = 1_792_400_000_000i64;
    let report = notifier.send_message(&[recipient()], &message, at).await;
    assert_eq!(report.accepted, 1, "{report:?}");

    // Inside the window the accepted row still suppresses a resend.
    let inside = DeliveryLog::open_at(Some(&state), at + apex_notify::state::RETENTION_MS - 1)
        .expect("inside");
    let contacts = [recipient()];
    assert!(inside.has_sent_at(
        message.template_class.as_str(),
        &message.dedup_key,
        contacts[0].email(),
        at + apex_notify::state::RETENTION_MS - 1,
    ));

    // Past the window the row is pruned and the file is compacted to empty.
    let outside = DeliveryLog::open_at(Some(&state), at + apex_notify::state::RETENTION_MS + 1)
        .expect("outside");
    assert!(!outside.has_sent_at(
        message.template_class.as_str(),
        &message.dedup_key,
        contacts[0].email(),
        at + apex_notify::state::RETENTION_MS + 1,
    ));
    let rows = DeliveryLog::read_rows(&state).expect("rows");
    assert!(rows.is_empty(), "expired rows were not compacted: {rows:?}");
    println!(
        "LIVE notify retention: accepted row suppressed inside {} ms, pruned and compacted after it",
        apex_notify::state::RETENTION_MS
    );
    let _ = std::fs::remove_dir_all(&dir);
}
