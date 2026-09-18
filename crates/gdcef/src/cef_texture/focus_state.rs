//! Pure focus transitions: the browser control and its IME proxy share one owner.

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
    pub editable: bool,
    cef_focused: Option<bool>,
}

impl FocusState {
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
}
