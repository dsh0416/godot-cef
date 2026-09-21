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
static DEVTOOLS_PORT: Mutex<Option<PortReservation>> = Mutex::new(None);

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

        let (parent, stem) = profile_location(base);

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

struct PortReservation {
    port: u16,
    _lock: ProcessLock,
    listener: Option<std::net::TcpListener>,
}

/// Reserves a localhost devtools port for this process.
///
/// The TCP socket stays open until [`release_devtools_listener`], so another
/// process cannot bind it before CEF does. A lock file in a directory shared by
/// every instance records the choice across that handoff. Returns `None` when
/// no port in the wrapped search is free. Port 0 is skipped.
pub fn claim_devtools_port(base: &Path, start: u16, attempts: u16) -> Option<u16> {
    let mut guard = match DEVTOOLS_PORT.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(existing) = guard.as_ref() {
        return Some(existing.port);
    }

    let reserved = reserve_port(&devtools_dir(base), start, attempts)?;
    let port = reserved.port;
    *guard = Some(reserved);
    Some(port)
}

/// Closes the probe socket immediately before `cef::initialize` binds the port.
/// The lock file stays held so another Godot process will not select it.
pub fn release_devtools_listener() {
    let mut guard = match DEVTOOLS_PORT.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(reservation) = guard.as_mut() {
        reservation.listener.take();
    }
}

/// Drops the devtools reservation after CEF initialization fails.
pub fn release_devtools_port() {
    let mut guard = match DEVTOOLS_PORT.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = None;
}

fn reserve_port(dir: &Path, start: u16, attempts: u16) -> Option<PortReservation> {
    if fs::create_dir_all(dir).is_err() {
        return None;
    }

    let mut offset = 0u16;
    let mut tried = 0u16;
    while tried < attempts {
        let port = start.wrapping_add(offset);
        offset = offset.wrapping_add(1);
        tried = tried.saturating_add(1);
        if port == 0 {
            continue;
        }

        let Some(lock) = try_lock_file(&dir.join(format!("{port}.lock"))) else {
            continue;
        };
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                return Some(PortReservation {
                    port,
                    _lock: lock,
                    listener: Some(listener),
                });
            }
            Err(_) => {}
        }
    }
    None
}

fn profile_location(base: &Path) -> (PathBuf, String) {
    let parent = base
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let stem = base
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("cef-data")
        .to_string();
    (parent, stem)
}

fn devtools_dir(base: &Path) -> PathBuf {
    let (parent, stem) = profile_location(base);
    parent.join(format!("{stem}-devtools"))
}

fn try_acquire(path: &Path) -> Option<ProcessLock> {
    if fs::create_dir_all(path).is_err() {
        return None;
    }
    try_lock_file(&path.join(".godot-cef.lock"))
}

fn try_lock_file(path: &Path) -> Option<ProcessLock> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
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
    fn reserve_port_skips_a_bound_port() {
        let held = match std::net::TcpListener::bind(("127.0.0.1", 0)) {
            Ok(listener) => listener,
            Err(_) => return,
        };
        let Ok(addr) = held.local_addr() else {
            return;
        };
        let root = scratch("ports");
        let _ = fs::remove_dir_all(&root);
        let Some(reserved) = reserve_port(&root, addr.port(), 8) else {
            let _ = fs::remove_dir_all(&root);
            return;
        };

        assert_ne!(reserved.port, addr.port());
        assert_ne!(reserved.port, 0);
        drop(reserved);
        drop(held);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reserve_port_wraps_past_u16_max() {
        let held = match std::net::TcpListener::bind(("127.0.0.1", u16::MAX)) {
            Ok(listener) => listener,
            Err(_) => return,
        };
        let root = scratch("port-max");
        let _ = fs::remove_dir_all(&root);
        let Some(reserved) = reserve_port(&root, u16::MAX, 8) else {
            drop(held);
            let _ = fs::remove_dir_all(&root);
            return;
        };

        assert_ne!(reserved.port, u16::MAX);
        assert_ne!(reserved.port, 0);
        drop(reserved);
        drop(held);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn second_port_reservation_uses_a_different_port() {
        let root = scratch("port-pair");
        let _ = fs::remove_dir_all(&root);
        let Some(first) = reserve_port(&root, 20000, 32) else {
            let _ = fs::remove_dir_all(&root);
            return;
        };
        let Some(second) = reserve_port(&root, first.port, 32) else {
            drop(first);
            let _ = fs::remove_dir_all(&root);
            return;
        };

        assert_ne!(first.port, second.port);
        drop(second);
        drop(first);
        let _ = fs::remove_dir_all(&root);
    }
}
