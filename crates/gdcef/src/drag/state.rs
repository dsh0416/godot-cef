use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_SESSION: AtomicI64 = AtomicI64::new(1);

pub(crate) type SourceDragHandle = Arc<Mutex<SourceDragState>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceDragSession {
    pub id: i64,
    pub position: (i32, i32),
    pub allowed_ops: u32,
}

/// Shared with the CEF render handler so acceptance precedes notification.
#[derive(Default)]
pub(crate) struct SourceDragState {
    pub enabled: bool,
    active: Option<SourceDragSession>,
}

impl SourceDragState {
    pub fn start(&mut self, position: (i32, i32), allowed_ops: u32) -> Option<i64> {
        if !self.enabled || self.active.is_some() {
            return None;
        }
        let id = NEXT_SESSION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .ok()?;
        self.active = Some(SourceDragSession {
            id,
            position,
            allowed_ops,
        });
        Some(id)
    }

    pub fn active_id(&self) -> Option<i64> {
        self.active.map(|session| session.id)
    }

    /// Clear before calling CEF; repeated and stale completions are harmless.
    pub fn finish(&mut self, id: Option<i64>) -> Option<SourceDragSession> {
        if id.is_some() && id != self.active_id() {
            return None;
        }
        self.active.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unhandled_and_overlapping_drags() {
        let mut state = SourceDragState::default();
        assert_eq!(state.start((1, 2), 1), None);
        state.enabled = true;
        let id = state.start((1, 2), 1).unwrap();
        assert_eq!(state.active_id(), Some(id));
        assert_eq!(state.start((3, 4), 16), None);
        assert_eq!(state.finish(Some(id)).unwrap().position, (1, 2));
        assert_eq!(state.finish(Some(id)), None);
    }

    #[test]
    fn late_completion_cannot_end_a_new_session_or_browser() {
        let mut first = SourceDragState {
            enabled: true,
            ..Default::default()
        };
        let old = first.start((0, 0), 1).unwrap();
        first.finish(None).unwrap();
        let new = first.start((1, 1), 16).unwrap();
        assert_eq!(first.finish(Some(old)), None);
        assert_eq!(first.active_id(), Some(new));
        let mut replacement = SourceDragState {
            enabled: true,
            ..Default::default()
        };
        let replacement_id = replacement.start((2, 2), 1).unwrap();
        assert_eq!(replacement.finish(Some(old)), None);
        assert_eq!(replacement.active_id(), Some(replacement_id));
    }

    #[test]
    fn rejected_notification_can_roll_back_acceptance() {
        let mut state = SourceDragState {
            enabled: true,
            ..Default::default()
        };
        let id = state.start((0, 0), 1).unwrap();
        state.finish(Some(id)).unwrap();
        assert!(state.start((0, 0), 1).is_some());
        state.enabled = false;
        assert!(state.finish(None).is_some());
        assert_eq!(state.start((0, 0), 1), None);
    }
}
