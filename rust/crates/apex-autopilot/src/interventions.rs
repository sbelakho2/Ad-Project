//! Fleet scheduling interventions: the runner side of pause/resume.
//!
//! The control plane appends `pause_scheduling` / `resume_scheduling`
//! records to the append-only JSONL register named by
//! `APEX_CONTROL_INTERVENTIONS`. The runner reads that same file before each
//! platform's cycle. Only *writes* stop: the audit, pull and settle reads
//! continue, so an operator pause never blinds measurement.
//!
//! The composition rule is the register's own order: the latest applicable
//! record wins. A record with no platform applies to every platform; a
//! platform-scoped record applies only to that wire name (`applovin_max`,
//! `levelplay`, `admob`). A malformed line is ignored (the control plane
//! keeps a corrupt-line count on its own read); a missing file means nothing
//! is paused. Any other read error fails closed: the cycle refuses rather
//! than writing while the pause state cannot be established.

use std::path::Path;

use serde::Deserialize;

/// Environment variable naming the intervention register the control plane
/// writes and the runner reads.
pub const INTERVENTIONS_ENV: &str = "APEX_CONTROL_INTERVENTIONS";
/// Default intervention register, matching the control plane's default.
pub const DEFAULT_INTERVENTIONS: &str = ".apex/control/interventions.jsonl";

/// One parsed pause/resume record (only the fields the runner needs).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Intervention {
    kind: String,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    at_ms: i64,
}

/// The pause in force for one platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformPause {
    /// The operator's recorded reason.
    pub reason: String,
    /// When the pause was recorded.
    pub at_ms: i64,
    /// The scope the pause named (`None` for an unscoped fleet pause).
    pub platform: Option<String>,
}

impl PlatformPause {
    /// The typed report reason the cycle carries.
    pub fn report_reason(&self, platform: &str) -> String {
        if self.reason.trim().is_empty() {
            format!("scheduling paused for {platform}")
        } else {
            format!("scheduling paused for {platform}: {}", self.reason.trim())
        }
    }
}

/// Whether `platform` is paused by the latest applicable record in `path`.
///
/// `Ok(None)` means not paused (including a missing file). An unreadable
/// register is an error so the caller refuses the cycle instead of assuming
/// scheduling is allowed.
pub fn platform_pause(
    path: Option<&Path>,
    platform: &str,
) -> Result<Option<PlatformPause>, String> {
    let Some(path) = path else {
        return Ok(None);
    };
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("interventions at {}: {error}", path.display())),
    };
    let wanted = platform.trim().to_ascii_lowercase();
    let mut latest: Option<(bool, PlatformPause)> = None;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Intervention>(line) else {
            // A malformed line carries no authority; the next valid record
            // still decides. The control plane reports its own corrupt count.
            continue;
        };
        let scoped = record
            .platform
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(scope) = scoped {
            if !scope.eq_ignore_ascii_case(&wanted) {
                continue;
            }
        }
        let paused = match record.kind.as_str() {
            "pause_scheduling" => true,
            "resume_scheduling" => false,
            _ => continue,
        };
        let candidate = PlatformPause {
            reason: record.reason,
            at_ms: record.at_ms,
            platform: scoped.map(str::to_string),
        };
        // Later lines win ties: an append-only register's order is its clock.
        if latest
            .as_ref()
            .is_none_or(|(_, current)| candidate.at_ms >= current.at_ms)
        {
            latest = Some((paused, candidate));
        }
    }
    Ok(match latest {
        Some((true, pause)) => Some(pause),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::io::Write as _;

    use super::*;

    fn register(lines: &[&str]) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "apex-interventions-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("interventions.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        (dir, path)
    }

    fn line(kind: &str, platform: Option<&str>, at_ms: i64, reason: &str) -> String {
        serde_json::json!({
            "interventionId": "itv_test",
            "kind": kind,
            "platform": platform,
            "customerPopulation": "all",
            "reason": reason,
            "expectedEffect": "writes stop",
            "actor": "maya",
            "atMs": at_ms,
            "correlationId": "c1",
        })
        .to_string()
    }

    #[test]
    fn a_missing_file_is_not_paused() {
        let dir =
            std::env::temp_dir().join(format!("apex-interventions-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("interventions.jsonl");
        assert_eq!(platform_pause(Some(&path), "applovin_max").unwrap(), None);
        assert_eq!(platform_pause(None, "applovin_max").unwrap(), None);
    }

    #[test]
    fn the_latest_pause_wins_and_a_resume_clears_it() {
        let (dir, path) = register(&[
            &line("pause_scheduling", Some("applovin_max"), 10, "incident"),
            &line("pause_scheduling", Some("applovin_max"), 20, "still broken"),
        ]);
        let pause = platform_pause(Some(&path), "applovin_max")
            .unwrap()
            .expect("paused");
        assert_eq!(pause.reason, "still broken");
        assert_eq!(pause.at_ms, 20);
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n",
                line("pause_scheduling", Some("applovin_max"), 10, "incident"),
                line("resume_scheduling", Some("applovin_max"), 20, "recovered")
            ),
        )
        .unwrap();
        assert_eq!(platform_pause(Some(&path), "applovin_max").unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_platform_pause_only_affects_that_platform_and_unscoped_pauses_apply_to_all() {
        let (dir, path) = register(&[
            &line("pause_scheduling", Some("applovin_max"), 10, "max incident"),
            &line("resume_scheduling", None, 20, "fleet resumes"),
            &line("pause_scheduling", None, 30, "fleet incident"),
        ]);
        assert_eq!(
            platform_pause(Some(&path), "levelplay")
                .unwrap()
                .unwrap()
                .at_ms,
            30
        );
        assert_eq!(
            platform_pause(Some(&path), "admob").unwrap().unwrap().at_ms,
            30
        );
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n",
                line("pause_scheduling", Some("applovin_max"), 10, "max incident"),
                line("resume_scheduling", Some("levelplay"), 20, "levelplay only")
            ),
        )
        .unwrap();
        assert!(platform_pause(Some(&path), "applovin_max")
            .unwrap()
            .is_some());
        assert_eq!(platform_pause(Some(&path), "levelplay").unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_resume_newer_than_a_pause_clears_it_and_malformed_lines_are_ignored() {
        let (dir, path) = register(&[
            &line("pause_scheduling", Some("admob"), 10, "incident"),
            "{not json",
            &line("mystery_kind", Some("admob"), 40, "not an intervention"),
            &line("resume_scheduling", Some("admob"), 30, "recovered"),
        ]);
        assert_eq!(platform_pause(Some(&path), "admob").unwrap(), None);
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n",
                "not json at all",
                line("pause_scheduling", Some("admob"), 10, "incident")
            ),
        )
        .unwrap();
        assert!(platform_pause(Some(&path), "admob").unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_platform_matches_case_insensitively() {
        let (dir, path) = register(&[&line(
            "pause_scheduling",
            Some("AppLovin_MAX"),
            10,
            "incident",
        )]);
        assert!(platform_pause(Some(&path), "applovin_max")
            .unwrap()
            .is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
