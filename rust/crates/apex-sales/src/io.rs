//! The engine's two effects on the outside world, behind traits so every
//! caller is testable: fetching public pages and handing mail to a
//! transport.
//!
//! * [`CurlFetcher`] runs the system `curl` with HTTPS-only redirects, a
//!   timeout and a size cap. URLs are validated first: `https://`, a real
//!   hostname (no IP literals, no `localhost`), no whitespace or control
//!   characters. The URL is passed as an argument after `--`, never through a
//!   shell.
//! * [`SendmailMailer`] pipes the message to the local MTA's `sendmail`
//!   interface with the recipient given explicitly (`-i -f <from> --
//!   <to>`), so header contents can never add recipients. Delivery, signing (DKIM),
//!   retries and bounces are the MTA's job.
//! * [`OutboxMailer`] writes `.eml` files instead: the shadow-mode and review
//!   transport.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::compose::Outgoing;

/// A fetched page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// HTTP status.
    pub status: u16,
    /// Body (lossy UTF-8, capped).
    pub body: String,
}

/// Why a fetch failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The URL was refused before any network activity.
    InvalidUrl(String),
    /// The transport failed (DNS, TLS, timeout, ...).
    Transport(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::InvalidUrl(u) => write!(f, "refused URL {u}"),
            FetchError::Transport(e) => write!(f, "fetch failed: {e}"),
        }
    }
}

/// Fetches public pages.
pub trait Fetcher: Send + Sync {
    /// GET `url`.
    fn get(&self, url: &str) -> Result<Fetched, FetchError>;
}

/// Validate a URL for fetching: HTTPS, a DNS hostname, nothing exotic.
pub fn validate_url(url: &str) -> Result<(), FetchError> {
    let bad = || FetchError::InvalidUrl(url.chars().take(200).collect());
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) || url.len() > 2048 {
        return Err(bad());
    }
    let rest = url.strip_prefix("https://").ok_or_else(bad)?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.contains('@') || host.contains(':') {
        return Err(bad());
    }
    let host = host.to_ascii_lowercase();
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || !crate::model::is_domain(&host)
    {
        return Err(bad());
    }
    Ok(())
}

/// `curl`-backed fetcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurlFetcher {
    /// Path to `curl`.
    pub binary: String,
    /// Timeout in seconds.
    pub timeout_secs: u32,
    /// Body size cap in bytes.
    pub max_bytes: usize,
    /// User-Agent sent.
    pub user_agent: String,
}

impl Default for CurlFetcher {
    fn default() -> Self {
        Self {
            binary: "curl".into(),
            timeout_secs: 15,
            max_bytes: 512 * 1024,
            user_agent: "ApexSales/1.0 (+https://apexmediation.ee)".into(),
        }
    }
}

const STATUS_MARKER: &str = "\n@@apex-status@@";

impl Fetcher for CurlFetcher {
    fn get(&self, url: &str) -> Result<Fetched, FetchError> {
        validate_url(url)?;
        let output = Command::new(&self.binary)
            .args([
                "--silent",
                "--show-error",
                "--location",
                "--max-redirs",
                "3",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                &self.timeout_secs.to_string(),
                "--max-filesize",
                &self.max_bytes.to_string(),
                "--user-agent",
                &self.user_agent,
                "--write-out",
                &format!("{STATUS_MARKER}%{{http_code}}"),
                "--",
                url,
            ])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| FetchError::Transport(e.to_string()))?;
        if !output.status.success() {
            return Err(FetchError::Transport(
                String::from_utf8_lossy(&output.stderr)
                    .trim()
                    .chars()
                    .take(300)
                    .collect(),
            ));
        }
        parse_curl_output(&output.stdout, self.max_bytes)
    }
}

/// Split curl's stdout into body and the status written after the marker.
pub fn parse_curl_output(stdout: &[u8], max_bytes: usize) -> Result<Fetched, FetchError> {
    let text = String::from_utf8_lossy(stdout);
    let (body, status) = text
        .rsplit_once(STATUS_MARKER)
        .ok_or_else(|| FetchError::Transport("no status from curl".into()))?;
    let status: u16 = status
        .trim()
        .parse()
        .map_err(|_| FetchError::Transport("bad status from curl".into()))?;
    let mut body = body.to_string();
    if body.len() > max_bytes {
        let mut cut = max_bytes;
        while !body.is_char_boundary(cut) {
            cut -= 1;
        }
        body.truncate(cut);
    }
    Ok(Fetched { status, body })
}

/// A fixed map of responses, for tests and replays.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapFetcher {
    /// URL → (status, body). Missing URLs are a transport failure.
    pub pages: Vec<(String, u16, String)>,
}

impl Fetcher for MapFetcher {
    fn get(&self, url: &str) -> Result<Fetched, FetchError> {
        validate_url(url)?;
        self.pages
            .iter()
            .find(|(u, _, _)| u == url)
            .map(|(_, status, body)| Fetched {
                status: *status,
                body: body.clone(),
            })
            .ok_or_else(|| FetchError::Transport(format!("no route to {url}")))
    }
}

/// Why a send failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailError(pub String);

impl std::fmt::Display for MailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mail transport: {}", self.0)
    }
}

/// Hands messages to a transport.
pub trait Mailer {
    /// Send one message.
    fn send(&self, msg: &Outgoing) -> Result<(), MailError>;
}

/// The local MTA's `sendmail` interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendmailMailer {
    /// Path to `sendmail`.
    pub binary: String,
}

impl Mailer for SendmailMailer {
    fn send(&self, msg: &Outgoing) -> Result<(), MailError> {
        let ok_addr = |a: &str| crate::model::normalize_email(a).is_some() && !a.starts_with('-');
        if !ok_addr(&msg.from) || !ok_addr(&msg.to) {
            return Err(MailError("refused address".into()));
        }
        let mut child = Command::new(&self.binary)
            .args(["-i", "-f", &msg.from, "--", &msg.to])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| MailError(e.to_string()))?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(msg.raw.as_bytes())
                .map_err(|e| MailError(e.to_string()))?;
        }
        let out = child
            .wait_with_output()
            .map_err(|e| MailError(e.to_string()))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(MailError(
                String::from_utf8_lossy(&out.stderr)
                    .trim()
                    .chars()
                    .take(300)
                    .collect(),
            ))
        }
    }
}

/// Writes each message to `<dir>/<message-id>.eml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxMailer {
    /// Output directory.
    pub dir: PathBuf,
}

impl Mailer for OutboxMailer {
    fn send(&self, msg: &Outgoing) -> Result<(), MailError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| MailError(e.to_string()))?;
        let name: String = msg
            .message_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        std::fs::write(self.dir.join(format!("{name}.eml")), &msg.raw)
            .map_err(|e| MailError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_validated_before_any_fetch() {
        assert!(validate_url("https://studio.example/app-ads.txt").is_ok());
        for bad in [
            "http://studio.example/app-ads.txt",
            "https://127.0.0.1/x",
            "https://localhost/x",
            "https://printer.local/x",
            "https://user@studio.example/",
            "https://studio.example:8443/",
            "https://studio.example/a b",
            "https://studio.example/\n",
            "-https://studio.example",
            "file:///etc/passwd",
        ] {
            assert!(
                matches!(validate_url(bad), Err(FetchError::InvalidUrl(_))),
                "{bad}"
            );
        }
        assert!(FetchError::InvalidUrl("x".into())
            .to_string()
            .contains("refused"));
    }

    #[test]
    fn curl_output_is_parsed_and_capped() {
        let out = format!("hello é world{STATUS_MARKER}200");
        let f = parse_curl_output(out.as_bytes(), 7).unwrap();
        assert_eq!(f.status, 200);
        assert_eq!(f.body, "hello ");
        assert!(parse_curl_output(b"no marker", 10).is_err());
        assert!(parse_curl_output(format!("x{STATUS_MARKER}abc").as_bytes(), 10).is_err());
    }

    #[test]
    fn map_fetcher_and_outbox_mailer() {
        let f = MapFetcher {
            pages: vec![("https://a.example/app-ads.txt".into(), 404, String::new())],
        };
        assert_eq!(f.get("https://a.example/app-ads.txt").unwrap().status, 404);
        assert!(f.get("https://b.example/").is_err());
        let dir = std::env::temp_dir().join(format!("apex-sales-outbox-{}", std::process::id()));
        let m = OutboxMailer { dir: dir.clone() };
        let msg = Outgoing {
            from: "a@apex.example".into(),
            to: "b@studio.example".into(),
            message_id: "t1@apex.example".into(),
            raw: "Subject: x\r\n\r\nbody\r\n".into(),
        };
        m.send(&msg).unwrap();
        assert!(dir.join("t1_apex_example.eml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stand-in `curl`: a shell script that prints what real curl would.
    #[cfg(unix)]
    fn fake_curl(dir: &std::path::Path, name: &str, script: &str) -> String {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    /// Executing a just-written script can race a concurrent fork in another
    /// test thread that still holds its write descriptor (ETXTBSY); retry.
    #[cfg(unix)]
    fn get_settled(f: &CurlFetcher, url: &str) -> Result<Fetched, FetchError> {
        let mut last = f.get(url);
        for _ in 0..20 {
            match &last {
                Err(FetchError::Transport(e)) if e.contains("busy") => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    last = f.get(url);
                }
                _ => break,
            }
        }
        last
    }

    #[cfg(unix)]
    #[test]
    fn curl_fetcher_runs_curl_and_reads_the_status_it_writes() {
        let dir = std::env::temp_dir().join(format!("apex-sales-curl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let defaults = CurlFetcher::default();
        assert_eq!(defaults.binary, "curl");
        assert!(defaults.timeout_secs > 0 && defaults.max_bytes > 0);
        let ok = CurlFetcher {
            binary: fake_curl(
                &dir,
                "ok.sh",
                "printf '%s\\n@@apex-status@@%s' 'google.com, pub-1, DIRECT' 200",
            ),
            ..CurlFetcher::default()
        };
        let page = get_settled(&ok, "https://studio.example/app-ads.txt").unwrap();
        assert_eq!(page.status, 200);
        assert_eq!(page.body, "google.com, pub-1, DIRECT");
        // Invalid URLs never reach the binary.
        assert!(matches!(
            ok.get("http://studio.example/"),
            Err(FetchError::InvalidUrl(_))
        ));
        let failing = CurlFetcher {
            binary: fake_curl(&dir, "fail.sh", "echo 'could not resolve host' >&2; exit 6"),
            ..CurlFetcher::default()
        };
        assert_eq!(
            get_settled(&failing, "https://studio.example/"),
            Err(FetchError::Transport("could not resolve host".into()))
        );
        let missing = CurlFetcher {
            binary: dir.join("absent").display().to_string(),
            ..CurlFetcher::default()
        };
        assert!(matches!(
            missing.get("https://studio.example/"),
            Err(FetchError::Transport(_))
        ));
        assert!(FetchError::Transport("x".into())
            .to_string()
            .contains("failed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn outbox_failures_are_errors() {
        let root =
            std::env::temp_dir().join(format!("apex-sales-outbox-err-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let msg = Outgoing {
            from: "a@apex.example".into(),
            to: "b@studio.example".into(),
            message_id: "m1".into(),
            raw: "x".into(),
        };
        // The outbox path is a file, so the directory cannot be created.
        let file = root.join("not-a-dir");
        std::fs::write(&file, "").unwrap();
        assert!(OutboxMailer { dir: file }.send(&msg).is_err());
        // The message path is a directory, so the write fails.
        std::fs::create_dir_all(root.join("box").join("m1.eml")).unwrap();
        assert!(OutboxMailer {
            dir: root.join("box")
        }
        .send(&msg)
        .is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn sendmail_that_exits_without_reading_is_a_failure() {
        // `true` exits at once: a message larger than the pipe buffer cannot
        // be handed over, and that must be an error, not a silent drop.
        let m = SendmailMailer {
            binary: "true".into(),
        };
        let msg = Outgoing {
            from: "a@apex.example".into(),
            to: "b@studio.example".into(),
            message_id: "m".into(),
            raw: "x".repeat(4 << 20),
        };
        assert!(m.send(&msg).is_err());
    }

    #[test]
    fn sendmail_refuses_hostile_addresses_and_reports_failures() {
        let m = SendmailMailer {
            binary: "/nonexistent/sendmail".into(),
        };
        let mut msg = Outgoing {
            from: "-oQ/tmp@x.example".into(),
            to: "b@studio.example".into(),
            message_id: "m".into(),
            raw: String::new(),
        };
        assert_eq!(m.send(&msg), Err(MailError("refused address".into())));
        msg.from = "a@apex.example".into();
        assert!(
            m.send(&msg).is_err(),
            "missing binary is an error, not a panic"
        );
        let ok = SendmailMailer {
            binary: "true".into(),
        };
        assert_eq!(ok.send(&msg), Ok(()));
        let fails = SendmailMailer {
            binary: "false".into(),
        };
        assert!(fails.send(&msg).is_err());
        assert!(MailError("x".into()).to_string().contains("transport"));
    }
}
