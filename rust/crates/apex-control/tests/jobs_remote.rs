#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The split-host operator-jobs transport, against a fake apex-api endpoint
//! that mirrors the real receiver's contract: POST persists the body bytes
//! verbatim into the jobs folder and deduplicates by `job_id`, GET reads one
//! job back, both gated by the bearer token.
//!
//! The headline assertion is differential: the bytes the remote client sends
//! (and the endpoint stores) are byte-identical to the bytes the local
//! single-host writer produces for the same job, so the runner host's folder
//! cannot tell the two modes apart. The fake endpoint enforces the same
//! bearer check the real route enforces, so the request-level contract
//! (Authorization header, paths, status mapping) is pinned here too.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use apex_control::jobs::{
    JobError, JobKind, JobRead, JobTransport, JobWrite, OperatorJob, RemoteJobsConfig,
    JOBS_ENDPOINT_PATH, JOBS_TOKEN_ENV,
};

const TOKEN: &str = "jobs-token-test";

/// A unique temp directory removed on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "apex-control-jobs-remote-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// One captured request: method, path and authorization header.
#[derive(Debug, Clone)]
struct Captured {
    method: String,
    path: String,
    authorization: Option<String>,
}

/// The fake endpoint state.
#[derive(Debug, Default)]
struct FakeState {
    stored: BTreeMap<String, Vec<u8>>,
    requests: Vec<Captured>,
    forced_status: Option<u16>,
    forced_body: String,
}

/// A fake apex-api jobs endpoint backed by a real folder.
struct FakeEndpoint {
    base: String,
    dir: PathBuf,
    state: Arc<Mutex<FakeState>>,
}

impl FakeEndpoint {
    fn start(dir: PathBuf) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let state = Arc::new(Mutex::new(FakeState::default()));
        let server_state = Arc::clone(&state);
        let server_dir = dir.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let state = Arc::clone(&server_state);
                let dir = server_dir.clone();
                std::thread::spawn(move || serve_one(&mut stream, &state, &dir));
            }
        });
        Self { base, dir, state }
    }

    fn config(&self) -> RemoteJobsConfig {
        RemoteJobsConfig {
            url: self.base.clone(),
            token: TOKEN.to_string(),
        }
    }

    fn requests(&self) -> Vec<Captured> {
        self.state.lock().expect("state").requests.clone()
    }

    fn force_status(&self, status: u16, body: &str) {
        let mut state = self.state.lock().expect("state");
        state.forced_status = Some(status);
        state.forced_body = body.to_string();
    }

    fn clear_forced_status(&self) {
        self.state.lock().expect("state").forced_status = None;
    }

    /// Seed one raw file exactly as it would sit in the shared folder.
    fn seed(&self, job_id: &str, bytes: &[u8]) {
        std::fs::write(self.dir.join(format!("{job_id}.json")), bytes).expect("seed");
        self.state
            .lock()
            .expect("state")
            .stored
            .insert(job_id.to_string(), bytes.to_vec());
    }

    /// Simulate the runner claiming/completing the job in the shared folder.
    fn overwrite(&self, job: &OperatorJob, mutate: impl FnOnce(&mut OperatorJob)) {
        let mut updated = job.clone();
        mutate(&mut updated);
        let body = serde_json::to_vec_pretty(&updated).expect("json");
        std::fs::write(self.dir.join(format!("{}.json", updated.job_id)), &body).expect("write");
        self.state
            .lock()
            .expect("state")
            .stored
            .insert(updated.job_id.clone(), body);
    }
}

fn serve_one(
    stream: &mut std::net::TcpStream,
    state: &Arc<Mutex<FakeState>>,
    dir: &std::path::Path,
) {
    let mut raw = Vec::new();
    let mut buffer = [0u8; 4096];
    let (head_end, content_length) = loop {
        let read = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        raw.extend_from_slice(&buffer[..read]);
        let Some(head_end) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
        let content_length = head
            .lines()
            .find_map(|line| {
                let lower = line.to_ascii_lowercase();
                lower
                    .strip_prefix("content-length:")
                    .map(|value| value.trim().parse::<usize>().unwrap_or(0))
            })
            .unwrap_or(0);
        if raw.len() >= head_end + 4 + content_length {
            break (head_end, content_length);
        }
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let authorization = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("authorization")
            .then(|| value.trim().to_string())
    });
    let body = raw[head_end + 4..head_end + 4 + content_length].to_vec();

    let (status, response_body) = {
        let mut state = state.lock().expect("state");
        state.requests.push(Captured {
            method: method.clone(),
            path: path.clone(),
            authorization: authorization.clone(),
        });
        if let Some(status) = state.forced_status {
            (status, state.forced_body.clone().into_bytes())
        } else if authorization.as_deref() != Some(&format!("Bearer {TOKEN}")) {
            (401, br#"{"error":"unauthorized"}"#.to_vec())
        } else if method == "POST" && path == JOBS_ENDPOINT_PATH {
            match serde_json::from_slice::<OperatorJob>(&body) {
                Err(error) => (
                    400,
                    format!("{{\"error\":\"bad_request\",\"detail\":\"{error}\"}}").into_bytes(),
                ),
                // The real endpoint: a duplicate job_id returns the existing
                // bytes and rewrites nothing.
                Ok(job) => match state.stored.get(&job.job_id) {
                    Some(existing) => (200, existing.clone()),
                    None => {
                        std::fs::write(dir.join(format!("{}.json", job.job_id)), &body)
                            .expect("store");
                        state.stored.insert(job.job_id.clone(), body.clone());
                        (200, body)
                    }
                },
            }
        } else if method == "GET" {
            let job_id = path
                .strip_prefix(JOBS_ENDPOINT_PATH)
                .and_then(|rest| rest.strip_prefix('/'));
            match job_id.and_then(|id| state.stored.get(id)) {
                Some(existing) => (200, existing.clone()),
                None => (404, br#"{"error":"not_found"}"#.to_vec()),
            }
        } else {
            (405, br#"{"error":"method_not_allowed"}"#.to_vec())
        }
    };
    let response = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response_body.len()
    );
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(&response_body);
    let _ = stream.write_all(&bytes);
}

/// The deterministic job both modes write: the id is pinned so the byte
/// comparison and the replays are exact.
fn a_job(job_id: &str) -> OperatorJob {
    let mut job = OperatorJob::queued(
        JobKind::RetryRead,
        "maya",
        1_792_000_000_000,
        "northstar:applovin_max:acct_northstar_max_01",
        serde_json::json!({"platform": "applovin_max", "account": "acct_northstar_max_01"}),
        "0123456789abcdef0123456789abcdef",
        "corr-remote-1",
    );
    job.job_id = job_id.to_string();
    job
}

fn frozen_bytes(job: &OperatorJob) -> Vec<u8> {
    serde_json::to_vec_pretty(job).expect("frozen bytes")
}

#[tokio::test]
async fn the_remote_transport_writes_the_exact_local_bytes_and_reads_them_back() {
    let local_dir = TempDir::new("local");
    let remote_dir = TempDir::new("remote");
    let fake = FakeEndpoint::start(remote_dir.path.clone());
    let job = a_job("opj_0000000000f1");

    // Single-host mode: the local folder writer, exactly as before the split.
    let local = JobTransport::local(local_dir.path.clone());
    let local_write = local.write(&job).await.expect("local write");
    assert!(local_write.created());
    let local_bytes =
        std::fs::read(local_dir.path.join(format!("{}.json", job.job_id))).expect("local file");
    assert_eq!(local_bytes, frozen_bytes(&job));

    // Split-host mode: the same job through the endpoint.
    let remote = JobTransport::remote(&fake.config()).expect("remote transport");
    let remote_write = remote.write(&job).await.expect("remote write");
    assert_eq!(remote_write.job(), &job);
    let remote_bytes =
        std::fs::read(remote_dir.path.join(format!("{}.json", job.job_id))).expect("remote file");
    assert_eq!(
        remote_bytes, local_bytes,
        "the endpoint stores the exact bytes the local writer would have written"
    );

    // The read-back is the same job in both modes.
    let local_read = local.read(&job.job_id).await.expect("local read");
    let remote_read = remote.read(&job.job_id).await.expect("remote read");
    match (local_read, remote_read) {
        (JobRead::Parsed(local_job), JobRead::Parsed(remote_job)) => {
            assert_eq!(*local_job, *remote_job);
            assert_eq!(*remote_job, job);
        }
        other => panic!("expected two parsed jobs, got {other:?}"),
    }

    // The requests were bearer-authenticated GET/POSTs on the frozen path.
    let requests = fake.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    let expected_authorization = format!("Bearer {TOKEN}");
    for request in &requests {
        assert_eq!(
            request.authorization.as_deref(),
            Some(expected_authorization.as_str())
        );
    }
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, JOBS_ENDPOINT_PATH);
    assert_eq!(requests[1].method, "GET");
    assert_eq!(
        requests[1].path,
        format!("{JOBS_ENDPOINT_PATH}/{}", job.job_id)
    );
}

#[tokio::test]
async fn a_duplicate_job_id_returns_the_existing_state_and_never_rewrites() {
    let remote_dir = TempDir::new("duplicate");
    let fake = FakeEndpoint::start(remote_dir.path.clone());
    let remote = JobTransport::remote(&fake.config()).expect("remote transport");
    let job = a_job("opj_0000000000f2");
    let write = remote.write(&job).await.expect("first write");
    assert!(write.created());
    let before =
        std::fs::read(remote_dir.path.join(format!("{}.json", job.job_id))).expect("stored file");

    // The runner claims the job in the shared folder; a replay of the same
    // POST returns the existing record (Duplicate), not a rewrite.
    fake.overwrite(&job, |stored| {
        stored.state = "completed".to_string();
        stored
            .state_history
            .push(apex_control::jobs::JobStateEntry {
                state: "completed".to_string(),
                at_ms: 1_792_000_100_000,
                note: "claimed by the northstar runner pass".to_string(),
            });
        stored.outcome = Some("state re-read: Running".to_string());
    });
    let after_mutation =
        std::fs::read(remote_dir.path.join(format!("{}.json", job.job_id))).expect("stored file");
    let replayed = remote.write(&job).await.expect("duplicate write");
    match replayed {
        JobWrite::Duplicate(existing) => {
            assert_eq!(existing.state, "completed");
            assert_eq!(existing.state_history.len(), 2);
        }
        other => panic!("expected the existing job, got {other:?}"),
    }
    assert_eq!(
        std::fs::read(remote_dir.path.join(format!("{}.json", job.job_id))).expect("stored file"),
        after_mutation,
        "the duplicate POST rewrote nothing"
    );
    assert_ne!(before, after_mutation);
}

#[tokio::test]
async fn endpoint_refusals_map_to_typed_errors() {
    let remote_dir = TempDir::new("refusals");
    let fake = FakeEndpoint::start(remote_dir.path.clone());
    let remote = JobTransport::remote(&fake.config()).expect("remote transport");
    let job = a_job("opj_0000000000f3");

    // 401 and 503 from the endpoint are write failures with the status named.
    for status in [401u16, 503] {
        fake.force_status(status, "{\"error\":\"forbidden\"}");
        let error = remote.write(&job).await.expect_err("refused");
        assert!(
            matches!(&error, JobError::Remote { .. }),
            "{status}: {error:?}"
        );
        assert!(error.to_string().contains(&status.to_string()), "{error}");
        std::fs::remove_file(remote_dir.path.join(format!("{}.json", job.job_id))).ok();
    }
    // A 200 that echoes nothing useful is a typed parse failure, not a panic.
    fake.force_status(200, "");
    let error = remote.write(&job).await.expect_err("empty 200 body");
    assert!(error.to_string().contains("did not parse"), "{error}");

    // 404 on GET is the endpoint's honest absence.
    fake.clear_forced_status();
    let error = remote.read("opj_aaaaaaaaaaaa").await.expect_err("absent");
    assert!(error.to_string().contains("no record"), "{error}");

    // A transport failure (nothing listening) is a typed error too.
    let closed = JobTransport::remote(&RemoteJobsConfig {
        url: "http://127.0.0.1:1".to_string(),
        token: TOKEN.to_string(),
    })
    .expect("transport");
    let error = closed.write(&job).await.expect_err("closed port");
    assert!(matches!(error, JobError::Remote { .. }), "{error:?}");
}

#[tokio::test]
async fn a_malformed_remote_job_reads_as_state_unknown() {
    let remote_dir = TempDir::new("malformed");
    let fake = FakeEndpoint::start(remote_dir.path.clone());
    let remote = JobTransport::remote(&fake.config()).expect("remote transport");
    fake.seed("opj_0000000000f4", b"{ not json");
    let read = remote.read("opj_0000000000f4").await.expect("read");
    match read {
        JobRead::StateUnknown { file, reason } => {
            assert_eq!(file, "opj_0000000000f4.json");
            assert!(reason.contains("did not parse"), "{reason}");
        }
        other => panic!("expected State unknown, got {other:?}"),
    }
}

#[test]
fn the_token_environment_variable_is_the_shared_contract() {
    assert_eq!(JOBS_TOKEN_ENV, "APEX_OPERATOR_JOBS_TOKEN");
    assert_eq!(JOBS_ENDPOINT_PATH, "/v1/operator/jobs");
}
