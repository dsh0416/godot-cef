//! Browser-scoped custom dialogs. CEF continuations run after releasing locks.
//!
//! CEF calls OnDialogClosed and OnResetDialogState synchronously from a custom
//! JS continuation. Reset does not disconnect the client's retained callback:
//! cefclient drops it (which cancels), so we explicitly complete it as canceled.
//! https://github.com/chromiumembedded/cef/blob/master/libcef/browser/javascript_dialog_manager.cc
//! https://github.com/chromiumembedded/cef/blob/master/tests/cefclient/browser/dialog_handler_gtk.cc

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use cef::{ImplFileDialogCallback, ImplJsdialogCallback};

use crate::browser::{
    EventQueuesHandle, FileDialogRequestEvent, JsDialogRequestEvent, dialog_policy,
};

static NEXT_REQUEST_ID: AtomicI64 = AtomicI64::new(1);
const MAX_FILE_DIALOGS: usize = 16;
const MAX_JS_DIALOGS: usize = 1;

thread_local! {
    static CONTROLLERS: RefCell<Vec<Weak<ControllerInner>>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone)]
pub(crate) struct DialogController {
    inner: Arc<ControllerInner>,
}

struct ControllerInner {
    state: Mutex<DialogState<CefCallback>>,
    event_queues: EventQueuesHandle,
}

enum CefCallback {
    File(cef::FileDialogCallback),
    Js(cef::JsdialogCallback),
}

impl CefCallback {
    fn finish(self, decision: Decision) {
        match (self, decision) {
            (Self::File(callback), Decision::File(paths)) => {
                let mut list = cef::CefStringList::new();
                if paths.iter().all(|path| list.append(path)) {
                    callback.cont(Some(&mut list));
                } else {
                    callback.cancel();
                }
            }
            (Self::File(callback), _) => callback.cancel(),
            (Self::Js(callback), Decision::Js(accepted, text)) => {
                callback.cont(accepted as i32, Some(&text.as_str().into()));
            }
            (Self::Js(callback), _) => callback.cont(0, None),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Native delegates to CEF; Suppress is only valid for ordinary JS dialogs and
/// must return false with suppress_message=true, without continuing its callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogDisposition {
    Native,
    Handled,
    Suppress,
}

impl DialogController {
    pub(crate) fn new(event_queues: EventQueuesHandle, policy: i32, timeout: Duration) -> Self {
        let inner = Arc::new(ControllerInner {
            state: Mutex::new(DialogState::new(policy, timeout)),
            event_queues,
        });
        CONTROLLERS.with_borrow_mut(|controllers| {
            controllers.retain(|controller| controller.strong_count() > 0);
            controllers.push(Arc::downgrade(&inner));
        });
        Self { inner }
    }

    pub(crate) fn request_file(
        &self,
        callback: cef::FileDialogCallback,
        event: FileDialogRequestEvent,
    ) -> DialogDisposition {
        self.request(CefCallback::File(callback), RequestEvent::File(event))
    }

    pub(crate) fn request_js(
        &self,
        callback: cef::JsdialogCallback,
        event: JsDialogRequestEvent,
    ) -> DialogDisposition {
        self.request(CefCallback::Js(callback), RequestEvent::Js(event))
    }

    fn request(&self, callback: CefCallback, event: RequestEvent) -> DialogDisposition {
        let now = Instant::now();
        // An expired JS continuation synchronously resets CEF's dialog state.
        // Complete it before inserting this request, or its reset would cancel
        // the new callback. Both state updates use the request's arrival time.
        self.update(|state| ((), state.expire(now)));
        self.update(|state| state.request(callback, event, now))
    }

    pub(crate) fn resolve_file(&self, id: i64, paths: Vec<String>) -> bool {
        self.update(|state| state.resolve_file(id, paths, Instant::now()))
    }

    pub(crate) fn resolve_js(&self, id: i64, accepted: bool, text: String) -> bool {
        self.update(|state| state.resolve_js(id, accepted, text, Instant::now()))
    }

    pub(crate) fn cancel_file(&self, id: i64) -> bool {
        self.update(|state| state.cancel(id, Some(true), "canceled", Instant::now()))
    }

    pub(crate) fn cancel_js(&self, id: i64) -> bool {
        self.update(|state| state.cancel(id, Some(false), "canceled", Instant::now()))
    }

    pub(crate) fn dismiss_request(&self, id: i64, reason: &str) -> bool {
        self.update(|state| state.cancel(id, None, reason, Instant::now()))
    }

    pub(crate) fn is_pending(&self, id: i64) -> bool {
        self.update(|state| {
            let update = state.expire(Instant::now());
            (state.pending.contains_key(&id), update)
        })
    }

    pub(crate) fn set_policy(&self, policy: i32) {
        self.update(|state| {
            let policy = normalize_policy(policy);
            let update = if policy != state.policy {
                state.policy = policy;
                state.cancel_where(|_| true, "policy_changed")
            } else {
                Update::default()
            };
            ((), update)
        });
    }

    pub(crate) fn cancel_all(&self, reason: &str) {
        self.update(|state| ((), state.cancel_where(|_| true, reason)));
    }

    pub(crate) fn reset_js(&self, reason: &str) {
        self.update(|state| {
            (
                (),
                state.cancel_where(|kind| matches!(kind, Kind::Js { .. }), reason),
            )
        });
    }

    pub(crate) fn close(&self, reason: &str) {
        self.update(|state| ((), state.close(reason)));
    }

    fn expire(&self) {
        self.update(|state| ((), state.expire(Instant::now())));
    }

    fn update<T>(
        &self,
        change: impl FnOnce(&mut DialogState<CefCallback>) -> (T, Update<CefCallback>),
    ) -> T {
        let (result, update) = {
            let mut state = lock(&self.inner.state);
            let (result, update) = change(&mut state);
            if !update.events.is_empty() || !update.completions.is_empty() {
                let mut queues = lock(&self.inner.event_queues);
                let removed: HashSet<_> = update
                    .completions
                    .iter()
                    .filter_map(|completion| completion.id)
                    .collect();
                queues
                    .file_dialog_requests
                    .retain(|event| !removed.contains(&event.request_id));
                queues
                    .js_dialog_requests
                    .retain(|event| !removed.contains(&event.request_id));
                for event in &update.events {
                    match event {
                        RequestEvent::File(event) => {
                            queues.file_dialog_requests.push_back(event.clone())
                        }
                        RequestEvent::Js(event) => {
                            queues.js_dialog_requests.push_back(event.clone())
                        }
                    }
                }
                for completion in &update.completions {
                    if let Some(id) = completion.id {
                        queues
                            .dialog_request_finished
                            .push_back((id, completion.reason.to_owned()));
                    }
                }
            }
            (result, update)
        };
        // Also release unhandled/native references outside locks: the final CEF
        // callback reference can cancel a dialog from its destructor.
        drop(update.discarded);
        for completion in update.completions {
            completion.callback.finish(completion.decision);
        }
        result
    }
}

/// The central CEF pump enforces deadlines even while individual nodes pause.
pub(crate) fn expire_pending() {
    let controllers = CONTROLLERS.with_borrow_mut(|registered| {
        let active: Vec<_> = registered.iter().filter_map(Weak::upgrade).collect();
        registered.retain(|controller| controller.strong_count() > 0);
        active
    });
    for inner in controllers {
        DialogController { inner }.expire();
    }
}

fn normalize_policy(policy: i32) -> i32 {
    match policy {
        dialog_policy::SIGNAL | dialog_policy::CANCEL => policy,
        _ => dialog_policy::NATIVE,
    }
}

#[derive(Clone)]
enum RequestEvent {
    File(FileDialogRequestEvent),
    Js(JsDialogRequestEvent),
}

impl RequestEvent {
    fn kind(&self) -> Kind {
        match self {
            Self::File(event) => Kind::File { mode: event.mode },
            Self::Js(event) => Kind::Js {
                prompt: event.kind == "prompt",
                before_unload: event.kind == "before_unload",
            },
        }
    }

    fn set_id(&mut self, id: i64) {
        match self {
            Self::File(event) => event.request_id = id,
            Self::Js(event) => event.request_id = id,
        }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    File { mode: i32 },
    Js { prompt: bool, before_unload: bool },
}

impl Kind {
    fn is_file(self) -> bool {
        matches!(self, Self::File { .. })
    }
    fn can_suppress(self) -> bool {
        matches!(
            self,
            Self::Js {
                before_unload: false,
                ..
            }
        )
    }
}

struct Pending<C> {
    callback: C,
    kind: Kind,
    deadline: Instant,
}

struct DialogState<C> {
    policy: i32,
    timeout: Duration,
    closed: bool,
    pending: HashMap<i64, Pending<C>>,
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Cancel,
    File(Vec<String>),
    Js(bool, String),
}

struct Completion<C> {
    id: Option<i64>,
    callback: C,
    decision: Decision,
    reason: String,
}

struct Update<C> {
    events: Vec<RequestEvent>,
    completions: Vec<Completion<C>>,
    discarded: Vec<C>,
}

impl<C> Default for Update<C> {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            completions: Vec::new(),
            discarded: Vec::new(),
        }
    }
}

impl<C> DialogState<C> {
    fn new(policy: i32, timeout: Duration) -> Self {
        Self {
            policy: normalize_policy(policy),
            timeout,
            closed: false,
            pending: HashMap::new(),
        }
    }

    fn request(
        &mut self,
        callback: C,
        mut event: RequestEvent,
        now: Instant,
    ) -> (DialogDisposition, Update<C>) {
        // The controller finishes expiry callbacks before calling this method;
        // never bundle an old JS completion with registration of a new dialog.
        let mut update = Update::default();
        let kind = event.kind();
        if !self.closed && self.policy == dialog_policy::NATIVE {
            update.discarded.push(callback);
            return (DialogDisposition::Native, update);
        }
        let limit = if kind.is_file() {
            MAX_FILE_DIALOGS
        } else {
            MAX_JS_DIALOGS
        };
        let pending_count = self
            .pending
            .values()
            .filter(|pending| pending.kind.is_file() == kind.is_file())
            .count();
        if self.closed || self.policy == dialog_policy::CANCEL || pending_count >= limit {
            if kind.can_suppress() {
                update.discarded.push(callback);
                return (DialogDisposition::Suppress, update);
            }
            update.completions.push(Completion {
                id: None,
                callback,
                decision: Decision::Cancel,
                reason: "canceled".into(),
            });
            return (DialogDisposition::Handled, update);
        }
        let id = NEXT_REQUEST_ID.try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        });
        let Ok(id) = id else {
            update.completions.push(Completion {
                id: None,
                callback,
                decision: Decision::Cancel,
                reason: "canceled".into(),
            });
            return (DialogDisposition::Handled, update);
        };
        event.set_id(id);
        self.pending.insert(
            id,
            Pending {
                callback,
                kind,
                deadline: now + self.timeout,
            },
        );
        update.events.push(event);
        (DialogDisposition::Handled, update)
    }

    fn resolve_file(&mut self, id: i64, paths: Vec<String>, now: Instant) -> (bool, Update<C>) {
        let mut update = self.expire(now);
        let Some(Pending {
            kind: Kind::File { mode },
            ..
        }) = self.pending.get(&id)
        else {
            return (false, update);
        };
        if !paths.is_empty() && !valid_file_paths(*mode, &paths) {
            return (false, update);
        }
        let (decision, reason) = if paths.is_empty() {
            (Decision::Cancel, "canceled")
        } else {
            (Decision::File(paths), "accepted")
        };
        self.complete(id, decision, reason, &mut update);
        (true, update)
    }

    fn resolve_js(
        &mut self,
        id: i64,
        accepted: bool,
        text: String,
        now: Instant,
    ) -> (bool, Update<C>) {
        let mut update = self.expire(now);
        let Some(Pending {
            kind: Kind::Js { prompt, .. },
            ..
        }) = self.pending.get(&id)
        else {
            return (false, update);
        };
        let text = if *prompt && accepted {
            text
        } else {
            String::new()
        };
        self.complete(
            id,
            Decision::Js(accepted, text),
            if accepted { "accepted" } else { "canceled" },
            &mut update,
        );
        (true, update)
    }

    fn cancel(
        &mut self,
        id: i64,
        file_kind: Option<bool>,
        reason: &str,
        now: Instant,
    ) -> (bool, Update<C>) {
        let mut update = self.expire(now);
        let Some(pending) = self.pending.get(&id) else {
            return (false, update);
        };
        if file_kind.is_some_and(|file| file != pending.kind.is_file()) {
            return (false, update);
        }
        self.complete(id, Decision::Cancel, reason, &mut update);
        (true, update)
    }

    fn complete(&mut self, id: i64, decision: Decision, reason: &str, update: &mut Update<C>) {
        if let Some(pending) = self.pending.remove(&id) {
            update.completions.push(Completion {
                id: Some(id),
                callback: pending.callback,
                decision,
                reason: reason.into(),
            });
        }
    }

    fn cancel_where(&mut self, predicate: impl Fn(Kind) -> bool, reason: &str) -> Update<C> {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(id, pending)| predicate(pending.kind).then_some(*id))
            .collect();
        let mut update = Update::default();
        for id in ids {
            self.complete(id, Decision::Cancel, reason, &mut update);
        }
        update
    }

    fn close(&mut self, reason: &str) -> Update<C> {
        self.closed = true;
        self.cancel_where(|_| true, reason)
    }

    fn expire(&mut self, now: Instant) -> Update<C> {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(id, pending)| (now >= pending.deadline).then_some(*id))
            .collect();
        let mut update = Update::default();
        for id in ids {
            self.complete(id, Decision::Cancel, "timed_out", &mut update);
        }
        update
    }
}

fn valid_file_paths(mode: i32, paths: &[String]) -> bool {
    let multiple = mode == cef::FileDialogMode::OPEN_MULTIPLE.get_raw();
    let known = [
        cef::FileDialogMode::OPEN,
        cef::FileDialogMode::OPEN_MULTIPLE,
        cef::FileDialogMode::OPEN_FOLDER,
        cef::FileDialogMode::SAVE,
    ]
    .iter()
    .any(|candidate| candidate.get_raw() == mode);
    known
        && (multiple || paths.len() == 1)
        && paths.iter().all(|path| {
            !path.trim().is_empty() && !path.contains('\0') && Path::new(path).is_absolute()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cef::rc::{ConvertReturnValue, RcImpl};
    use std::sync::atomic::AtomicBool;

    type JsCalls = Arc<Mutex<Vec<i32>>>;

    struct JsProbe {
        controller: Weak<ControllerInner>,
        calls: JsCalls,
        reentered_without_locks: Arc<AtomicBool>,
    }

    // DLL-side callback types have no client wrapper macro. A Rust-owned CEF
    // refcount/vtable exercises real continuation calls without browser startup.
    fn js_probe(
        controller: &DialogController,
    ) -> (cef::JsdialogCallback, JsCalls, Arc<AtomicBool>) {
        unsafe extern "C" fn continue_js(
            this: *mut cef::sys::cef_jsdialog_callback_t,
            accepted: i32,
            _text: *const cef::sys::cef_string_t,
        ) {
            let probe = unsafe {
                &(*this.cast::<RcImpl<cef::sys::cef_jsdialog_callback_t, JsProbe>>()).interface
            };
            lock(&probe.calls).push(accepted);
            if let Some(inner) = probe.controller.upgrade() {
                let state_unlocked = inner.state.try_lock().is_ok();
                let queues_unlocked = inner.event_queues.try_lock().is_ok();
                if state_unlocked && queues_unlocked {
                    let controller = DialogController { inner };
                    controller.reset_js("dialog_closed");
                    controller.reset_js("reset");
                    probe.reentered_without_locks.store(true, Ordering::Relaxed);
                }
            }
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        let reentered = Arc::new(AtomicBool::new(false));
        let raw = cef::sys::cef_jsdialog_callback_t {
            // RcImpl supplies valid refcount functions for the zeroed base.
            base: unsafe { std::mem::zeroed() },
            cont: Some(continue_js),
        };
        let pointer = RcImpl::new(
            raw,
            JsProbe {
                controller: Arc::downgrade(&controller.inner),
                calls: calls.clone(),
                reentered_without_locks: reentered.clone(),
            },
        );
        (
            pointer
                .cast::<cef::sys::cef_jsdialog_callback_t>()
                .wrap_result(),
            calls,
            reentered,
        )
    }

    fn js_event(kind: &str) -> JsDialogRequestEvent {
        let RequestEvent::Js(event) = js(kind) else {
            unreachable!()
        };
        event
    }

    fn file(mode: cef::FileDialogMode) -> RequestEvent {
        RequestEvent::File(FileDialogRequestEvent {
            request_id: 0,
            mode: mode.get_raw(),
            title: "选择文件".into(),
            default_file_path: String::new(),
            accept_filters: vec!["image/*".into()],
            accept_extensions: vec![".png;.jpg".into()],
            accept_descriptions: vec!["图片".into()],
        })
    }

    fn js(kind: &str) -> RequestEvent {
        RequestEvent::Js(JsDialogRequestEvent {
            request_id: 0,
            kind: kind.into(),
            origin_url: "https://example.test".into(),
            message_text: "确认？".into(),
            default_prompt_text: "你好".into(),
            is_reload: false,
        })
    }

    fn id<C>(update: &Update<C>) -> i64 {
        match &update.events[0] {
            RequestEvent::File(event) => event.request_id,
            RequestEvent::Js(event) => event.request_id,
        }
    }

    fn path() -> String {
        if cfg!(windows) {
            "C:\\浏览器\\文件.txt".into()
        } else {
            "/tmp/浏览器/文件.txt".into()
        }
    }

    #[test]
    fn mode_validation_preserves_request_on_invalid_answer() {
        let now = Instant::now();
        for mode in [
            cef::FileDialogMode::OPEN,
            cef::FileDialogMode::OPEN_MULTIPLE,
            cef::FileDialogMode::OPEN_FOLDER,
            cef::FileDialogMode::SAVE,
        ] {
            let mut state = DialogState::new(dialog_policy::SIGNAL, Duration::from_secs(60));
            let (_, update) = state.request(1, file(mode), now);
            let request_id = id(&update);
            assert!(
                !state
                    .resolve_file(request_id, vec!["relative.txt".into()], now)
                    .0
            );
            assert!(!state.resolve_file(request_id, vec![String::new()], now).0);
            assert!(state.pending.contains_key(&request_id));
            let two_paths = vec![path(), path()];
            assert_eq!(
                state.resolve_file(request_id, two_paths, now).0,
                mode == cef::FileDialogMode::OPEN_MULTIPLE
            );
            if mode != cef::FileDialogMode::OPEN_MULTIPLE {
                assert!(state.resolve_file(request_id, vec![path()], now).0);
            }
        }
    }

    #[test]
    fn stale_duplicate_and_wrong_kind_answers_cannot_consume_request() {
        let now = Instant::now();
        let mut state = DialogState::new(dialog_policy::SIGNAL, Duration::from_secs(60));
        let (_, update) = state.request(11, file(cef::FileDialogMode::OPEN), now);
        let file_id = id(&update);
        let (_, update) = state.request(22, js("prompt"), now);
        let js_id = id(&update);
        assert_ne!(file_id, js_id);
        assert!(!state.resolve_js(file_id, true, String::new(), now).0);
        assert!(!state.resolve_file(js_id, vec![path()], now).0);
        assert!(!state.cancel(file_id, Some(false), "canceled", now).0);
        assert!(!state.cancel(js_id, Some(true), "canceled", now).0);
        assert_eq!(state.pending.len(), 2);
        let (success, update) = state.resolve_js(js_id, true, "你好😀".into(), now);
        assert!(success);
        assert_eq!(
            update.completions[0].decision,
            Decision::Js(true, "你好😀".into())
        );
        assert!(!state.resolve_js(js_id, false, String::new(), now).0);
        assert!(state.cancel(file_id, Some(true), "canceled", now).0);
        assert!(!state.cancel(file_id, None, "canceled", now).0);
        let (_, update) = state.request(33, js("confirm"), now);
        assert!(id(&update) > js_id);
    }

    #[test]
    fn expiry_cancels_once_and_rejects_late_answer() {
        let now = Instant::now();
        let mut state = DialogState::new(dialog_policy::SIGNAL, Duration::from_secs(1));
        let (_, update) = state.request(1, js("confirm"), now);
        let request_id = id(&update);
        assert!(
            state
                .expire(now + Duration::from_millis(999))
                .completions
                .is_empty()
        );
        let (success, update) = state.resolve_js(
            request_id,
            true,
            String::new(),
            now + Duration::from_secs(1),
        );
        assert!(!success);
        assert_eq!(update.completions.len(), 1);
        assert_eq!(update.completions[0].reason, "timed_out");
        assert_eq!(update.completions[0].decision, Decision::Cancel);
        assert!(
            state
                .expire(now + Duration::from_secs(2))
                .completions
                .is_empty()
        );
    }

    #[test]
    fn reset_cancels_js_but_preserves_file_and_close_rejects_new_requests() {
        let now = Instant::now();
        let mut state = DialogState::new(dialog_policy::SIGNAL, Duration::from_secs(60));
        let (_, file_update) = state.request(1, file(cef::FileDialogMode::OPEN), now);
        let (_, js_update) = state.request(2, js("before_unload"), now);
        let reset = state.cancel_where(|kind| !kind.is_file(), "reset");
        assert_eq!(reset.completions[0].id, Some(id(&js_update)));
        assert_eq!(reset.completions[0].reason, "reset");
        assert!(state.pending.contains_key(&id(&file_update)));
        let closed = state.close("browser_closed");
        assert_eq!(closed.completions.len(), 1);
        assert!(state.close("browser_closed").completions.is_empty());
        let (disposition, update) = state.request(3, js("before_unload"), now);
        assert_eq!(disposition, DialogDisposition::Handled);
        assert!(update.events.is_empty());
        assert_eq!(update.completions[0].decision, Decision::Cancel);
    }

    #[test]
    fn native_cancel_and_spam_have_distinct_callback_contracts() {
        let now = Instant::now();
        let mut native = DialogState::new(dialog_policy::NATIVE, Duration::from_secs(60));
        let (disposition, update) = native.request(1, js("alert"), now);
        assert_eq!(disposition, DialogDisposition::Native);
        assert!(update.completions.is_empty());
        assert_eq!(update.discarded, [1]);
        let mut canceled = DialogState::new(dialog_policy::CANCEL, Duration::from_secs(60));
        assert_eq!(
            canceled.request(1, js("alert"), now).0,
            DialogDisposition::Suppress
        );
        assert_eq!(
            canceled.request(1, js("before_unload"), now).0,
            DialogDisposition::Handled
        );
        let mut signal = DialogState::new(dialog_policy::SIGNAL, Duration::from_secs(60));
        assert_eq!(
            signal.request(1, js("alert"), now).0,
            DialogDisposition::Handled
        );
        assert_eq!(
            signal.request(2, js("alert"), now).0,
            DialogDisposition::Suppress
        );
        assert_eq!(signal.pending.len(), 1);
        assert_eq!(
            signal.request(3, js("before_unload"), now).0,
            DialogDisposition::Handled
        );
        assert_eq!(signal.pending.len(), 1);
    }

    #[test]
    fn continuations_reenter_reset_after_atomic_completion_without_locks() {
        let queues = Arc::new(Mutex::new(crate::browser::EventQueues::new()));
        let controller = DialogController::new(
            queues.clone(),
            dialog_policy::SIGNAL,
            Duration::from_secs(60),
        );
        let (callback, calls, reentered) = js_probe(&controller);
        assert_eq!(
            controller.request_js(callback, js_event("prompt")),
            DialogDisposition::Handled
        );
        let request_id = lock(&queues).js_dialog_requests[0].request_id;
        assert!(controller.resolve_js(request_id, true, "你好😀".into()));
        assert!(reentered.load(Ordering::Relaxed));
        assert_eq!(*lock(&calls), [1]);
        assert!(!controller.resolve_js(request_id, false, String::new()));
        assert!(!controller.cancel_js(request_id));
        let events = lock(&queues);
        assert!(events.js_dialog_requests.is_empty());
        assert_eq!(
            events
                .dialog_request_finished
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            [(request_id, "accepted".to_owned())]
        );
    }

    #[test]
    fn expired_dialog_reset_finishes_before_registering_the_next_request() {
        let queues = Arc::new(Mutex::new(crate::browser::EventQueues::new()));
        let controller = DialogController::new(
            queues.clone(),
            dialog_policy::SIGNAL,
            Duration::from_secs(60),
        );
        let (old_callback, old_calls, old_reentered) = js_probe(&controller);
        controller.request_js(old_callback, js_event("confirm"));
        let old_id = lock(&queues).js_dialog_requests[0].request_id;
        // Force deterministic expiry without a sleep or a browser runtime.
        let now = Instant::now();
        if let Some(pending) = lock(&controller.inner.state).pending.get_mut(&old_id) {
            pending.deadline = now;
        }

        let (new_callback, new_calls, new_reentered) = js_probe(&controller);
        assert_eq!(
            controller.request_js(new_callback, js_event("prompt")),
            DialogDisposition::Handled
        );
        assert!(old_reentered.load(Ordering::Relaxed));
        assert_eq!(*lock(&old_calls), [0]);
        assert!(lock(&new_calls).is_empty());
        let new_id = lock(&queues).js_dialog_requests[0].request_id;
        assert_ne!(old_id, new_id);
        assert!(controller.is_pending(new_id));
        assert!(!controller.is_pending(old_id));
        assert_eq!(
            lock(&queues)
                .dialog_request_finished
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            [(old_id, "timed_out".to_owned())]
        );
        assert!(controller.resolve_js(new_id, true, "新请求".into()));
        assert!(new_reentered.load(Ordering::Relaxed));
        assert_eq!(*lock(&new_calls), [1]);
        assert!(!controller.resolve_js(new_id, true, String::new()));
    }

    #[test]
    fn lifecycle_cancellation_and_global_pump_expiry_complete_once() {
        for reason in [
            "reset",
            "browser_closed",
            "navigation",
            "renderer_terminated",
            "policy_changed",
            "unhandled",
            "timed_out",
        ] {
            let queues = Arc::new(Mutex::new(crate::browser::EventQueues::new()));
            let timeout = if reason == "timed_out" {
                Duration::ZERO
            } else {
                Duration::from_secs(60)
            };
            let controller = DialogController::new(queues.clone(), dialog_policy::SIGNAL, timeout);
            let (callback, calls, reentered) = js_probe(&controller);
            controller.request_js(callback, js_event("confirm"));
            let request_id = lock(&queues).js_dialog_requests[0].request_id;
            match reason {
                "reset" => controller.reset_js(reason),
                "browser_closed" => controller.close(reason),
                "policy_changed" => controller.set_policy(dialog_policy::CANCEL),
                "unhandled" => {
                    assert!(controller.dismiss_request(request_id, reason));
                }
                "timed_out" => expire_pending(),
                _ => controller.cancel_all(reason),
            }
            assert!(reentered.load(Ordering::Relaxed), "{reason}");
            assert_eq!(*lock(&calls), [0], "{reason}");
            assert!(!controller.dismiss_request(request_id, "canceled"));
            assert!(!controller.is_pending(request_id));
            assert_eq!(*lock(&calls), [0]);
            let events = lock(&queues);
            assert!(events.js_dialog_requests.is_empty());
            assert_eq!(
                events
                    .dialog_request_finished
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
                [(request_id, reason.to_owned())]
            );
        }
    }

    #[test]
    fn request_ids_do_not_cross_browser_or_recreation_boundaries() {
        let now = Instant::now();
        let mut first = DialogState::new(dialog_policy::SIGNAL, Duration::from_secs(60));
        let (_, old) = first.request(1, js("alert"), now);
        let mut second = DialogState::new(dialog_policy::SIGNAL, Duration::from_secs(60));
        let (_, current) = second.request(2, js("alert"), now);
        assert_ne!(id(&old), id(&current));
        assert!(!second.resolve_js(id(&old), true, String::new(), now).0);
        assert!(!first.resolve_js(id(&current), true, String::new(), now).0);
        first.close("browser_closed");
        let mut recreated = DialogState::new(dialog_policy::SIGNAL, Duration::from_secs(60));
        let (_, new) = recreated.request(3, js("alert"), now);
        assert_ne!(id(&old), id(&new));
        assert!(!recreated.resolve_js(id(&old), true, String::new(), now).0);
    }
}
