//! End-to-end checks of the `apex-ingest` binary in `--check` mode: stdin
//! and file inputs, the JSON summary, exit codes, and the guarantee that
//! nothing needs a store (no network) when checking. The rows the binary
//! writes, the spool and the replay are covered by the binary's own tests
//! over a `MemoryStore` and by the real-server integration in
//! `tests/clickhouse.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run_check(args: &[&str], input: &str) -> (i32, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apex-ingest"));
    command.args(args);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn apex-ingest");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

fn request_line(uri: &str, status: u16, user_agent: &str) -> String {
    serde_json::json!({
        "ts": 1_767_225_600.0_f64,
        "status": status,
        "request": {
            "method": "GET",
            "host": "apexmediation.ee",
            "uri": uri,
            "remote_ip": "203.0.113.9",
            "headers": { "User-Agent": [user_agent] }
        }
    })
    .to_string()
}

#[test]
fn check_mode_reports_a_clean_log() {
    let log = format!(
        "{}\n{}\n{}\n",
        request_line("/pricing", 200, "Mozilla/5.0 Safari/605.1.15"),
        request_line("/guides/raise-ecpm?utm_source=news", 200, "Googlebot/2.1"),
        serde_json::json!({"ts": 1.0, "msg": "log rotated"}),
    );
    let (code, stdout) = run_check(&["--check", "--json"], &log);
    assert_eq!(code, 0, "stdout: {stdout}");
    let summary: serde_json::Value = serde_json::from_str(&stdout).expect("json summary");
    assert_eq!(summary["check_only"], true);
    assert_eq!(summary["lines"], 3);
    assert_eq!(summary["request_logs"], 2);
    assert_eq!(summary["skipped_non_request"], 1);
    assert_eq!(summary["rejected"], 0);
    assert_eq!(summary["input"], "stdin");
    // The redaction guarantees hold end to end.
    assert!(!stdout.contains("203.0.113.9"));
}

#[test]
fn check_mode_rejects_malformed_lines_with_exit_three() {
    let log = format!(
        "{}\nnot json at all\n{}\n",
        request_line("/", 200, "curl/8.4.0"),
        serde_json::json!({"ts": 1.0, "status": 200, "request": {"uri": "no-slash"}}),
    );
    let (code, stdout) = run_check(&["--check", "--json"], &log);
    assert_eq!(code, 3);
    let summary: serde_json::Value = serde_json::from_str(&stdout).expect("json summary");
    assert_eq!(summary["rejected"], 2);
    assert_eq!(summary["request_logs"], 1);
    let errors = summary["errors"].as_array().expect("errors");
    assert!(errors.len() >= 2);
    assert!(errors[0].as_str().expect("text").contains("line 2"));
}

#[test]
fn check_mode_reads_a_file_argument() {
    let path = std::env::temp_dir().join(format!("apex-ingest-check-{}.json", std::process::id()));
    std::fs::write(
        &path,
        format!("{}\n", request_line("/proof", 200, "wget/1.21")),
    )
    .expect("write log");
    let (code, stdout) = run_check(
        &["--check", "--json", path.to_str().expect("utf-8 path")],
        "",
    );
    assert_eq!(code, 0, "stdout: {stdout}");
    let summary: serde_json::Value = serde_json::from_str(&stdout).expect("json summary");
    assert_eq!(summary["request_logs"], 1);
    assert!(summary["input"]
        .as_str()
        .expect("input")
        .contains("apex-ingest-check"),);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn human_summary_names_the_counts() {
    let log = format!("{}\n", request_line("/", 404, "Mozilla/5.0"));
    let (code, stdout) = run_check(&["--check"], &log);
    assert_eq!(code, 0);
    assert!(stdout.contains("1 lines"), "{stdout}");
    assert!(stdout.contains("1 request logs"), "{stdout}");
    assert!(stdout.contains("check only"), "{stdout}");
}

#[test]
fn bad_usage_exits_two() {
    let (code, _) = run_check(&["--nonsense"], "");
    assert_eq!(code, 2);
    let (code, _) = run_check(&["--spool"], "");
    assert_eq!(code, 2);
}

#[test]
fn a_store_failure_is_reported_and_exits_two() {
    // Normal mode against a closed port: schema setup fails, the run reports
    // it on stderr and exits 2 without claiming success.
    let log = format!("{}\n", request_line("/", 200, "Mozilla/5.0"));
    let mut command = Command::new(env!("CARGO_BIN_EXE_apex-ingest"));
    command
        .env("APEX_CLICKHOUSE_HTTP", "http://127.0.0.1:1")
        .env("APEX_CLICKHOUSE_DB", "apex_probe")
        .env_remove("APEX_ENVIRONMENT")
        .env_remove("APEX_SERVICE_VERSION")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn apex-ingest");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(log.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("schema setup failed"), "{stderr}");
}
