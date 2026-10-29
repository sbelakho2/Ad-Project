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
    /// Resolve and reject private/loopback addresses before curl runs.
    /// Disabled only by deterministic tests that use a fake curl.
    pub resolve_public: bool,
}

impl Default for CurlFetcher {
    fn default() -> Self {
        Self {
            binary: "curl".into(),
            timeout_secs: 15,
            max_bytes: 512 * 1024,
            user_agent: "ApexSales/1.0 (+https://apexmediation.ee)".into(),
            resolve_public: true,
        }
    }
}

const STATUS_MARKER: &str = "\n@@apex-status@@";

impl Fetcher for CurlFetcher {
    fn get(&self, url: &str) -> Result<Fetched, FetchError> {
        let mut current = url.to_string();
        for _ in 0..=3 {
            validate_url(&current)?;
            let pinned = if self.resolve_public {
                Some(pin_public_host(&current)?)
            } else {
                None
            };
            let mut command = Command::new(&self.binary);
            command.args([
                "--silent",
                "--show-error",
                "--max-redirs",
                "0",
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
                &format!("{STATUS_MARKER}%{{http_code}}\n%{{redirect_url}}"),
            ]);
            if let Some((host, ip)) = &pinned {
                let address = match ip {
                    std::net::IpAddr::V6(v6) => format!("[{v6}]"),
                    std::net::IpAddr::V4(v4) => v4.to_string(),
                };
                command
                    .arg("--resolve")
                    .arg(format!("{host}:443:{address}"));
            }
            let output = command
                .arg("--")
                .arg(&current)
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
            let text = String::from_utf8_lossy(&output.stdout);
            let (body, tail) = text
                .rsplit_once(STATUS_MARKER)
                .ok_or_else(|| FetchError::Transport("no status from curl".into()))?;
            let mut lines = tail.lines();
            let status: u16 = lines
                .next()
                .unwrap_or_default()
                .trim()
                .parse()
                .map_err(|_| FetchError::Transport("bad status from curl".into()))?;
            let redirect = lines.next().unwrap_or_default().trim();
            if is_redirect(status, redirect) {
                current = redirect.to_string();
                continue;
            }
            let mut body = body.to_string();
            if body.len() > self.max_bytes {
                let mut cut = self.max_bytes;
                while !body.is_char_boundary(cut) {
                    cut -= 1;
                }
                body.truncate(cut);
            }
            return Ok(Fetched { status, body });
        }
        Err(FetchError::InvalidUrl("too many redirects".into()))
    }
}

fn is_redirect(status: u16, redirect: &str) -> bool {
    (300..400).contains(&status) && !redirect.is_empty()
}

fn pin_public_host(url: &str) -> Result<(String, std::net::IpAddr), FetchError> {
    use std::net::ToSocketAddrs as _;
    let host = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .filter(|host| !host.is_empty())
        .ok_or_else(|| FetchError::InvalidUrl(url.to_string()))?;
    let addresses: Vec<std::net::IpAddr> = (host, 443)
        .to_socket_addrs()
        .map_err(|e| FetchError::Transport(format!("resolve {host}: {e}")))?
        .map(|addr| addr.ip())
        .collect();
    let address = select_public_address(host, &addresses)?;
    Ok((host.to_string(), address))
}

fn select_public_address(
    host: &str,
    addresses: &[std::net::IpAddr],
) -> Result<std::net::IpAddr, FetchError> {
    if addresses.is_empty() || addresses.iter().any(|ip| !public_ip(*ip)) {
        return Err(FetchError::InvalidUrl(format!("non-public host {host}")));
    }
    Ok(addresses[0])
}

/// Carrier-grade NAT range (100.64/10).
fn is_cgnat(octets: &[u8; 4]) -> bool {
    octets[0] == 100 && (64..128).contains(&octets[1])
}

fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.octets()[0] == 0
                || is_cgnat(&v4.octets()))
        }
        std::net::IpAddr::V6(v6) => {
            // Reject IPv4-mapped (::ffff:a.b.c.d), NAT64 (64:ff9b::/96),
            // 6to4 (2002::/16) and site-local (fec0::/10) as non-public.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_ip(std::net::IpAddr::V4(v4));
            }
            let octets = v6.octets();
            let is_nat64 =
                octets[0] == 0x00 && octets[1] == 0x64 && octets[2] == 0xff && octets[3] == 0x9b;
            let is_6to4 = octets[0] == 0x20 && octets[1] == 0x02;
            let is_site_local = octets[0] == 0xfe && (octets[1] & 0xc0) == 0xc0;
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
                || is_nat64
                || is_6to4
                || is_site_local)
        }
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
        let path = self.dir.join(format!("{name}.eml"));
        std::fs::write(&path, &msg.raw).map_err(|e| MailError(e.to_string()))?;
        // A preview carries the recipient's address and the message body:
        // owner-only, like the engine state.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| MailError(e.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgnat_boundaries() {
        assert!(is_cgnat(&[100, 64, 0, 1]));
        assert!(is_cgnat(&[100, 127, 255, 255]));
        assert!(!is_cgnat(&[100, 63, 255, 255]));
        assert!(!is_cgnat(&[100, 128, 0, 0]));
        assert!(!is_cgnat(&[101, 64, 0, 0]));
    }

    #[test]
    fn cgnat_and_site_local_are_not_public() {
        for addr in ["100.64.0.1", "100.127.255.255"] {
            assert!(!public_ip(addr.parse().unwrap()), "{addr}");
        }
        for addr in ["100.0.0.1", "100.128.0.1", "8.8.8.8"] {
            assert!(public_ip(addr.parse().unwrap()), "{addr}");
        }
        for addr in ["::ffff:10.0.0.1", "64:ff9b::0a00:0001", "fec0::1"] {
            assert!(!public_ip(addr.parse().unwrap()), "{addr}");
        }
    }

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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.join("t1_apex_example.eml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "a preview with an address is owner-only");
        }
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
    fn a_host_with_any_private_answer_is_refused() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        let public = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));
        let private = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        assert_eq!(
            select_public_address("example.test", &[public]).unwrap(),
            public
        );
        assert!(select_public_address("example.test", &[]).is_err());
        assert!(select_public_address("example.test", &[public, private]).is_err());
        assert!(select_public_address("example.test", &[IpAddr::V6(Ipv6Addr::LOCALHOST)]).is_err());
        assert!(!public_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(public_ip(public));
    }

    #[test]
    fn redirect_following_needs_a_3xx_and_a_location() {
        assert!(is_redirect(301, "https://next.example/"));
        assert!(is_redirect(399, "https://next.example/"));
        assert!(!is_redirect(302, ""));
        assert!(!is_redirect(200, "https://next.example/"));
    }

    #[test]
    fn curl_fetcher_runs_curl_and_reads_the_status_it_writes() {
        let dir = std::env::temp_dir().join(format!("apex-sales-curl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let defaults = CurlFetcher::default();
        assert_eq!(defaults.binary, "curl");
        assert!(defaults.timeout_secs > 0 && defaults.max_bytes > 0);
        assert!(defaults.resolve_public);
        let ok = CurlFetcher {
            resolve_public: false,
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
            resolve_public: false,
            binary: fake_curl(&dir, "fail.sh", "echo 'could not resolve host' >&2; exit 6"),
            ..CurlFetcher::default()
        };
        assert_eq!(
            get_settled(&failing, "https://studio.example/"),
            Err(FetchError::Transport("could not resolve host".into()))
        );
        let missing = CurlFetcher {
            resolve_public: false,
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
