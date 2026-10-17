//! The daily settings watcher: reconcile what the platform reports each day
//! against what Apex wrote, so a publisher edit is detected as a voiding
//! guardrail and an override day stops billing. A later Apex change to the
//! same setting never clears an override: the chain keeps multiplying.

use std::collections::BTreeMap;

/// One day's settings for one scope, hashed canonically by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsSnapshot {
    pub day: String,
    pub scope: String,
    pub values_hash: String,
}

/// A write Apex made and expects to see on the next snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApexApplication {
    pub change_id: String,
    pub day: String,
    pub scope: String,
    pub values_hash_after: String,
}

/// What the watcher concluded for one day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsEvent {
    /// The snapshot matches the Apex write that was expected.
    ApexApplied { change_id: String, day: String },
    /// The snapshot differs from both the last snapshot and any expected
    /// Apex write: someone else changed the ad unit.
    PublisherEdit {
        day: String,
        scope: String,
        expected_hash: String,
        found_hash: String,
    },
    /// No change since the previous snapshot.
    Unchanged { day: String },
}

/// The per-scope watcher. `expected` holds at most one outstanding Apex write
/// per scope; `overrides` remembers the first publisher edit per scope
/// forever.
#[derive(Debug, Default)]
pub struct SettingsWatcher {
    last: BTreeMap<String, SettingsSnapshot>,
    overrides: BTreeMap<String, String>,
}

impl SettingsWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe one day's snapshot for one scope.
    pub fn observe(
        &mut self,
        snapshot: &SettingsSnapshot,
        expected_apex: Option<&ApexApplication>,
    ) -> SettingsEvent {
        let previous = self.last.insert(snapshot.scope.clone(), snapshot.clone());
        if let Some(application) = expected_apex {
            if application.scope == snapshot.scope
                && snapshot.day >= application.day
                && snapshot.values_hash == application.values_hash_after
            {
                return SettingsEvent::ApexApplied {
                    change_id: application.change_id.clone(),
                    day: snapshot.day.clone(),
                };
            }
        }
        match previous {
            Some(previous) if previous.values_hash == snapshot.values_hash => {
                SettingsEvent::Unchanged {
                    day: snapshot.day.clone(),
                }
            }
            Some(previous) => {
                // A publisher edit only registers the first time per scope;
                // a later Apex write does not clear it.
                self.overrides
                    .entry(snapshot.scope.clone())
                    .or_insert_with(|| snapshot.day.clone());
                SettingsEvent::PublisherEdit {
                    day: snapshot.day.clone(),
                    scope: snapshot.scope.clone(),
                    expected_hash: previous.values_hash,
                    found_hash: snapshot.values_hash.clone(),
                }
            }
            None => SettingsEvent::Unchanged {
                day: snapshot.day.clone(),
            },
        }
    }

    /// The first publisher-edit day for a scope, when one happened. This is
    /// the override day that stops billing for changes on that scope.
    pub fn override_day(&self, scope: &str) -> Option<&str> {
        self.overrides.get(scope).map(String::as_str)
    }
}

/// Apply the watcher's override to a kept change: set the override day when
/// one exists, leaving an existing override untouched.
pub fn apply_override(
    change: &mut crate::billing::KeptChange,
    scope: &str,
    watcher: &SettingsWatcher,
) {
    if let Some(day) = watcher.override_day(scope) {
        if change.overridden_day.is_none() {
            change.overridden_day = Some(day.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn snapshot(day: &str, hash: &str) -> SettingsSnapshot {
        SettingsSnapshot {
            day: day.to_string(),
            scope: "u1".to_string(),
            values_hash: hash.to_string(),
        }
    }

    #[test]
    fn the_watcher_separates_apex_writes_from_publisher_edits() {
        let mut watcher = SettingsWatcher::new();
        assert!(matches!(
            watcher.observe(&snapshot("2026-10-01", "h0"), None),
            SettingsEvent::Unchanged { .. }
        ));
        let application = ApexApplication {
            change_id: "c1".into(),
            day: "2026-10-02".into(),
            scope: "u1".into(),
            values_hash_after: "h1".into(),
        };
        assert_eq!(
            watcher.observe(&snapshot("2026-10-02", "h1"), Some(&application)),
            SettingsEvent::ApexApplied {
                change_id: "c1".into(),
                day: "2026-10-02".into()
            }
        );
        // A publisher edit is anything else that differs from the last seen.
        match watcher.observe(&snapshot("2026-10-03", "h9"), None) {
            SettingsEvent::PublisherEdit { day, scope, .. } => {
                assert_eq!(day, "2026-10-03");
                assert_eq!(scope, "u1");
            }
            other => panic!("expected publisher edit, got {other:?}"),
        }
        assert_eq!(watcher.override_day("u1"), Some("2026-10-03"));
    }

    #[test]
    fn a_later_apex_write_never_clears_an_override() {
        let mut watcher = SettingsWatcher::new();
        let _ = watcher.observe(&snapshot("2026-10-01", "h0"), None);
        let _ = watcher.observe(&snapshot("2026-10-02", "h9"), None);
        assert_eq!(watcher.override_day("u1"), Some("2026-10-02"));
        let application = ApexApplication {
            change_id: "c2".into(),
            day: "2026-10-03".into(),
            scope: "u1".into(),
            values_hash_after: "h2".into(),
        };
        assert!(matches!(
            watcher.observe(&snapshot("2026-10-03", "h2"), Some(&application)),
            SettingsEvent::ApexApplied { .. }
        ));
        assert_eq!(watcher.override_day("u1"), Some("2026-10-02"));
    }

    #[test]
    fn overrides_apply_to_kept_changes_once() {
        let mut watcher = SettingsWatcher::new();
        let _ = watcher.observe(&snapshot("2026-10-01", "h0"), None);
        let _ = watcher.observe(&snapshot("2026-10-02", "h9"), None);
        let mut change = crate::billing::KeptChange {
            change_id: "c1".into(),
            lambda: 0.03,
            rate_bp: 2000,
            kept_day: "2026-10-01".into(),
            overridden_day: None,
            pre_registration_hash: "pre".into(),
            result_hash: "res".into(),
        };
        apply_override(&mut change, "u1", &watcher);
        assert_eq!(change.overridden_day.as_deref(), Some("2026-10-02"));
        apply_override(&mut change, "u1", &watcher);
        assert_eq!(change.overridden_day.as_deref(), Some("2026-10-02"));
    }

    #[test]
    fn unchanged_snapshots_and_unmatched_expectations_fall_through() {
        let mut watcher = SettingsWatcher::new();
        let _ = watcher.observe(&snapshot("2026-10-01", "h0"), None);
        // The same hash on a later day is Unchanged, not an edit.
        assert_eq!(
            watcher.observe(&snapshot("2026-10-02", "h0"), None),
            SettingsEvent::Unchanged {
                day: "2026-10-02".into()
            }
        );
        // An expected Apex write whose hash does not match falls through and
        // is reported as the publisher edit it must be.
        let application = ApexApplication {
            change_id: "c1".into(),
            day: "2026-10-03".into(),
            scope: "u1".into(),
            values_hash_after: "h1".into(),
        };
        assert!(matches!(
            watcher.observe(&snapshot("2026-10-03", "h9"), Some(&application)),
            SettingsEvent::PublisherEdit { .. }
        ));
        // Applying an override for an unwatched scope leaves the change as is.
        let mut change = crate::billing::KeptChange {
            change_id: "c2".into(),
            lambda: 0.02,
            rate_bp: 2000,
            kept_day: "2026-10-01".into(),
            overridden_day: None,
            pre_registration_hash: "pre".into(),
            result_hash: "res".into(),
        };
        apply_override(&mut change, "u-other", &watcher);
        assert_eq!(change.overridden_day, None);
    }
}
