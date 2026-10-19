//! The new-workspace ownership proof: the creating email domain's public
//! `app-ads.txt`.
//!
//! A new self-serve workspace is created only when the email address really
//! belongs to the domain it names. The check fetches
//! `https://<email-domain>/app-ads.txt` and accepts exactly two shapes:
//!
//! - `OWNERDOMAIN=<email-domain>`, or
//! - `CONTACT=<the exact creating email>`.
//!
//! Everything else is a typed refusal and no sign-in link is issued. The
//! fetch is guarded before any network activity: the domain is resolved with
//! the standard library and **every** answer must be public. Private,
//! loopback, link-local, shared (including the cloud metadata ranges),
//! multicast, unspecified, broadcast and reserved answers are refused, with
//! IPv4-mapped IPv6 answers unwrapped and checked as IPv4. The guard is a
//! small self-contained copy of the watch service's resolve-time rules so
//! this path never depends on a DNS library or a second service being
//! configured.
//!
//! Redirects are not followed (the pinned client disables them): an
//! `app-ads.txt` host that answers `3xx` is a refusal rather than a bounce to
//! an internal address. The guard-approved addresses are pinned into the
//! client for that one request (`ClientBuilder::resolve`), so the connection
//! can only reach an address the guard accepted; a second DNS answer cannot
//! redirect the fetch after the check. A non-200 answer, an oversized body or
//! a transport failure is a refusal with the bounded reason. There is no DNS
//! library here: the only resolution is `std::net::ToSocketAddrs`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use apex_connectors::transport::{
    HttpMethod, HttpRequest, HttpResponse, TransportError, MAX_RESPONSE_BYTES,
};
use apex_core::PlatformId;

use crate::signin::{WorkspaceProofSource, WorkspaceRefusal, UNPROVEN_DOMAIN_REASON};

/// The fetched path, rooted at the email domain.
pub const APP_ADS_PATH: &str = "/app-ads.txt";
/// The response cap: a larger file is refused instead of truncated.
pub const MAX_APP_ADS_BYTES: usize = 1024 * 1024;
/// The fetch timeout.
pub const PROOF_TIMEOUT_MS: u64 = 10_000;
/// The user agent this check presents.
pub const PROOF_USER_AGENT: &str = "ApexWorkspaceProof/1.0 (+https://apexmediation.ee)";

/// One refused address: the class that refused it. The address itself is
/// logged by the caller, never returned to the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardedAddress {
    /// The offending address.
    pub address: IpAddr,
    /// `private`, `loopback`, `link-local`, `shared`, `multicast`,
    /// `unspecified`, `broadcast` or `reserved`.
    pub class: &'static str,
}

/// Classify one address; `None` means public and fetchable.
pub fn classify(ip: IpAddr) -> Option<GuardedAddress> {
    let class = match ip {
        IpAddr::V4(v4) => ipv4_class(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(mapped) => ipv4_class(mapped),
            None => ipv6_class(v6),
        },
    }?;
    Some(GuardedAddress { address: ip, class })
}

/// Refuse the first non-public address in `addresses`.
pub fn guard_addresses(addresses: &[IpAddr]) -> Result<(), GuardedAddress> {
    addresses
        .iter()
        .find_map(|address| classify(*address))
        .map_or(Ok(()), Err)
}

fn ipv4_class(v4: Ipv4Addr) -> Option<&'static str> {
    let octets = v4.octets();
    if v4.is_private() {
        Some("private")
    } else if v4.is_loopback() {
        Some("loopback")
    } else if v4.is_link_local() {
        Some("link-local")
    } else if v4.is_multicast() {
        Some("multicast")
    } else if v4.is_broadcast() {
        Some("broadcast")
    } else if v4.is_unspecified() || octets[0] == 0 {
        Some("unspecified")
    } else if v4.is_documentation()
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || octets[0] >= 240
    {
        Some("reserved")
    } else if octets[0] == 100 && (64..128).contains(&octets[1]) {
        // 100.64.0.0/10 shared address space; the cloud metadata endpoint
        // 100.100.100.200 lives inside it.
        Some("shared")
    } else {
        None
    }
}

fn ipv6_class(v6: Ipv6Addr) -> Option<&'static str> {
    let segments = v6.segments();
    if v6.is_unspecified() {
        Some("unspecified")
    } else if v6.is_loopback() {
        Some("loopback")
    } else if segments[0] & 0xffc0 == 0xfe80 {
        // fe80::/10 link-local.
        Some("link-local")
    } else if segments[0] & 0xfe00 == 0xfc00 {
        // fc00::/7 unique local.
        Some("private")
    } else if segments[0] & 0xff00 == 0xff00 {
        // ff00::/8 multicast.
        Some("multicast")
    } else if segments[..6].iter().all(|segment| *segment == 0) {
        // ::/96 IPv4-compatible space (mapped addresses were unwrapped
        // above), including ::ffff:0:0 style shims.
        Some("reserved")
    } else {
        None
    }
}

/// Resolve a host to its addresses. The production implementation is
/// [`StdDomainResolver`]; tests inject fixed answers.
#[async_trait::async_trait]
pub trait DomainResolver: Send + Sync + std::fmt::Debug {
    /// All addresses the host name resolves to.
    async fn resolve(&self, domain: &str) -> Result<Vec<IpAddr>, String>;
}

/// The standard library resolver (`std::net::ToSocketAddrs`), run on the
/// blocking pool so it never blocks a runtime worker.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdDomainResolver;

#[async_trait::async_trait]
impl DomainResolver for StdDomainResolver {
    async fn resolve(&self, domain: &str) -> Result<Vec<IpAddr>, String> {
        let domain = domain.to_string();
        tokio::task::spawn_blocking(move || {
            let addresses: Vec<IpAddr> = (domain.as_str(), 0u16)
                .to_socket_addrs()
                .map_err(|error| format!("the domain could not be resolved: {error}"))?
                .map(|address| address.ip())
                .collect();
            if addresses.is_empty() {
                return Err("the resolver returned no addresses".to_string());
            }
            Ok(addresses)
        })
        .await
        .map_err(|error| format!("the resolver task failed: {error}"))?
    }
}

/// The HTTP seam the proof uses. Unlike a plain [`apex_connectors::transport::
/// Transport`], it receives the guard-approved addresses with every request so
/// the implementation can pin them into its client. Tests implement this to
/// record exactly which addresses a fetch would use.
#[async_trait::async_trait]
pub trait PinnedTransport: Send + Sync + std::fmt::Debug {
    /// Fetch `request` for `domain`, connecting only to `addresses` (each a
    /// guard-approved IP with the HTTPS port).
    async fn request(
        &self,
        domain: &str,
        addresses: &[SocketAddr],
        request: HttpRequest,
    ) -> Result<HttpResponse, TransportError>;
}

/// The production pinned transport: a fresh `reqwest` client per request with
/// `.resolve(domain, address)` for every guard-approved address, redirects
/// disabled and the proof's timeout. The response is streamed with the same
/// 16 MiB hard cap as the shared transport; the proof then applies its own
/// 1 MiB `app-ads.txt` cap.
#[derive(Debug, Clone, Copy, Default)]
pub struct PinnedReqwestTransport;

#[async_trait::async_trait]
impl PinnedTransport for PinnedReqwestTransport {
    async fn request(
        &self,
        domain: &str,
        addresses: &[SocketAddr],
        request: HttpRequest,
    ) -> Result<HttpResponse, TransportError> {
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(request.timeout_ms));
        for address in addresses {
            builder = builder.resolve(domain, *address);
        }
        let client = builder
            .build()
            .map_err(|error| TransportError::Network(error.without_url().to_string()))?;
        let mut outgoing = client
            .request(reqwest::Method::GET, &request.url)
            .timeout(Duration::from_millis(request.timeout_ms));
        for (name, value) in &request.headers {
            outgoing = outgoing.header(name, value);
        }
        let response = outgoing.send().await.map_err(|error| {
            let error = error.without_url();
            if error.is_timeout() {
                TransportError::Timeout(error.to_string())
            } else {
                TransportError::Network(error.to_string())
            }
        })?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value.to_str().unwrap_or("").to_string(),
                )
            })
            .collect();
        if let Some(declared) = response.content_length() {
            if declared > MAX_RESPONSE_BYTES as u64 {
                return Err(TransportError::Network(format!(
                    "response body declares {declared} bytes, above the {MAX_RESPONSE_BYTES} byte cap"
                )));
            }
        }
        let mut response = response;
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| TransportError::Network(error.without_url().to_string()))?
        {
            let total = body.len() as u64 + chunk.len() as u64;
            if total > MAX_RESPONSE_BYTES as u64 {
                return Err(TransportError::Network(format!(
                    "response body exceeds the {MAX_RESPONSE_BYTES} byte cap"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// The production proof source: resolve-guard, pin the approved addresses,
/// then one HTTPS GET.
pub struct HttpAppAdsProof {
    transport: Arc<dyn PinnedTransport>,
    resolver: Arc<dyn DomainResolver>,
}

impl std::fmt::Debug for HttpAppAdsProof {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpAppAdsProof")
            .field("resolver", &self.resolver)
            .finish_non_exhaustive()
    }
}

impl HttpAppAdsProof {
    /// A proof source over `transport` and the standard resolver (production
    /// wires [`PinnedReqwestTransport`]).
    pub fn new(transport: Arc<dyn PinnedTransport>) -> Self {
        Self {
            transport,
            resolver: Arc::new(StdDomainResolver),
        }
    }

    /// Replace the resolver (tests pin public answers).
    #[must_use]
    pub fn with_resolver(mut self, resolver: Arc<dyn DomainResolver>) -> Self {
        self.resolver = resolver;
        self
    }

    async fn fetch(&self, domain: &str) -> Result<String, String> {
        let addresses = self.resolver.resolve(domain).await?;
        guard_addresses(&addresses)
            .map_err(|refusal| format!("{} resolves to {}", domain, refusal.class))?;
        // Pin every approved answer with the HTTPS port. The client is built
        // from exactly this list, so the connection cannot use a later DNS
        // answer for the same name.
        let pinned: Vec<SocketAddr> = addresses
            .iter()
            .map(|address| SocketAddr::new(*address, 443))
            .collect();
        let request = HttpRequest {
            platform: PlatformId::Billing,
            operation: "fetchWorkspaceAppAdsTxt".to_string(),
            method: HttpMethod::Get,
            url: format!("https://{domain}{APP_ADS_PATH}"),
            headers: vec![
                ("Accept".into(), "text/plain".into()),
                ("User-Agent".into(), PROOF_USER_AGENT.into()),
            ],
            body: None,
            timeout_ms: PROOF_TIMEOUT_MS,
        };
        let response = self
            .transport
            .request(domain, &pinned, request)
            .await
            .map_err(|error| match error {
                TransportError::Network(detail) => format!("app-ads.txt fetch failed: {detail}"),
                TransportError::Timeout(detail) => {
                    format!("app-ads.txt fetch timed out: {detail}")
                }
                TransportError::Platform(error) => format!("app-ads.txt fetch failed: {error}"),
            })?;
        if response.status != 200 {
            return Err(format!("app-ads.txt answered HTTP {}", response.status));
        }
        if response.body.len() > MAX_APP_ADS_BYTES {
            return Err(format!(
                "app-ads.txt is {} bytes, above the {MAX_APP_ADS_BYTES} byte cap",
                response.body.len()
            ));
        }
        String::from_utf8(response.body).map_err(|_| "app-ads.txt is not UTF-8".to_string())
    }
}

/// Whether `text` proves `email` (with `domain` its email domain). The
/// accepted lines are `OWNERDOMAIN=<domain>` and `CONTACT=<email>`; variable
/// names, the domain and the address compare case-insensitively. Blank lines
/// and comments (`#`) are ignored.
pub fn app_ads_proves_owner(text: &str, domain: &str, email: &str) -> bool {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let email = email.trim().to_ascii_lowercase();
    text.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        line.split(',').any(|entry| {
            let Some((variable, value)) = entry.split_once('=') else {
                return false;
            };
            let value = value.trim();
            if variable.trim().eq_ignore_ascii_case("OWNERDOMAIN") {
                value.trim_end_matches('.').eq_ignore_ascii_case(&domain)
            } else if variable.trim().eq_ignore_ascii_case("CONTACT") {
                value.eq_ignore_ascii_case(&email)
            } else {
                false
            }
        })
    })
}

/// The number of distinct ad-system account ids a fetched app-ads.txt lists.
///
/// This is kept as public evidence about the file, but it is deliberately not
/// the watch tier: one publisher routinely lists many account ids for one app,
/// so the tier counts the distinct apps the subscriber names instead.
pub fn app_ads_app_count(text: &str) -> u32 {
    let mut ids = std::collections::BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        if fields.len() < 3 {
            continue;
        }
        if !fields[1].is_empty() {
            ids.insert(fields[1].to_ascii_lowercase());
        }
    }
    ids.len() as u32
}

#[async_trait::async_trait]
impl WorkspaceProofSource for HttpAppAdsProof {
    async fn prove_owner(&self, email: &str) -> Result<(), WorkspaceRefusal> {
        let domain = email
            .split_once('@')
            .map(|(_, domain)| domain.trim().trim_end_matches('.').to_ascii_lowercase())
            .filter(|domain| !domain.is_empty())
            .ok_or(WorkspaceRefusal::InvalidEmail)?;
        let normalized = apex_watch::normalize_domain(&domain).map_err(|reason| {
            WorkspaceRefusal::UnprovenDomain {
                domain: domain.clone(),
                reason,
            }
        })?;
        let text =
            self.fetch(&normalized)
                .await
                .map_err(|reason| WorkspaceRefusal::UnprovenDomain {
                    domain: normalized.clone(),
                    reason,
                })?;
        if app_ads_proves_owner(&text, &normalized, email) {
            Ok(())
        } else {
            Err(WorkspaceRefusal::UnprovenDomain {
                domain: normalized,
                reason: UNPROVEN_DOMAIN_REASON.to_string(),
            })
        }
    }

    async fn app_count(&self, domain: &str) -> Result<u32, WorkspaceRefusal> {
        let normalized = apex_watch::normalize_domain(domain.trim().trim_end_matches('.'))
            .map_err(|reason| WorkspaceRefusal::UnprovenDomain {
                domain: domain.to_string(),
                reason,
            })?;
        let text =
            self.fetch(&normalized)
                .await
                .map_err(|reason| WorkspaceRefusal::UnprovenDomain {
                    domain: normalized.clone(),
                    reason,
                })?;
        Ok(app_ads_app_count(&text))
    }
}

/// A proof source for tests and embedders: accepts a fixed set of email
/// domains without any network activity.
#[derive(Debug, Clone, Default)]
pub struct StaticAppAdsProof {
    accepted_domains: Vec<String>,
    app_counts: std::collections::BTreeMap<String, u32>,
}

impl StaticAppAdsProof {
    /// Accept exactly `domains` (compared case-insensitively).
    pub fn accepting(domains: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            accepted_domains: domains
                .into_iter()
                .map(|domain| domain.into().trim().to_ascii_lowercase())
                .collect(),
            app_counts: std::collections::BTreeMap::new(),
        }
    }

    /// Record `count` account ids for `domain` as test evidence. The watch
    /// tier does not read this count; it is available to callers that still
    /// inspect the account-id histogram.
    #[must_use]
    pub fn with_app_count(mut self, domain: &str, count: u32) -> Self {
        self.app_counts.insert(
            domain.trim().trim_end_matches('.').to_ascii_lowercase(),
            count,
        );
        self
    }
}

#[async_trait::async_trait]
impl WorkspaceProofSource for StaticAppAdsProof {
    async fn prove_owner(&self, email: &str) -> Result<(), WorkspaceRefusal> {
        let domain = email
            .split_once('@')
            .map(|(_, domain)| domain.trim().trim_end_matches('.').to_ascii_lowercase())
            .ok_or(WorkspaceRefusal::InvalidEmail)?;
        if self.accepted_domains.contains(&domain) {
            Ok(())
        } else {
            Err(WorkspaceRefusal::UnprovenDomain {
                domain,
                reason: "the test proof source does not accept this domain".to_string(),
            })
        }
    }

    async fn app_count(&self, domain: &str) -> Result<u32, WorkspaceRefusal> {
        let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
        Ok(self.app_counts.get(&domain).copied().unwrap_or(0))
    }
}

/// A response helper for scripted test transports.
#[cfg(test)]
fn response(status: u16, body: &str) -> apex_connectors::transport::HttpResponse {
    apex_connectors::transport::HttpResponse {
        status,
        headers: vec![],
        body: body.as_bytes().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::net::IpAddr;

    use apex_connectors::transport::HttpResponse;

    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("test address")
    }

    #[test]
    fn private_and_metadata_answers_are_refused() {
        for (address, class) in [
            ("127.0.0.1", "loopback"),
            ("10.0.0.5", "private"),
            ("172.16.3.4", "private"),
            ("192.168.1.1", "private"),
            ("169.254.169.254", "link-local"),
            ("100.100.100.200", "shared"),
            ("100.64.0.1", "shared"),
            ("224.0.0.1", "multicast"),
            ("0.0.0.0", "unspecified"),
            ("255.255.255.255", "broadcast"),
            ("192.0.2.10", "reserved"),
            ("240.0.0.1", "reserved"),
            ("::1", "loopback"),
            ("fe80::1", "link-local"),
            ("fc00::1", "private"),
            ("ff02::1", "multicast"),
            ("::ffff:127.0.0.1", "loopback"),
            ("::ffff:169.254.169.254", "link-local"),
        ] {
            let refusal = classify(ip(address)).unwrap_or_else(|| panic!("{address} allowed"));
            assert_eq!(refusal.class, class, "{address}");
        }
        for address in ["8.8.8.8", "93.184.216.34", "2606:4700:4700::1111"] {
            assert_eq!(classify(ip(address)), None, "{address}");
        }
        assert!(guard_addresses(&[ip("8.8.8.8"), ip("10.0.0.1")]).is_err());
        assert!(guard_addresses(&[ip("8.8.8.8"), ip("1.1.1.1")]).is_ok());
    }

    #[test]
    fn app_ads_owner_domain_and_contact_are_accepted_exactly() {
        assert!(app_ads_proves_owner(
            "google.com, pub-1, DIRECT\nOWNERDOMAIN=Studio.Example.\n",
            "studio.example",
            "founder@studio.example"
        ));
        assert!(app_ads_proves_owner(
            "CONTACT=Founder@Studio.Example\n",
            "studio.example",
            "founder@studio.example"
        ));
        assert!(app_ads_proves_owner(
            "google.com, pub-1, DIRECT, CONTACT=founder@studio.example\n",
            "studio.example",
            "founder@studio.example"
        ));
        for text in [
            "",
            "OWNERDOMAIN=other.example\n",
            "CONTACT=other@studio.example\n",
            "CONTACT=founder@other.example\n",
            "CONTACT=founder@studio.example.evil\n",
            "# OWNERDOMAIN=studio.example\n",
        ] {
            assert!(
                !app_ads_proves_owner(text, "studio.example", "founder@studio.example"),
                "{text:?}"
            );
        }
    }

    #[test]
    fn inline_entries_and_comments_are_parsed() {
        let text = " # a comment\nplaceholder.example, 1, DIRECT\n";
        assert!(!app_ads_proves_owner(
            text,
            "studio.example",
            "a@studio.example"
        ));
    }

    #[tokio::test]
    async fn the_static_proof_accepts_only_its_domains() {
        let proof = StaticAppAdsProof::accepting(["studio.example"]);
        proof
            .prove_owner("a@studio.example")
            .await
            .expect("accepted");
        assert!(matches!(
            proof.prove_owner("a@evil.example").await,
            Err(WorkspaceRefusal::UnprovenDomain { .. })
        ));
        assert!(matches!(
            proof.prove_owner("not-an-email").await,
            Err(WorkspaceRefusal::InvalidEmail)
        ));
    }

    #[tokio::test]
    async fn http_proof_fetches_through_the_transport_and_refuses_non_owners() {
        #[derive(Debug)]
        struct Scripted {
            body: Vec<u8>,
            calls: std::sync::Mutex<usize>,
        }

        #[async_trait::async_trait]
        impl PinnedTransport for Scripted {
            async fn request(
                &self,
                domain: &str,
                addresses: &[SocketAddr],
                request: HttpRequest,
            ) -> Result<HttpResponse, TransportError> {
                assert_eq!(domain, "studio.example");
                assert_eq!(
                    addresses,
                    vec![SocketAddr::new(ip("93.184.216.34"), 443)],
                    "the approved answer is the one pinned into the client"
                );
                assert_eq!(request.url, "https://studio.example/app-ads.txt");
                assert_eq!(request.method, HttpMethod::Get);
                *self.calls.lock().expect("calls") += 1;
                Ok(response(
                    200,
                    std::str::from_utf8(&self.body).expect("utf8"),
                ))
            }
        }

        #[derive(Debug)]
        struct PublicResolver;
        #[async_trait::async_trait]
        impl DomainResolver for PublicResolver {
            async fn resolve(&self, _domain: &str) -> Result<Vec<IpAddr>, String> {
                Ok(vec![ip("93.184.216.34")])
            }
        }

        let transport = Arc::new(Scripted {
            body: b"OWNERDOMAIN=studio.example\n".to_vec(),
            calls: std::sync::Mutex::new(0),
        });
        let proof = HttpAppAdsProof::new(transport.clone()).with_resolver(Arc::new(PublicResolver));
        proof.prove_owner("a@studio.example").await.expect("accept");
        assert_eq!(*transport.calls.lock().expect("calls"), 1);

        let transport = Arc::new(Scripted {
            body: b"CONTACT=other@studio.example\n".to_vec(),
            calls: std::sync::Mutex::new(0),
        });
        let proof = HttpAppAdsProof::new(transport).with_resolver(Arc::new(PublicResolver));
        assert!(matches!(
            proof.prove_owner("a@studio.example").await,
            Err(WorkspaceRefusal::UnprovenDomain { .. })
        ));

        // A domain that does not normalize (reserved local name) is refused
        // before any resolution or fetch.
        let transport = Arc::new(Scripted {
            body: b"OWNERDOMAIN=studio.example\n".to_vec(),
            calls: std::sync::Mutex::new(0),
        });
        let proof = HttpAppAdsProof::new(transport.clone()).with_resolver(Arc::new(PublicResolver));
        assert!(matches!(
            proof.prove_owner("a@localhost").await,
            Err(WorkspaceRefusal::UnprovenDomain { .. })
        ));
        assert_eq!(
            *transport.calls.lock().expect("calls"),
            0,
            "a reserved name is refused before any request"
        );
    }

    #[tokio::test]
    async fn a_private_or_metadata_resolution_is_refused_before_any_request() {
        #[derive(Debug)]
        struct MetadataResolver;
        #[async_trait::async_trait]
        impl DomainResolver for MetadataResolver {
            async fn resolve(&self, _domain: &str) -> Result<Vec<IpAddr>, String> {
                Ok(vec![ip("169.254.169.254"), ip("93.184.216.34")])
            }
        }

        #[derive(Debug, Default)]
        struct CountingTransport {
            calls: std::sync::Mutex<usize>,
        }

        #[async_trait::async_trait]
        impl PinnedTransport for CountingTransport {
            async fn request(
                &self,
                _domain: &str,
                _addresses: &[SocketAddr],
                _request: HttpRequest,
            ) -> Result<HttpResponse, TransportError> {
                *self.calls.lock().expect("calls") += 1;
                Ok(response(200, "OWNERDOMAIN=studio.example\n"))
            }
        }

        let transport = Arc::new(CountingTransport::default());
        let proof =
            HttpAppAdsProof::new(transport.clone()).with_resolver(Arc::new(MetadataResolver));
        let refusal = proof
            .prove_owner("a@studio.example")
            .await
            .expect_err("metadata answer");
        let message = refusal.to_string();
        assert!(message.contains("link-local"), "{message}");
        assert_eq!(
            *transport.calls.lock().expect("calls"),
            0,
            "the fetch never starts"
        );
    }

    #[tokio::test]
    async fn the_client_is_pinned_to_every_approved_address_only() {
        #[derive(Debug)]
        struct MultiResolver;
        #[async_trait::async_trait]
        impl DomainResolver for MultiResolver {
            async fn resolve(&self, _domain: &str) -> Result<Vec<IpAddr>, String> {
                Ok(vec![ip("93.184.216.34"), ip("1.1.1.1")])
            }
        }

        #[derive(Debug, Default)]
        struct Recording {
            pinned: std::sync::Mutex<Vec<(String, Vec<SocketAddr>)>>,
        }

        #[async_trait::async_trait]
        impl PinnedTransport for Recording {
            async fn request(
                &self,
                domain: &str,
                addresses: &[SocketAddr],
                request: HttpRequest,
            ) -> Result<HttpResponse, TransportError> {
                self.pinned
                    .lock()
                    .expect("pinned")
                    .push((domain.to_string(), addresses.to_vec()));
                assert_eq!(request.url, "https://studio.example/app-ads.txt");
                Ok(response(200, "OWNERDOMAIN=studio.example\n"))
            }
        }

        let transport = Arc::new(Recording::default());
        let proof = HttpAppAdsProof::new(transport.clone()).with_resolver(Arc::new(MultiResolver));
        proof.prove_owner("a@studio.example").await.expect("accept");
        let recorded = transport.pinned.lock().expect("pinned");
        assert_eq!(recorded.len(), 1);
        let (domain, addresses) = &recorded[0];
        assert_eq!(domain, "studio.example");
        assert_eq!(
            addresses,
            &vec![
                SocketAddr::new(ip("93.184.216.34"), 443),
                SocketAddr::new(ip("1.1.1.1"), 443)
            ],
            "exactly the guard-approved answers are pinned"
        );
    }
}
