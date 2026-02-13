//! Cookie & session management for CEF integration.
//!
//! This module provides CEF callback implementations that bridge CEF's
//! asynchronous cookie APIs back to Godot's main-thread event loop via
//! the shared `EventQueues`.

use cef::{self, *};
use std::sync::{Arc, Mutex};

use crate::browser::EventQueuesHandle;

// ── Data types ──────────────────────────────────────────────────────────────

/// A cookie's data, extracted from CEF's `Cookie` struct for safe cross-thread transfer.
#[derive(Debug, Clone)]
pub struct CookieData {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub httponly: bool,
    pub same_site: CookieSameSite,
    pub has_expires: bool,
}

impl CookieData {
    /// Converts a CEF `Cookie` into our owned `CookieData`.
    fn from_cef(cookie: &cef::Cookie) -> Self {
        Self {
            name: cookie.name.to_string(),
            value: cookie.value.to_string(),
            domain: cookie.domain.to_string(),
            path: cookie.path.to_string(),
            secure: cookie.secure != 0,
            httponly: cookie.httponly != 0,
            same_site: cookie.same_site,
            has_expires: cookie.has_expires != 0,
        }
    }
}

/// Events emitted by cookie operations, consumed by the Godot main thread.
#[derive(Debug, Clone)]
pub enum CookieEvent {
    /// Cookies retrieved by a visitor (may be empty if no cookies matched).
    Received(Vec<CookieData>),
    /// Result of a `set_cookie` call.
    Set(bool),
    /// Result of a `delete_cookies` call, with the number of cookies deleted.
    Deleted(i32),
    /// Cookie store was flushed to disk.
    Flushed,
}

// ── Cookie collector (handles the 0-cookies edge case via Drop) ─────────────

/// Accumulates cookies during a `visit_all_cookies` / `visit_url_cookies` call.
///
/// When the visitor finishes (either after the last cookie or when CEF drops
/// the visitor reference for a 0-cookie result), the `Drop` impl pushes
/// whatever was collected into the event queue.
struct CookieCollector {
    cookies: Vec<CookieData>,
    event_queues: EventQueuesHandle,
    flushed: bool,
}

impl CookieCollector {
    fn flush(&mut self) {
        if !self.flushed {
            if let Ok(mut queues) = self.event_queues.lock() {
                queues
                    .cookie_events
                    .push_back(CookieEvent::Received(std::mem::take(&mut self.cookies)));
            }
            self.flushed = true;
        }
    }
}

impl Drop for CookieCollector {
    fn drop(&mut self) {
        self.flush();
    }
}

type CollectorHandle = Arc<Mutex<CookieCollector>>;

// ── CookieVisitor ───────────────────────────────────────────────────────────

wrap_cookie_visitor! {
    pub(crate) struct CookieVisitorImpl {
        collector: CollectorHandle,
    }

    impl CookieVisitor {
        fn visit(
            &self,
            cookie: Option<&Cookie>,
            count: ::std::os::raw::c_int,
            total: ::std::os::raw::c_int,
            _delete_cookie: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            if let Ok(mut collector) = self.collector.lock() {
                if let Some(c) = cookie {
                    collector.cookies.push(CookieData::from_cef(c));
                }
                // Flush on the last cookie (or if total == 0, this won't be called —
                // Drop handles that case)
                if count >= total - 1 {
                    collector.flush();
                }
            }
            true as _ // continue visiting
        }
    }
}

impl CookieVisitorImpl {
    pub fn build(event_queues: EventQueuesHandle) -> CookieVisitor {
        let collector = Arc::new(Mutex::new(CookieCollector {
            cookies: Vec::new(),
            event_queues,
            flushed: false,
        }));
        Self::new(collector)
    }
}

// ── SetCookieCallback ───────────────────────────────────────────────────────

wrap_set_cookie_callback! {
    pub(crate) struct SetCookieCallbackImpl {
        event_queues: EventQueuesHandle,
    }

    impl SetCookieCallback {
        fn on_complete(&self, success: ::std::os::raw::c_int) {
            if let Ok(mut queues) = self.event_queues.lock() {
                queues.cookie_events.push_back(CookieEvent::Set(success != 0));
            }
        }
    }
}

impl SetCookieCallbackImpl {
    pub fn build(event_queues: EventQueuesHandle) -> SetCookieCallback {
        Self::new(event_queues)
    }
}

// ── DeleteCookiesCallback ───────────────────────────────────────────────────

wrap_delete_cookies_callback! {
    pub(crate) struct DeleteCookiesCallbackImpl {
        event_queues: EventQueuesHandle,
    }

    impl DeleteCookiesCallback {
        fn on_complete(&self, num_deleted: ::std::os::raw::c_int) {
            if let Ok(mut queues) = self.event_queues.lock() {
                queues
                    .cookie_events
                    .push_back(CookieEvent::Deleted(num_deleted));
            }
        }
    }
}

impl DeleteCookiesCallbackImpl {
    pub fn build(event_queues: EventQueuesHandle) -> DeleteCookiesCallback {
        Self::new(event_queues)
    }
}

// ── FlushCookieStoreCallback (reuses CompletionCallback) ────────────────────

wrap_completion_callback! {
    pub(crate) struct FlushCookieStoreCallbackImpl {
        event_queues: EventQueuesHandle,
    }

    impl CompletionCallback {
        fn on_complete(&self) {
            if let Ok(mut queues) = self.event_queues.lock() {
                queues.cookie_events.push_back(CookieEvent::Flushed);
            }
        }
    }
}

impl FlushCookieStoreCallbackImpl {
    pub fn build(event_queues: EventQueuesHandle) -> CompletionCallback {
        Self::new(event_queues)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cookie_event_variants() {
        let received = CookieEvent::Received(vec![CookieData {
            name: "session".into(),
            value: "abc123".into(),
            domain: ".example.com".into(),
            path: "/".into(),
            secure: true,
            httponly: false,
            same_site: CookieSameSite::NO_RESTRICTION,
            has_expires: false,
        }]);
        assert!(matches!(received, CookieEvent::Received(ref v) if v.len() == 1));

        let set = CookieEvent::Set(true);
        assert!(matches!(set, CookieEvent::Set(true)));

        let deleted = CookieEvent::Deleted(5);
        assert!(matches!(deleted, CookieEvent::Deleted(5)));

        let flushed = CookieEvent::Flushed;
        assert!(matches!(flushed, CookieEvent::Flushed));
    }

    #[test]
    fn test_cookie_event_received_empty() {
        let received = CookieEvent::Received(Vec::new());
        assert!(matches!(received, CookieEvent::Received(ref v) if v.is_empty()));
    }

    #[test]
    fn test_cookie_data_defaults() {
        let cookie = CookieData {
            name: String::new(),
            value: String::new(),
            domain: String::new(),
            path: String::new(),
            secure: false,
            httponly: false,
            same_site: CookieSameSite::NO_RESTRICTION,
            has_expires: false,
        };
        assert!(!cookie.secure);
        assert!(!cookie.httponly);
        assert!(!cookie.has_expires);
    }
}
