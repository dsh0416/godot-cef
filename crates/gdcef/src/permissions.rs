//! Browser-scoped permission decisions with process-wide, non-reusable public ids.
//!
//! Each CEF callback is one atomic request, even when Godot receives one signal
//! per permission. CEF continuations can synchronously dismiss their own prompt;
//! callbacks are therefore removed from state and all locks released first.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use cef::{ImplMediaAccessCallback, ImplPermissionPromptCallback};

use crate::browser::{EventQueuesHandle, PermissionRequestEvent, permission_policy};

static NEXT_REQUEST_ID: AtomicI64 = AtomicI64::new(1);

thread_local! {
    // Creation, permission callbacks and the central pump all use CEF's UI
    // thread. Weak references do not retain closed browser callbacks.
    static CONTROLLERS: RefCell<Vec<Weak<ControllerInner>>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone)]
pub(crate) struct PermissionController {
    inner: Arc<ControllerInner>,
}

struct ControllerInner {
    state: Mutex<PermissionState<CefCallback>>,
    event_queues: EventQueuesHandle,
}

enum CefCallback {
    Media(cef::MediaAccessCallback),
    Prompt(cef::PermissionPromptCallback),
}

impl CefCallback {
    fn finish(self, allowed: Option<bool>, requested: u32, reason: &str) {
        let Some(allowed) = allowed else {
            // CEF has already dismissed/invalidated this request. Releasing our
            // reference is the only operation permitted on its old callback.
            return;
        };
        match self {
            Self::Media(callback) => callback.cont(if allowed { requested } else { 0 }),
            Self::Prompt(callback) => callback.cont(prompt_result(allowed, reason)),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl PermissionController {
    pub(crate) fn new(event_queues: EventQueuesHandle, policy: i32, timeout: Duration) -> Self {
        let inner = Arc::new(ControllerInner {
            state: Mutex::new(PermissionState::new(policy, timeout)),
            event_queues,
        });
        CONTROLLERS.with_borrow_mut(|controllers| {
            controllers.retain(|controller| controller.strong_count() > 0);
            controllers.push(Arc::downgrade(&inner));
        });
        Self { inner }
    }

    pub(crate) fn request_media(
        &self,
        callback: cef::MediaAccessCallback,
        requested: u32,
        origin: String,
        frame_id: String,
    ) {
        self.request(
            CefCallback::Media(callback),
            requested,
            origin,
            Scope::Media { frame_id },
            media_permissions(requested),
        );
    }

    pub(crate) fn request_prompt(
        &self,
        callback: cef::PermissionPromptCallback,
        prompt_id: u64,
        requested: u32,
        origin: String,
    ) {
        self.request(
            CefCallback::Prompt(callback),
            requested,
            origin,
            Scope::Prompt { prompt_id },
            prompt_permissions(requested),
        );
    }

    fn request(
        &self,
        callback: CefCallback,
        requested: u32,
        origin: String,
        scope: Scope,
        permissions: Vec<Permission>,
    ) {
        self.update(|state| {
            (
                (),
                state.request(
                    callback,
                    requested,
                    origin,
                    scope,
                    permissions,
                    Instant::now(),
                ),
            )
        });
    }

    pub(crate) fn resolve(&self, id: i64, grant: bool) -> bool {
        self.update(|state| state.resolve(id, grant, Instant::now()))
    }

    pub(crate) fn is_pending(&self, id: i64) -> bool {
        self.update(|state| {
            let update = state.expire(Instant::now());
            (state.requests.contains_key(&id), update)
        })
    }

    pub(crate) fn dismiss(&self, prompt_id: u64, result: cef::PermissionRequestResult) {
        let reason = match result {
            cef::PermissionRequestResult::ACCEPT => "allowed",
            cef::PermissionRequestResult::DENY => "denied",
            _ => "dismissed",
        };
        self.update(|state| {
            (
                (),
                state.cancel_where(
                    |scope| matches!(scope, Scope::Prompt { prompt_id: id } if *id == prompt_id),
                    reason,
                    None,
                ),
            )
        });
    }

    pub(crate) fn cancel_all(&self, reason: &str) {
        self.update(|state| ((), state.cancel_where(|_| true, reason, Some(false))));
    }

    pub(crate) fn cancel_frame(&self, frame_id: &str) {
        self.update(|state| ((), state.cancel_frame(frame_id)));
    }

    /// Stop accepting requests before callbacks can reenter during shutdown.
    pub(crate) fn close(&self, reason: &str) {
        self.update(|state| ((), state.close(reason)));
    }

    /// CEF already invalidated these callbacks. A surviving browser may reload
    /// after renderer termination, so this does not permanently close it.
    pub(crate) fn invalidate(&self, reason: &str) {
        self.update(|state| ((), state.cancel_where(|_| true, reason, None)));
    }

    pub(crate) fn set_policy(&self, policy: i32) {
        self.update(|state| ((), state.set_policy(policy)));
    }

    fn expire(&self) {
        self.update(|state| ((), state.expire(Instant::now())));
    }

    fn update<T>(
        &self,
        change: impl FnOnce(&mut PermissionState<CefCallback>) -> (T, Update<CefCallback>),
    ) -> T {
        let (result, completions) = {
            let mut state = lock(&self.inner.state);
            let (result, update) = change(&mut state);
            if !update.events.is_empty()
                || !update.removed.is_empty()
                || !update.completions.is_empty()
            {
                let mut queues = lock(&self.inner.event_queues);
                let removed: HashSet<_> = update.removed.into_iter().collect();
                queues
                    .permission_requests
                    .retain(|event| !removed.contains(&event.request_id));
                queues.permission_requests.extend(update.events);
                for completion in &update.completions {
                    queues.permission_request_finished.extend(
                        completion
                            .ids
                            .iter()
                            .map(|id| (*id, completion.reason.to_owned())),
                    );
                }
            }
            (result, update.completions)
        };
        // In particular, Continue(ACCEPT/DENY) can synchronously call
        // OnDismissPermissionPrompt. Neither the state nor queue lock is held.
        for completion in completions {
            completion.callback.finish(
                completion.allowed,
                completion.requested,
                &completion.reason,
            );
        }
        result
    }
}

/// Called from the process-wide CEF pump, independently of texture processing,
/// node visibility, SceneTree pause, and RenderingServer frame callbacks.
pub(crate) fn expire_pending() {
    let controllers = CONTROLLERS.with_borrow_mut(|registered| {
        let active: Vec<_> = registered.iter().filter_map(Weak::upgrade).collect();
        registered.retain(|controller| controller.strong_count() > 0);
        active
    });
    // Release the registry's RefCell borrow before callbacks can create/close browsers.
    for inner in controllers {
        PermissionController { inner }.expire();
    }
}

fn normalize_policy(policy: i32) -> i32 {
    match policy {
        permission_policy::ALLOW_ALL | permission_policy::SIGNAL => policy,
        _ => permission_policy::DENY_ALL,
    }
}

fn prompt_result(allowed: bool, reason: &str) -> cef::PermissionRequestResult {
    if allowed {
        cef::PermissionRequestResult::ACCEPT
    } else if matches!(
        reason,
        "dismissed" | "timed_out" | "navigation" | "policy_changed" | "browser_closed"
    ) {
        // Lifecycle cancellation must not become a persistent site-level DENY.
        cef::PermissionRequestResult::DISMISS
    } else {
        cef::PermissionRequestResult::DENY
    }
}

#[derive(Clone, Debug)]
struct Permission {
    name: &'static str,
    grantable: bool,
}

#[derive(Debug)]
enum Scope {
    Media { frame_id: String },
    Prompt { prompt_id: u64 },
}

struct Group<C> {
    callback: C,
    requested: u32,
    scope: Scope,
    deadline: Instant,
    ids: Vec<i64>,
    remaining: usize,
}

struct PendingDecision {
    group: i64,
    grantable: bool,
}

struct PermissionState<C> {
    policy: i32,
    timeout: Duration,
    closed: bool,
    groups: HashMap<i64, Group<C>>,
    requests: HashMap<i64, PendingDecision>,
}

struct Completion<C> {
    callback: C,
    requested: u32,
    allowed: Option<bool>,
    ids: Vec<i64>,
    reason: String,
}

struct Update<C> {
    events: Vec<PermissionRequestEvent>,
    removed: Vec<i64>,
    completions: Vec<Completion<C>>,
}

impl<C> Default for Update<C> {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            removed: Vec::new(),
            completions: Vec::new(),
        }
    }
}

impl<C> PermissionState<C> {
    fn new(policy: i32, timeout: Duration) -> Self {
        Self {
            policy: normalize_policy(policy),
            timeout,
            closed: false,
            groups: HashMap::new(),
            requests: HashMap::new(),
        }
    }

    fn request(
        &mut self,
        callback: C,
        requested: u32,
        origin: String,
        scope: Scope,
        permissions: Vec<Permission>,
        now: Instant,
    ) -> Update<C> {
        let mut update = self.expire(now);
        let grantable = requested != 0 && permissions.iter().all(|permission| permission.grantable);
        let deadline = now.checked_add(self.timeout);
        if self.closed || self.policy != permission_policy::SIGNAL || deadline.is_none() {
            let allowed = !self.closed && self.policy == permission_policy::ALLOW_ALL && grantable;
            update.completions.push(Completion {
                callback,
                requested,
                allowed: Some(allowed),
                ids: Vec::new(),
                reason: if self.closed {
                    "browser_closed"
                } else if allowed {
                    "allowed"
                } else {
                    "denied"
                }
                .to_owned(),
            });
            return update;
        }
        let Some(ids) = allocate_request_ids(permissions.len()) else {
            update.completions.push(Completion {
                callback,
                requested,
                allowed: Some(false),
                ids: Vec::new(),
                reason: "denied".to_owned(),
            });
            return update;
        };
        let Some((&group_id, deadline)) = ids.first().zip(deadline) else {
            update.completions.push(Completion {
                callback,
                requested,
                allowed: Some(false),
                ids: Vec::new(),
                reason: "denied".to_owned(),
            });
            return update;
        };
        for (id, permission) in ids.iter().copied().zip(permissions) {
            self.requests.insert(
                id,
                PendingDecision {
                    group: group_id,
                    grantable: permission.grantable,
                },
            );
            update.events.push(PermissionRequestEvent {
                permission_type: permission.name.to_owned(),
                url: origin.clone(),
                request_id: id,
            });
        }
        self.groups.insert(
            group_id,
            Group {
                callback,
                requested,
                scope,
                deadline,
                remaining: ids.len(),
                ids,
            },
        );
        update
    }

    fn resolve(&mut self, id: i64, grant: bool, now: Instant) -> (bool, Update<C>) {
        let mut update = self.expire(now);
        let Some(decision) = self.requests.remove(&id) else {
            return (false, update);
        };
        update.removed.push(id);
        if !grant || !decision.grantable {
            self.finish(decision.group, Some(false), "denied", &mut update);
        } else if let Some(group) = self.groups.get_mut(&decision.group) {
            group.remaining -= 1;
            if group.remaining == 0 {
                self.finish(decision.group, Some(true), "allowed", &mut update);
            }
        }
        (true, update)
    }

    fn expire(&mut self, now: Instant) -> Update<C> {
        let ids: Vec<_> = self
            .groups
            .iter()
            .filter_map(|(&id, group)| (group.deadline <= now).then_some(id))
            .collect();
        let mut update = Update::default();
        for id in ids {
            self.finish(id, Some(false), "timed_out", &mut update);
        }
        update
    }

    fn cancel_where(
        &mut self,
        matches: impl Fn(&Scope) -> bool,
        reason: &str,
        allowed: Option<bool>,
    ) -> Update<C> {
        let ids: Vec<_> = self
            .groups
            .iter()
            .filter_map(|(&id, group)| matches(&group.scope).then_some(id))
            .collect();
        let mut update = Update::default();
        for id in ids {
            self.finish(id, allowed, reason, &mut update);
        }
        update
    }

    fn cancel_frame(&mut self, frame_id: &str) -> Update<C> {
        self.cancel_where(
            |scope| match scope {
                Scope::Media { frame_id: id } => id == frame_id,
                // Generic CEF prompts do not identify their requesting frame.
                Scope::Prompt { .. } => true,
            },
            "navigation",
            Some(false),
        )
    }

    fn close(&mut self, reason: &str) -> Update<C> {
        self.closed = true;
        self.cancel_where(|_| true, reason, Some(false))
    }

    fn set_policy(&mut self, policy: i32) -> Update<C> {
        let policy = normalize_policy(policy);
        if self.policy == policy {
            return Update::default();
        }
        self.policy = policy;
        self.cancel_where(|_| true, "policy_changed", Some(false))
    }

    fn finish(&mut self, id: i64, allowed: Option<bool>, reason: &str, update: &mut Update<C>) {
        let Some(group) = self.groups.remove(&id) else {
            return;
        };
        for id in &group.ids {
            self.requests.remove(id);
        }
        update.removed.extend(group.ids.iter().copied());
        update.completions.push(Completion {
            callback: group.callback,
            requested: group.requested,
            allowed,
            ids: group.ids,
            reason: reason.to_owned(),
        });
    }
}

fn allocate_request_ids(count: usize) -> Option<Vec<i64>> {
    let count = i64::try_from(count).ok().filter(|count| *count > 0)?;
    let first = NEXT_REQUEST_ID
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(count)
        })
        .ok()?;
    Some((first..first + count).collect())
}

fn map_permissions(
    requested: u32,
    known: &[(u32, &'static str)],
    unknown: &'static str,
) -> Vec<Permission> {
    let mut permissions = Vec::new();
    let mut known_mask = 0;
    for &(bit, name) in known {
        known_mask |= bit;
        if requested & bit != 0 {
            permissions.push(Permission {
                name,
                grantable: true,
            });
        }
    }
    if requested == 0 || requested & !known_mask != 0 {
        permissions.push(Permission {
            name: unknown,
            grantable: false,
        });
    }
    permissions
}

fn media_permissions(requested: u32) -> Vec<Permission> {
    use cef::MediaAccessPermissionTypes as P;
    map_permissions(
        requested,
        &[
            (
                crate::cef_raw_to_u32!(P::DEVICE_AUDIO_CAPTURE.get_raw()),
                "microphone",
            ),
            (
                crate::cef_raw_to_u32!(P::DEVICE_VIDEO_CAPTURE.get_raw()),
                "camera",
            ),
            (
                crate::cef_raw_to_u32!(P::DESKTOP_AUDIO_CAPTURE.get_raw()),
                "desktop_audio_capture",
            ),
            (
                crate::cef_raw_to_u32!(P::DESKTOP_VIDEO_CAPTURE.get_raw()),
                "desktop_video_capture",
            ),
        ],
        "unknown_media_permission",
    )
}

fn prompt_permissions(requested: u32) -> Vec<Permission> {
    use cef::PermissionRequestTypes as P;
    map_permissions(
        requested,
        &[
            (
                crate::cef_raw_to_u32!(P::AR_SESSION.get_raw()),
                "ar_session",
            ),
            (
                crate::cef_raw_to_u32!(P::CAMERA_PAN_TILT_ZOOM.get_raw()),
                "camera_pan_tilt_zoom",
            ),
            (crate::cef_raw_to_u32!(P::CAMERA_STREAM.get_raw()), "camera"),
            (
                crate::cef_raw_to_u32!(P::CAPTURED_SURFACE_CONTROL.get_raw()),
                "captured_surface_control",
            ),
            (crate::cef_raw_to_u32!(P::CLIPBOARD.get_raw()), "clipboard"),
            (
                crate::cef_raw_to_u32!(P::TOP_LEVEL_STORAGE_ACCESS.get_raw()),
                "top_level_storage_access",
            ),
            (
                crate::cef_raw_to_u32!(P::DISK_QUOTA.get_raw()),
                "disk_quota",
            ),
            (
                crate::cef_raw_to_u32!(P::LOCAL_FONTS.get_raw()),
                "local_fonts",
            ),
            (
                crate::cef_raw_to_u32!(P::GEOLOCATION.get_raw()),
                "geolocation",
            ),
            (
                crate::cef_raw_to_u32!(P::HAND_TRACKING.get_raw()),
                "hand_tracking",
            ),
            (
                crate::cef_raw_to_u32!(P::IDENTITY_PROVIDER.get_raw()),
                "identity_provider",
            ),
            (
                crate::cef_raw_to_u32!(P::IDLE_DETECTION.get_raw()),
                "idle_detection",
            ),
            (
                crate::cef_raw_to_u32!(P::MIC_STREAM.get_raw()),
                "microphone",
            ),
            (
                crate::cef_raw_to_u32!(P::MIDI_SYSEX.get_raw()),
                "midi_sysex",
            ),
            (
                crate::cef_raw_to_u32!(P::MULTIPLE_DOWNLOADS.get_raw()),
                "multiple_downloads",
            ),
            (
                crate::cef_raw_to_u32!(P::NOTIFICATIONS.get_raw()),
                "notifications",
            ),
            (
                crate::cef_raw_to_u32!(P::KEYBOARD_LOCK.get_raw()),
                "keyboard_lock",
            ),
            (
                crate::cef_raw_to_u32!(P::POINTER_LOCK.get_raw()),
                "pointer_lock",
            ),
            (
                crate::cef_raw_to_u32!(P::PROTECTED_MEDIA_IDENTIFIER.get_raw()),
                "protected_media_identifier",
            ),
            (
                crate::cef_raw_to_u32!(P::REGISTER_PROTOCOL_HANDLER.get_raw()),
                "register_protocol_handler",
            ),
            (
                crate::cef_raw_to_u32!(P::STORAGE_ACCESS.get_raw()),
                "storage_access",
            ),
            (
                crate::cef_raw_to_u32!(P::VR_SESSION.get_raw()),
                "vr_session",
            ),
            (
                crate::cef_raw_to_u32!(P::WEB_APP_INSTALLATION.get_raw()),
                "web_app_installation",
            ),
            (
                crate::cef_raw_to_u32!(P::WINDOW_MANAGEMENT.get_raw()),
                "window_management",
            ),
            (
                crate::cef_raw_to_u32!(P::FILE_SYSTEM_ACCESS.get_raw()),
                "file_system_access",
            ),
            (
                crate::cef_raw_to_u32!(P::LOCAL_NETWORK_ACCESS_DEPRECATED.get_raw()),
                "local_network_access",
            ),
            (
                crate::cef_raw_to_u32!(P::LOCAL_NETWORK.get_raw()),
                "local_network",
            ),
            (
                crate::cef_raw_to_u32!(P::LOOPBACK_NETWORK.get_raw()),
                "loopback_network",
            ),
            (crate::cef_raw_to_u32!(P::SENSORS.get_raw()), "sensors"),
        ],
        "unknown_permission",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cef::rc::{ConvertReturnValue, RcImpl};
    use std::sync::atomic::AtomicBool;

    type ProbeResults = Arc<Mutex<Vec<cef::PermissionRequestResult>>>;

    struct PromptProbe {
        controller: Weak<ControllerInner>,
        prompt_id: u64,
        results: ProbeResults,
        reentered_without_locks: Arc<AtomicBool>,
    }

    // CEF allocates this callback type itself, so cef-rs intentionally has no
    // wrap_* macro for it. A Rust-owned CEF refcount/vtable tests the actual
    // controller continuation without initializing either CEF or Godot.
    fn prompt_probe(
        controller: &PermissionController,
        prompt_id: u64,
    ) -> (cef::PermissionPromptCallback, ProbeResults, Arc<AtomicBool>) {
        unsafe extern "C" fn continue_prompt(
            this: *mut cef::sys::cef_permission_prompt_callback_t,
            result: cef::sys::cef_permission_request_result_t,
        ) {
            // RcImpl is repr(C), with this callback as its first member. The
            // owning cef-rs reference keeps both the vtable and probe alive.
            let probe = unsafe {
                &(*this.cast::<RcImpl<cef::sys::cef_permission_prompt_callback_t, PromptProbe>>())
                    .interface
            };
            let result: cef::PermissionRequestResult = result.into();
            lock(&probe.results).push(result);
            let Some(inner) = probe.controller.upgrade() else {
                return;
            };
            // Never turn a failed lock-order regression into a hanging test.
            let state_unlocked = inner.state.try_lock().is_ok();
            let queues_unlocked = inner.event_queues.try_lock().is_ok();
            if state_unlocked && queues_unlocked {
                PermissionController { inner }.dismiss(probe.prompt_id, result);
                probe.reentered_without_locks.store(true, Ordering::Relaxed);
            }
        }

        let results = Arc::new(Mutex::new(Vec::new()));
        let reentered_without_locks = Arc::new(AtomicBool::new(false));
        let raw = cef::sys::cef_permission_prompt_callback_t {
            // All base fields are integers or optional function pointers;
            // RcImpl::new fills in its own valid reference-counting methods.
            base: unsafe { std::mem::zeroed() },
            cont: Some(continue_prompt),
        };
        let pointer = RcImpl::new(
            raw,
            PromptProbe {
                controller: Arc::downgrade(&controller.inner),
                prompt_id,
                results: results.clone(),
                reentered_without_locks: reentered_without_locks.clone(),
            },
        );
        let callback = pointer
            .cast::<cef::sys::cef_permission_prompt_callback_t>()
            .wrap_result();
        (callback, results, reentered_without_locks)
    }

    fn pending_media(
        state: &mut PermissionState<u32>,
        now: Instant,
        frame: &str,
        mask: u32,
    ) -> Vec<i64> {
        state
            .request(
                42,
                mask,
                "https://example.test".to_owned(),
                Scope::Media {
                    frame_id: frame.to_owned(),
                },
                media_permissions(mask),
                now,
            )
            .events
            .into_iter()
            .map(|event| event.request_id)
            .collect()
    }

    #[test]
    fn camera_and_microphone_grant_only_completes_after_both_decisions() {
        let now = Instant::now();
        let mut state = PermissionState::new(permission_policy::SIGNAL, Duration::from_secs(30));
        let ids = pending_media(&mut state, now, "main", 3);
        assert_eq!(ids.len(), 2);
        let (accepted, first) = state.resolve(ids[1], true, now);
        assert!(accepted);
        assert!(first.completions.is_empty());
        assert!(!state.resolve(ids[1], true, now).0);
        let (accepted, last) = state.resolve(ids[0], true, now);
        assert!(accepted);
        assert_eq!(last.completions.len(), 1);
        assert_eq!(last.completions[0].allowed, Some(true));
        assert_eq!(last.completions[0].requested, 3);
        assert_eq!(last.completions[0].ids, ids);
        assert_eq!(last.completions[0].reason, "allowed");
        assert!(state.groups.is_empty());
        assert!(state.requests.is_empty());
    }

    #[test]
    fn continuing_a_prompt_can_synchronously_dismiss_it_without_locks_or_duplicate_results() {
        let queues = Arc::new(Mutex::new(crate::browser::EventQueues::new()));
        let controller = PermissionController::new(
            queues.clone(),
            permission_policy::SIGNAL,
            Duration::from_secs(30),
        );
        let (callback, results, reentered) = prompt_probe(&controller, 77);
        let camera = crate::cef_raw_to_u32!(cef::PermissionRequestTypes::CAMERA_STREAM.get_raw());
        controller.request_prompt(callback, 77, camera, "https://example.test".to_owned());
        let id = lock(&queues).permission_requests[0].request_id;
        assert!(controller.resolve(id, true));
        assert!(reentered.load(Ordering::Relaxed));
        assert_eq!(*lock(&results), [cef::PermissionRequestResult::ACCEPT]);
        assert!(!controller.resolve(id, false));
        let events = lock(&queues);
        assert!(events.permission_requests.is_empty());
        assert_eq!(events.permission_request_finished.len(), 1);
        assert_eq!(
            events.permission_request_finished[0],
            (id, "allowed".to_owned())
        );
    }

    #[test]
    fn central_expiration_completes_requests_without_surface_processing() {
        let queues = Arc::new(Mutex::new(crate::browser::EventQueues::new()));
        let controller =
            PermissionController::new(queues.clone(), permission_policy::SIGNAL, Duration::ZERO);
        let (callback, results, reentered) = prompt_probe(&controller, 88);
        let local_network =
            crate::cef_raw_to_u32!(cef::PermissionRequestTypes::LOCAL_NETWORK.get_raw());
        controller.request_prompt(
            callback,
            88,
            local_network,
            "https://example.test".to_owned(),
        );
        let id = lock(&queues).permission_requests[0].request_id;
        expire_pending();
        assert!(reentered.load(Ordering::Relaxed));
        assert_eq!(*lock(&results), [cef::PermissionRequestResult::DISMISS]);
        assert!(!controller.is_pending(id));
        let events = lock(&queues);
        assert!(events.permission_requests.is_empty());
        assert_eq!(events.permission_request_finished.len(), 1);
        assert_eq!(
            events.permission_request_finished[0],
            (id, "timed_out".to_owned())
        );
    }

    #[test]
    fn denying_one_permission_immediately_denies_the_whole_group() {
        let now = Instant::now();
        let mut state = PermissionState::new(permission_policy::SIGNAL, Duration::from_secs(30));
        let ids = pending_media(&mut state, now, "main", 3);
        let (_, denied) = state.resolve(ids[0], false, now);
        assert_eq!(denied.completions.len(), 1);
        assert_eq!(denied.completions[0].allowed, Some(false));
        assert_eq!(denied.completions[0].ids, ids);
        assert!(!state.resolve(ids[1], true, now).0);
    }

    #[test]
    fn partially_approved_groups_time_out_as_a_single_denial() {
        let now = Instant::now();
        let mut state = PermissionState::new(permission_policy::SIGNAL, Duration::from_secs(1));
        let ids = pending_media(&mut state, now, "main", 3);
        assert!(state.resolve(ids[0], true, now).1.completions.is_empty());
        let (accepted, expired) = state.resolve(ids[1], true, now + Duration::from_secs(1));
        assert!(!accepted);
        assert_eq!(expired.completions[0].allowed, Some(false));
        assert_eq!(expired.completions[0].reason, "timed_out");
        assert_eq!(expired.completions[0].ids, ids);
        assert!(
            state
                .expire(now + Duration::from_secs(2))
                .completions
                .is_empty()
        );
    }

    #[test]
    fn ids_cannot_authorize_another_browser_or_recreated_request() {
        let now = Instant::now();
        let mut first = PermissionState::new(permission_policy::SIGNAL, Duration::from_secs(30));
        let old = pending_media(&mut first, now, "main", 2)[0];
        let mut second = PermissionState::new(permission_policy::SIGNAL, Duration::from_secs(30));
        let new = pending_media(&mut second, now, "main", 2)[0];
        assert_ne!(old, new);
        assert!(!second.resolve(old, true, now).0);
        assert!(!first.resolve(new, true, now).0);
    }

    #[test]
    fn unknown_bits_cannot_be_approved_even_with_allow_all() {
        let now = Instant::now();
        let mut signal = PermissionState::new(permission_policy::SIGNAL, Duration::from_secs(30));
        let ids = pending_media(&mut signal, now, "main", 2 | (1 << 31));
        assert!(signal.resolve(ids[0], true, now).1.completions.is_empty());
        let (_, unknown) = signal.resolve(ids[1], true, now);
        assert_eq!(unknown.completions[0].allowed, Some(false));
        for mask in [0, 1 << 31, 2 | (1 << 31)] {
            let mut allow =
                PermissionState::new(permission_policy::ALLOW_ALL, Duration::from_secs(30));
            let update = allow.request(
                42,
                mask,
                String::new(),
                Scope::Media {
                    frame_id: String::new(),
                },
                media_permissions(mask),
                now,
            );
            assert!(update.events.is_empty());
            assert_eq!(update.completions[0].allowed, Some(false));
        }
    }

    #[test]
    fn frame_navigation_cancels_only_its_media_and_unscoped_prompts() {
        let now = Instant::now();
        let mut state = PermissionState::new(permission_policy::SIGNAL, Duration::from_secs(30));
        let main = pending_media(&mut state, now, "main", 2)[0];
        let child = pending_media(&mut state, now, "child", 2)[0];
        let prompt = state
            .request(
                7,
                1,
                String::new(),
                Scope::Prompt { prompt_id: 77 },
                prompt_permissions(1),
                now,
            )
            .events[0]
            .request_id;
        let update = state.cancel_frame("child");
        assert_eq!(update.completions.len(), 2);
        assert!(state.requests.contains_key(&main));
        assert!(!state.requests.contains_key(&child));
        assert!(!state.requests.contains_key(&prompt));
    }

    #[test]
    fn cef_dismissal_drops_the_callback_without_continuing_it_again() {
        let now = Instant::now();
        let mut state = PermissionState::new(permission_policy::SIGNAL, Duration::from_secs(30));
        let id = state
            .request(
                7,
                1,
                String::new(),
                Scope::Prompt { prompt_id: 77 },
                prompt_permissions(1),
                now,
            )
            .events[0]
            .request_id;
        let dismissed = state.cancel_where(
            |scope| matches!(scope, Scope::Prompt { prompt_id: 77 }),
            "dismissed",
            None,
        );
        assert_eq!(dismissed.completions.len(), 1);
        assert_eq!(dismissed.completions[0].allowed, None);
        assert_eq!(dismissed.completions[0].ids, [id]);
        assert!(!state.resolve(id, true, now).0);
        assert!(
            state
                .cancel_where(|_| true, "browser_closed", None)
                .completions
                .is_empty()
        );
    }

    #[test]
    fn closed_state_denies_new_requests_before_callbacks_can_reenter() {
        let now = Instant::now();
        let mut state = PermissionState::new(permission_policy::ALLOW_ALL, Duration::from_secs(30));
        let _ = state.close("browser_closed");
        let update = state.request(
            42,
            2,
            String::new(),
            Scope::Media {
                frame_id: String::new(),
            },
            media_permissions(2),
            now,
        );
        assert!(update.events.is_empty());
        assert_eq!(update.completions[0].allowed, Some(false));
        assert_eq!(update.completions[0].reason, "browser_closed");
    }

    #[test]
    fn changing_policy_cancels_existing_prompts_without_granting_them() {
        let now = Instant::now();
        let mut state = PermissionState::new(permission_policy::SIGNAL, Duration::from_secs(30));
        let ids = pending_media(&mut state, now, "main", 3);
        assert!(
            state
                .set_policy(permission_policy::SIGNAL)
                .completions
                .is_empty()
        );
        assert!(state.resolve(ids[0], true, now).0);
        let update = state.set_policy(permission_policy::ALLOW_ALL);
        assert_eq!(update.completions.len(), 1);
        assert_eq!(update.completions[0].ids, ids);
        assert_eq!(update.completions[0].allowed, Some(false));
        assert_eq!(update.completions[0].reason, "policy_changed");
        assert!(!state.resolve(ids[1], true, now).0);
        let subsequent = state.request(
            42,
            3,
            String::new(),
            Scope::Media {
                frame_id: "main".to_owned(),
            },
            media_permissions(3),
            now,
        );
        assert_eq!(subsequent.completions[0].allowed, Some(true));
        assert_eq!(subsequent.completions[0].requested, 3);
    }

    #[test]
    fn renderer_invalidation_drops_old_callbacks_and_allows_new_requests_after_reload() {
        let now = Instant::now();
        let mut state = PermissionState::new(permission_policy::SIGNAL, Duration::from_secs(30));
        let old = pending_media(&mut state, now, "main", 2)[0];
        let invalidated = state.cancel_where(|_| true, "renderer_terminated", None);
        assert_eq!(invalidated.completions[0].allowed, None);
        assert_eq!(invalidated.completions[0].ids, [old]);
        assert!(!state.resolve(old, true, now).0);
        let new = pending_media(&mut state, now, "main", 2)[0];
        assert_ne!(old, new);
        assert_eq!(
            state.resolve(new, true, now).1.completions[0].allowed,
            Some(true)
        );
    }

    #[test]
    fn lifecycle_cancellations_dismiss_without_persisting_a_site_denial() {
        for reason in [
            "dismissed",
            "timed_out",
            "navigation",
            "policy_changed",
            "browser_closed",
        ] {
            assert_eq!(
                prompt_result(false, reason),
                cef::PermissionRequestResult::DISMISS
            );
        }
        assert_eq!(
            prompt_result(false, "denied"),
            cef::PermissionRequestResult::DENY
        );
        assert_eq!(
            prompt_result(true, "allowed"),
            cef::PermissionRequestResult::ACCEPT
        );
    }

    #[test]
    fn every_cef_154_prompt_bit_has_a_distinct_known_label() {
        let all = prompt_permissions((1 << 29) - 1);
        assert_eq!(all.len(), 29);
        assert!(all.iter().all(|permission| permission.grantable));
        let names: HashSet<_> = all.iter().map(|permission| permission.name).collect();
        assert_eq!(names.len(), 29);
        assert!(names.contains("local_network_access"));
        assert!(names.contains("local_network"));
        assert!(names.contains("loopback_network"));
        assert!(names.contains("camera"));
        assert!(names.contains("microphone"));
    }
}
