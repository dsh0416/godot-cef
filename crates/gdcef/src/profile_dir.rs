//! Per-process CEF profile directory.
//!
//! Chromium allows only one browser process to use a given `root_cache_path`.
//! Every browser inside that process shares the directory, so cookies and
//! localStorage stay shared across tabs. A second Godot process takes the next
//! free sibling directory instead of exiting.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const INSTANCE_SLOTS: u32 = 16;

static PROCESS_PROFILE: Mutex<Option<ProfileClaim>> = Mutex::new(None);

pub struct ActiveProfile {
    pub path: PathBuf,
    pub primary: bool,
}

pub struct ProfileClaim {
    pub path: PathBuf,
    pub primary: bool,
    _lock: Option<ProcessLock>,
}

struct ProcessLock {
    _file: std::fs::File,
}

impl ProfileClaim {
    /// Claims `base` when this process is the first user of that directory.
    /// Otherwise claims `<base>-instance-N` beside it.
    pub fn claim(base: &Path) -> Self {
        if let Some(lock) = try_acquire(base) {
            return Self {
                path: base.to_path_buf(),
                primary: true,
                _lock: Some(lock),
            };
        }

        let parent = base
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let stem = base
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("cef-data");

        for index in 1..=INSTANCE_SLOTS {
            let candidate = parent.join(format!("{stem}-instance-{index}"));
            if let Some(lock) = try_acquire(&candidate) {
                return Self {
                    path: candidate,
                    primary: false,
                    _lock: Some(lock),
                };
            }
        }

        let fallback = parent.join(format!("{stem}-instance-{}", std::process::id()));
        let lock = try_acquire(&fallback);
        Self {
            path: fallback,
            primary: false,
            _lock: lock,
        }
    }
}

/// Claims a profile for this process. Later calls return the same directory.
pub fn claim_process_profile(base: &Path) -> ActiveProfile {
    let mut guard = match PROCESS_PROFILE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(existing) = guard.as_ref() {
        return ActiveProfile {
            path: existing.path.clone(),
            primary: existing.primary,
        };
    }

    let claim = ProfileClaim::claim(base);
    let active = ActiveProfile {
        path: claim.path.clone(),
        primary: claim.primary,
    };
    *guard = Some(claim);
    active
}

/// Returns the first port in `start..start+attempts` that can be bound.
/// Returns `start` when none of those ports are free.
pub fn pick_available_port(start: u16, attempts: u16) -> u16 {
    let mut offset = 0u16;
    while offset < attempts {
        let Some(port) = start.checked_add(offset) else {
            break;
        };
        if port != 0 && std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
        offset = offset.saturating_add(1);
    }
    start
}

fn try_acquire(path: &Path) -> Option<ProcessLock> {
    if fs::create_dir_all(path).is_err() {
        return None;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.join(".godot-cef.lock"))
        .ok()?;
    // Released when the file is closed at the end of this process.
    if file.try_lock().is_err() {
        return None;
    }
    Some(ProcessLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("godot-cef-{label}-{}", std::process::id()))
    }

    #[test]
    fn second_claim_uses_a_sibling_directory() {
        let root = scratch("slots");
        let _ = fs::remove_dir_all(&root);
        let base = root.join("cef-data");

        let first = ProfileClaim::claim(&base);
        let second = ProfileClaim::claim(&base);

        assert!(first.primary);
        assert_eq!(first.path, base);
        assert!(!second.primary);
        assert_eq!(second.path, root.join("cef-data-instance-1"));

        drop(second);
        drop(first);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn released_primary_directory_can_be_claimed_again() {
        let root = scratch("reclaim");
        let _ = fs::remove_dir_all(&root);
        let base = root.join("cef-data");

        let first = ProfileClaim::claim(&base);
        drop(first);
        let again = ProfileClaim::claim(&base);

        assert!(again.primary);
        assert_eq!(again.path, base);

        drop(again);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pick_available_port_skips_a_bound_port() {
        let held = match std::net::TcpListener::bind(("127.0.0.1", 0)) {
            Ok(listener) => listener,
            Err(_) => return,
        };
        let Ok(addr) = held.local_addr() else {
            return;
        };
        let port = addr.port();
        if port > 65528 {
            return;
        }

        let chosen = pick_available_port(port, 8);
        assert_ne!(chosen, port);
        assert!(chosen > port);
    }
}
