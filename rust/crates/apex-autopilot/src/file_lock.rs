//! One advisory pass lock for read-modify-replace state files.
//!
//! A single writer gets atomic rename semantics; two concurrent workers do
//! not. This lock serialises the *sequence* (`load -> mutate -> rename`) with
//! `create_new` (atomic first-holder), a timestamped body so a crashed holder
//! can be stolen only after the stale window, and a `Drop` release. It is a
//! fence for cooperating Apex processes, not a filesystem journal.

use std::path::{Path, PathBuf};

/// How long a held state lock is trusted before it is treated as a crashed
/// predecessor and stolen. Longer than any single save takes.
const LOCK_STALE_MS: i64 = 6 * 60 * 60 * 1_000;

/// One inter-process lock guarding a state file's save sequence.
#[derive(Debug)]
pub struct StateLock {
    path: PathBuf,
}

impl StateLock {
    /// Hold the lock for `target`, or refuse while another live process holds
    /// it. A lock older than [`LOCK_STALE_MS`] is a crashed predecessor's and
    /// is replaced once.
    pub fn acquire(target: &Path) -> Result<Self, String> {
        if let Some(parent) = target.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("{}: parent directory: {error}", parent.display()))?;
            }
        }
        let path = {
            let mut name = target.as_os_str().to_os_string();
            name.push(".lock");
            PathBuf::from(name)
        };
        for attempt in 0..2 {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    use std::io::Write as _;
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|since| since.as_millis() as i128)
                        .unwrap_or(0);
                    let _ = write!(file, "{} {now_ms}", std::process::id());
                    let _ = file.sync_all();
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if attempt == 0 && Self::is_stale(&path) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    let holder = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|raw| raw.split_whitespace().next().map(str::to_string))
                        .unwrap_or_else(|| "an unknown process".to_string());
                    return Err(format!("state {} is locked by {holder}", target.display()));
                }
                Err(error) => {
                    return Err(format!(
                        "state lock {} could not be created: {error}",
                        path.display()
                    ));
                }
            }
        }
        Err(format!(
            "state lock {} could not be acquired",
            path.display()
        ))
    }

    /// Whether the lock body (or, unparseably, its mtime) is older than the
    /// stale window. An unreadable lock fails closed: never stolen on a guess.
    fn is_stale(path: &Path) -> bool {
        let age = std::fs::read_to_string(path).ok().and_then(|raw| {
            raw.split_whitespace()
                .nth(1)
                .and_then(|stamp| stamp.parse::<i64>().ok())
                .and_then(|started| {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|now| now.as_millis() as i64 - started)
                })
        });
        match age {
            Some(age) => age > LOCK_STALE_MS,
            None => std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|mtime| {
                    let started = mtime
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|since| since.as_millis() as i64)?;
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|since| since.as_millis() as i64)?;
                    Some(now - started)
                })
                .is_some_and(|age| age > LOCK_STALE_MS),
        }
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Run one read-modify-replace save under the target's state lock. Any error
/// from `body` propagates; the lock is released on every exit.
pub fn save_locked<T>(
    target: &Path,
    body: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _lock = StateLock::acquire(target)?;
    body()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_file_is_saved_by_one_process_at_a_time() {
        let target = std::env::temp_dir().join(format!(
            "apex-state-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.subsec_nanos())
                .unwrap_or_default()
        ));
        let held = StateLock::acquire(&target).expect("first hold");
        let second = StateLock::acquire(&target).expect_err("a second saver refuses");
        assert!(second.contains("locked by"), "{second}");
        drop(held);
        let saved = save_locked(&target, || Ok(7)).expect("save under lock");
        assert_eq!(saved, 7);
        // A crashed holder's lock is stolen only past the stale window.
        let mut lock_path = target.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock_path = PathBuf::from(lock_path);
        std::fs::write(
            &lock_path,
            format!(
                "999999 {}",
                (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64)
                    - LOCK_STALE_MS
                    - 1_000
            ),
        )
        .expect("stale lock");
        let stolen = StateLock::acquire(&target).expect("stale lock is stolen");
        drop(stolen);
        let _ = std::fs::remove_file(&target);
    }
}
