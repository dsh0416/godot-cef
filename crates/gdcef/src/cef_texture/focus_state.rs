//! Pure focus transitions: the browser control and its IME proxy share one owner.

use crate::browser::ImeEnableEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FocusOwner {
    Browser,
    Proxy,
    Outside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FocusUpdate {
    pub cef_focus: Option<bool>,
    pub ime_active: bool,
    pub transfer: Option<FocusOwner>,
}

#[derive(Default)]
pub(super) struct FocusState {
    editable: bool,
    cef_focused: Option<bool>,
    editable_frame: Option<String>,
    key_event_commit: bool,
}

impl FocusState {
    pub fn update_editable(&mut self, event: &ImeEnableEvent) {
        // Different renderer processes have independent IPC channels. Losing
        // focus in the old frame can arrive after the new frame enabled IME.
        // A focused noneditable node, however, really does end text input.
        if !event.enabled
            && !event.focused_node
            && self.editable_frame.as_deref() != Some(event.frame_id.as_str())
        {
            return;
        }
        self.editable = event.enabled;
        self.key_event_commit = event.enabled && event.requires_key_event_commit;
        self.editable_frame = event.enabled.then(|| event.frame_id.clone());
    }

    pub fn requires_key_event_commit(&self) -> bool {
        self.key_event_commit
    }

    pub fn clear_editable(&mut self) {
        self.editable = false;
        self.editable_frame = None;
        self.key_event_commit = false;
    }

    pub fn invalidate_host_focus(&mut self) {
        self.cef_focused = None;
    }

    pub fn reconcile(&mut self, owner: FocusOwner, available: bool) -> FocusUpdate {
        let focused = available && owner != FocusOwner::Outside;
        let ime_active = focused && self.editable;
        let cef_focus = (Some(focused) != self.cef_focused).then_some(focused);
        self.cef_focused = Some(focused);

        let transfer = match (owner, available, ime_active) {
            (FocusOwner::Browser, true, true) => Some(FocusOwner::Proxy),
            (FocusOwner::Proxy, true, false) => Some(FocusOwner::Browser),
            _ => None,
        };
        FocusUpdate {
            cef_focus,
            ime_active,
            transfer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ime_event(frame: &str, enabled: bool, focused_node: bool) -> ImeEnableEvent {
        ImeEnableEvent {
            enabled,
            focused_node,
            requires_key_event_commit: frame != "main",
            frame_id: frame.into(),
        }
    }

    #[test]
    fn late_blur_from_a_different_renderer_does_not_disable_ime() {
        for (old, new) in [("main", "child"), ("child", "main"), ("child", "other")] {
            let mut focus = FocusState::default();
            focus.update_editable(&ime_event(old, true, true));
            focus.update_editable(&ime_event(new, true, true));
            focus.update_editable(&ime_event(old, false, false));
            assert!(focus.reconcile(FocusOwner::Proxy, true).ime_active);
            assert_eq!(focus.editable_frame.as_deref(), Some(new));
            assert_eq!(focus.requires_key_event_commit(), new != "main");
        }
    }

    #[test]
    fn active_frame_blur_and_focus_on_noneditable_content_disable_ime() {
        for event in [
            ime_event("child", false, false),
            ime_event("child", false, true),
            ime_event("main", false, true),
            ime_event("other", false, true),
        ] {
            let mut focus = FocusState::default();
            focus.update_editable(&ime_event("child", true, true));
            focus.update_editable(&event);
            let update = focus.reconcile(FocusOwner::Proxy, true);
            assert!(!update.ime_active);
            assert_eq!(update.transfer, Some(FocusOwner::Browser));
            assert!(!focus.requires_key_event_commit());
            assert!(focus.editable_frame.is_none());
            focus.update_editable(&ime_event("child", true, true));
            assert!(focus.reconcile(FocusOwner::Browser, true).ime_active);
        }
    }

    #[test]
    fn navigation_clears_subframe_text_routing_without_changing_focus_owner() {
        let mut focus = FocusState::default();
        focus.update_editable(&ime_event("child", true, true));
        focus.reconcile(FocusOwner::Proxy, true);
        focus.clear_editable();
        assert!(!focus.requires_key_event_commit());
        assert!(focus.editable_frame.is_none());
        let update = focus.reconcile(FocusOwner::Proxy, true);
        assert!(!update.ime_active);
        assert_eq!(update.cef_focus, None);
        assert_eq!(update.transfer, Some(FocusOwner::Browser));
        focus.update_editable(&ime_event("main", true, true));
        assert!(focus.reconcile(FocusOwner::Browser, true).ime_active);
        assert!(!focus.requires_key_event_commit());
    }

    #[test]
    fn same_site_frames_keep_native_ime_unless_nested_below_a_remote_parent() {
        let mut focus = FocusState::default();
        let mut event = ime_event("same-site-child", true, true);
        event.requires_key_event_commit = false;
        focus.update_editable(&event);
        assert!(focus.reconcile(FocusOwner::Browser, true).ime_active);
        assert!(!focus.requires_key_event_commit());
        // The renderer reports its local ancestry, not URL equality. A
        // same-site descendant of an OOPIF can still use a separate widget.
        event.frame_id = "same-site-child-below-oopif".into();
        event.requires_key_event_commit = true;
        focus.update_editable(&event);
        assert!(focus.requires_key_event_commit());
    }

    #[test]
    fn new_and_recreated_hosts_receive_an_explicit_focus_state() {
        let mut focus = FocusState::default();
        assert_eq!(
            focus.reconcile(FocusOwner::Outside, true).cef_focus,
            Some(false)
        );
        assert_eq!(
            focus.reconcile(FocusOwner::Browser, true).cef_focus,
            Some(true)
        );
        focus.invalidate_host_focus();
        assert_eq!(
            focus.reconcile(FocusOwner::Browser, true).cef_focus,
            Some(true)
        );
        focus.invalidate_host_focus();
        assert_eq!(
            focus.reconcile(FocusOwner::Outside, true).cef_focus,
            Some(false)
        );
    }

    #[test]
    fn repeated_clicks_keep_cef_focused_across_proxy_transfers() {
        let mut focus = FocusState::default();
        assert_eq!(
            focus.reconcile(FocusOwner::Browser, true).cef_focus,
            Some(true)
        );
        focus.editable = true;
        for _ in 0..100 {
            let click = focus.reconcile(FocusOwner::Browser, true);
            assert_eq!(click.cef_focus, None);
            assert!(click.ime_active);
            assert_eq!(click.transfer, Some(FocusOwner::Proxy));
            let proxy = focus.reconcile(FocusOwner::Proxy, true);
            assert_eq!(proxy.cef_focus, None);
            assert_eq!(proxy.transfer, None);
        }
    }

    #[test]
    fn external_focus_loss_never_requests_focus_back() {
        let mut focus = FocusState {
            editable: true,
            ..Default::default()
        };
        focus.reconcile(FocusOwner::Proxy, true);
        let update = focus.reconcile(FocusOwner::Outside, true);
        assert_eq!(update.cef_focus, Some(false));
        assert!(!update.ime_active);
        assert_eq!(update.transfer, None);
        // A late renderer editability notification cannot steal native focus.
        focus.editable = true;
        assert_eq!(focus.reconcile(FocusOwner::Outside, true).transfer, None);
    }

    #[test]
    fn editable_exit_returns_focus_only_when_proxy_still_owns_it() {
        let mut focus = FocusState {
            editable: true,
            ..Default::default()
        };
        focus.reconcile(FocusOwner::Proxy, true);
        focus.editable = false;
        let update = focus.reconcile(FocusOwner::Proxy, true);
        assert_eq!(update.cef_focus, None);
        assert_eq!(update.transfer, Some(FocusOwner::Browser));
        assert!(!update.ime_active);
        assert_eq!(focus.reconcile(FocusOwner::Outside, true).transfer, None);
    }

    #[test]
    fn window_hide_or_pause_blurs_without_moving_godot_focus() {
        let mut focus = FocusState {
            editable: true,
            ..Default::default()
        };
        focus.reconcile(FocusOwner::Proxy, true);
        let suspended = focus.reconcile(FocusOwner::Proxy, false);
        assert_eq!(suspended.cef_focus, Some(false));
        assert!(!suspended.ime_active);
        assert_eq!(suspended.transfer, None);
        let resumed = focus.reconcile(FocusOwner::Proxy, true);
        assert_eq!(resumed.cef_focus, Some(true));
        assert!(resumed.ime_active);
    }

    #[test]
    fn navigation_reasserts_host_focus_without_stealing_the_current_owner() {
        let mut focus = FocusState::default();
        assert_eq!(
            focus.reconcile(FocusOwner::Browser, true).cef_focus,
            Some(true)
        );
        // A new Chromium render widget starts unfocused, while the same Godot
        // control still owns focus. Navigation must invalidate the host cache.
        assert_eq!(focus.reconcile(FocusOwner::Browser, true).cef_focus, None);
        focus.invalidate_host_focus();
        assert_eq!(
            focus.reconcile(FocusOwner::Browser, true).cef_focus,
            Some(true)
        );

        // Completion arriving after the user moved to another control, or
        // while the window is inactive, must not reclaim focus.
        focus.invalidate_host_focus();
        let outside = focus.reconcile(FocusOwner::Outside, true);
        assert_eq!(outside.cef_focus, Some(false));
        assert_eq!(outside.transfer, None);
        focus.invalidate_host_focus();
        let inactive = focus.reconcile(FocusOwner::Browser, false);
        assert_eq!(inactive.cef_focus, Some(false));
        assert_eq!(inactive.transfer, None);
    }
}
