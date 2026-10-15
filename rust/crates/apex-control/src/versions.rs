//! Object versions for the server-side stale-state check.
//!
//! The design document requires stale-state validation on the server (page
//! 64): an action form captures a version token, and the POST only runs when
//! the object still matches what the form was prepared from. The token
//! combines two facts:
//!
//! - a monotonically increasing version, bumped whenever an executed action
//!   or a new observation changes the object, and
//! - the SHA-256 digest of the object's presented state, so an external
//!   change (a fresh analytics read, a new incident update) invalidates the
//!   form even when this process never acted on it.
//!
//! A mismatch answers 409 with the conflict copy and requests nothing.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::signing::digest_json;

/// A `versionful` token: `v<counter>:<sha256>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionToken {
    /// Monotonic version at the moment the form was rendered.
    pub version: u64,
    /// State digest at the moment the form was rendered.
    pub digest: String,
}

impl VersionToken {
    /// Render the hidden-form value.
    pub fn encode(&self) -> String {
        format!("v{}:{}", self.version, self.digest)
    }

    /// Parse a hidden-form value; `None` for anything malformed, which
    /// fails the check (a forged token never passes).
    pub fn parse(raw: &str) -> Option<Self> {
        let rest = raw.strip_prefix('v')?;
        let (version, digest) = rest.split_once(':')?;
        let version = version.parse::<u64>().ok()?;
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        Some(Self {
            version,
            digest: digest.to_ascii_lowercase(),
        })
    }
}

/// Why a submit was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionCheck {
    /// The token matches the current object state.
    Match,
    /// The token no longer matches; the caller answers 409.
    Stale {
        /// The reason, bounded and safe to journal.
        reason: &'static str,
    },
}

#[derive(Debug, Clone)]
struct Versioned {
    version: u64,
    digest: String,
}

#[derive(Debug, Default)]
struct Inner {
    objects: HashMap<String, Versioned>,
}

/// The bounded in-memory object-version table.
#[derive(Debug)]
pub struct ObjectVersions {
    inner: Mutex<Inner>,
    capacity: usize,
}

/// Default bound on tracked objects.
pub const DEFAULT_VERSION_CAPACITY: usize = 4096;

impl ObjectVersions {
    /// A table with an explicit bound.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            capacity: capacity.max(1),
        }
    }

    /// The deployment default.
    pub fn bounded() -> Self {
        Self::new(DEFAULT_VERSION_CAPACITY)
    }

    /// Observe the object's current state and return the token a form must
    /// embed. The version bumps exactly when the digest changes, so the
    /// first observation is version 1 and a re-rendered unchanged object
    /// keeps its version.
    pub fn observe(&self, key: &str, state: &serde_json::Value) -> VersionToken {
        let digest = digest_json(state);
        let mut inner = self.lock();
        if !inner.objects.contains_key(key) && inner.objects.len() >= self.capacity {
            inner.evict_one();
        }
        let entry = inner
            .objects
            .entry(key.to_string())
            .or_insert_with(|| Versioned {
                version: 1,
                digest: digest.clone(),
            });
        if entry.digest != digest {
            entry.version = entry.version.saturating_add(1);
            entry.digest = digest.clone();
        }
        VersionToken {
            version: entry.version,
            digest,
        }
    }

    /// Validate a token against the object's current state. A digest or
    /// version mismatch, or an unparsable token, is `Stale`.
    pub fn check(
        &self,
        key: &str,
        token: &VersionToken,
        current: &serde_json::Value,
    ) -> VersionCheck {
        let current_digest = digest_json(current);
        if current_digest != token.digest {
            return VersionCheck::Stale {
                reason: "the object state changed since the form was prepared",
            };
        }
        let inner = self.lock();
        match inner.objects.get(key) {
            Some(entry) if entry.version == token.version => VersionCheck::Match,
            Some(_) => VersionCheck::Stale {
                reason: "the object moved to a newer version",
            },
            None => VersionCheck::Stale {
                reason: "the object is not present in this snapshot",
            },
        }
    }

    /// Bump the object after an executed action, invalidating every open
    /// form that captured the previous version.
    pub fn bump(&self, key: &str) {
        let mut inner = self.lock();
        match inner.objects.get_mut(key) {
            Some(entry) => entry.version = entry.version.saturating_add(1),
            None => {
                inner.objects.insert(
                    key.to_string(),
                    Versioned {
                        version: 1,
                        digest: "unobserved".to_string(),
                    },
                );
            }
        }
    }

    /// Number of tracked objects.
    pub fn len(&self) -> usize {
        self.lock().objects.len()
    }

    /// True when no object is tracked.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Inner {
    /// Evict one arbitrary entry when at capacity. Version tokens are
    /// deliberately ephemeral: an evicted object re-observes as version 1
    /// with a fresh digest, and any form captured before eviction still
    /// fails the digest comparison whenever the state moved.
    fn evict_one(&mut self) {
        if let Some(key) = self.objects.keys().next().cloned() {
            self.objects.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(owner: &str) -> serde_json::Value {
        serde_json::json!({"owner": owner, "state": "open"})
    }

    #[test]
    fn observe_returns_a_stable_token_until_the_state_changes() {
        let versions = ObjectVersions::bounded();
        let first = versions.observe("inc_018", &state("maya"));
        assert_eq!(first.version, 1);
        let second = versions.observe("inc_018", &state("maya"));
        assert_eq!(second, first, "unchanged state keeps its version");
        let third = versions.observe("inc_018", &state("sam"));
        assert_eq!(third.version, 2);
        assert_ne!(third.digest, first.digest);
    }

    #[test]
    fn a_changed_digest_or_version_is_stale() {
        let versions = ObjectVersions::bounded();
        let token = versions.observe("inv_104", &state("maya"));
        assert_eq!(
            versions.check("inv_104", &token, &state("maya")),
            VersionCheck::Match
        );
        // External change: same version, different digest.
        assert!(matches!(
            versions.check("inv_104", &token, &state("sam")),
            VersionCheck::Stale { .. }
        ));
        // Internal change: bump moves the version.
        versions.bump("inv_104");
        assert!(matches!(
            versions.check("inv_104", &token, &state("maya")),
            VersionCheck::Stale { .. }
        ));
    }

    #[test]
    fn unknown_objects_and_bad_tokens_are_stale() {
        let versions = ObjectVersions::bounded();
        let token = versions.observe("inc_018", &state("maya"));
        assert!(matches!(
            versions.check("inc_999", &token, &state("maya")),
            VersionCheck::Stale { .. }
        ));
        assert_eq!(VersionToken::parse("garbage"), None);
        assert_eq!(VersionToken::parse("v1:short"), None);
        assert_eq!(VersionToken::parse("v1:zz"), None);
        assert_eq!(VersionToken::parse("v"), None);
        let good = token.encode();
        assert_eq!(VersionToken::parse(&good), Some(token));
    }

    #[test]
    fn the_table_is_bounded() {
        let versions = ObjectVersions::new(4);
        for index in 0..16 {
            versions.observe(&format!("obj_{index}"), &state("maya"));
        }
        assert!(versions.len() <= 4, "bounded: {}", versions.len());
    }
}
