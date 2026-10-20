//! Bounded ownership state machine for externally captured GPU snapshots.
//!
//! No Godot or graphics API calls belong here. A token identifies both a resource
//! generation and a particular use of a slot, so a late callback cannot release
//! either a replacement texture or a newer submission of the same texture.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotToken {
    generation: u64,
    slot: usize,
    serial: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    Bootstrapping,
    Free,
    Capturing,
    Ready,
    Submitted,
    Retired,
    Failed,
}

struct Slot<T> {
    value: Option<T>,
    state: SlotState,
    serial: u64,
}

pub struct SnapshotPool<T> {
    generation: u64,
    next_serial: u64,
    retired: bool,
    slots: Vec<Slot<T>>,
}

impl<T> SnapshotPool<T> {
    pub fn new(generation: u64, resources: impl IntoIterator<Item = T>) -> Self {
        Self {
            generation,
            next_serial: 1,
            retired: false,
            slots: resources
                .into_iter()
                .map(|value| Slot {
                    value: Some(value),
                    state: SlotState::Bootstrapping,
                    serial: 0,
                })
                .collect(),
        }
    }

    pub fn bootstrap_tokens(&self) -> Vec<SnapshotToken> {
        (0..self.slots.len())
            .map(|slot| SnapshotToken {
                generation: self.generation,
                slot,
                serial: 0,
            })
            .collect()
    }

    pub fn get(&self, token: SnapshotToken) -> Option<&T> {
        self.matching_slot(token)?.value.as_ref()
    }

    fn matching_slot(&self, token: SnapshotToken) -> Option<&Slot<T>> {
        let slot = self.slots.get(token.slot)?;
        (self.generation == token.generation && slot.serial == token.serial).then_some(slot)
    }

    fn transition(&mut self, token: SnapshotToken, from: SlotState, to: SlotState) -> bool {
        if self
            .matching_slot(token)
            .is_none_or(|slot| slot.state != from)
        {
            return false;
        }
        self.slots[token.slot].state = to;
        true
    }

    pub fn bootstrap_complete(&mut self, token: SnapshotToken) -> bool {
        self.transition(token, SlotState::Bootstrapping, self.available_state())
    }

    fn available_state(&self) -> SlotState {
        if self.retired {
            SlotState::Retired
        } else {
            SlotState::Free
        }
    }

    /// Never blocks and never steals a slot which a GPU may still be reading.
    pub fn acquire_capture(&mut self) -> Option<SnapshotToken> {
        if self.retired {
            return None;
        }
        let index = self
            .slots
            .iter()
            .position(|slot| slot.state == SlotState::Free)?;
        let serial = self.next_serial;
        self.next_serial = self.next_serial.checked_add(1)?;
        self.slots[index].serial = serial;
        self.slots[index].state = SlotState::Capturing;
        Some(SnapshotToken {
            generation: self.generation,
            slot: index,
            serial,
        })
    }

    /// Only call once the native producer has completed and restored COPY_SOURCE.
    pub fn capture_complete(&mut self, token: SnapshotToken) -> bool {
        let state = if self.retired {
            SlotState::Retired
        } else {
            SlotState::Ready
        };
        self.transition(token, SlotState::Capturing, state)
    }

    /// A rejected capture can be cancelled only if no native work was submitted.
    pub fn cancel_capture(&mut self, token: SnapshotToken) -> bool {
        self.transition(token, SlotState::Capturing, self.available_state())
    }

    /// Select the newest completed frame, releasing only other completed frames.
    pub fn submit_latest(&mut self) -> Option<SnapshotToken> {
        if self.retired {
            return None;
        }
        let (index, serial) = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.state == SlotState::Ready)
            .max_by_key(|(_, slot)| slot.serial)
            .map(|(index, slot)| (index, slot.serial))?;
        for slot in &mut self.slots {
            if slot.state == SlotState::Ready {
                slot.state = SlotState::Free;
            }
        }
        self.slots[index].state = SlotState::Submitted;
        Some(SnapshotToken {
            generation: self.generation,
            slot: index,
            serial,
        })
    }

    /// The completion must be downstream of the staging read on the GPU.
    pub fn publication_complete(&mut self, token: SnapshotToken) -> bool {
        self.transition(token, SlotState::Submitted, self.available_state())
    }

    /// An error is not completion evidence. Quarantine the resource permanently.
    pub fn fail(&mut self, token: SnapshotToken) -> bool {
        let Some(slot) = self.matching_slot(token) else {
            return false;
        };
        if !matches!(
            slot.state,
            SlotState::Bootstrapping | SlotState::Capturing | SlotState::Submitted
        ) {
            return false;
        }
        self.slots[token.slot].state = SlotState::Failed;
        true
    }

    pub fn retire(&mut self) {
        self.retired = true;
        for slot in &mut self.slots {
            if matches!(slot.state, SlotState::Free | SlotState::Ready) {
                slot.state = SlotState::Retired;
            }
        }
    }

    /// Return only resources for which no native or Godot work remains in flight.
    pub fn take_retired(&mut self) -> Vec<T> {
        self.slots
            .iter_mut()
            .filter(|slot| slot.state == SlotState::Retired)
            .filter_map(|slot| slot.value.take())
            .collect()
    }

    pub fn is_drained(&self) -> bool {
        self.retired
            && self
                .slots
                .iter()
                .all(|slot| slot.state == SlotState::Retired)
    }

    pub fn has_ready(&self) -> bool {
        !self.retired && self.slots.iter().any(|slot| slot.state == SlotState::Ready)
    }

    pub fn has_failed(&self) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.state == SlotState::Failed)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ready_pool(generation: u64) -> SnapshotPool<u32> {
        let mut pool = SnapshotPool::new(generation, [10, 11, 12]);
        for token in pool.bootstrap_tokens() {
            assert!(pool.bootstrap_complete(token));
        }
        pool
    }

    #[test]
    fn bootstrap_and_exhaustion_never_reuse_busy_slots() {
        let mut pool = SnapshotPool::new(1, [10]);
        assert!(pool.acquire_capture().is_none());
        let token = pool.bootstrap_tokens()[0];
        assert!(pool.bootstrap_complete(token));
        let capture = pool.acquire_capture().unwrap();
        assert!(pool.acquire_capture().is_none());
        assert!(pool.capture_complete(capture));
        assert!(pool.acquire_capture().is_none());
        assert_eq!(pool.submit_latest(), Some(capture));
        assert!(pool.acquire_capture().is_none());
        assert!(pool.publication_complete(capture));
        assert!(pool.acquire_capture().is_some());
    }

    #[test]
    fn latest_completed_frame_wins_without_reclaiming_submission() {
        let mut pool = ready_pool(1);
        let first = pool.acquire_capture().unwrap();
        pool.capture_complete(first);
        assert_eq!(pool.submit_latest(), Some(first));
        let older = pool.acquire_capture().unwrap();
        let latest = pool.acquire_capture().unwrap();
        pool.capture_complete(latest);
        pool.capture_complete(older);
        assert_eq!(pool.submit_latest(), Some(latest));
        let reusable = pool.acquire_capture().unwrap();
        assert_eq!(reusable.slot, older.slot);
        assert_eq!(pool.slots[first.slot].state, SlotState::Submitted);
        assert_eq!(pool.slots[latest.slot].state, SlotState::Submitted);
    }

    #[test]
    fn duplicate_and_old_use_callbacks_cannot_release_new_capture() {
        let mut pool = ready_pool(1);
        let old = pool.acquire_capture().unwrap();
        pool.capture_complete(old);
        pool.submit_latest();
        assert!(pool.publication_complete(old));
        assert!(!pool.publication_complete(old));
        let new = pool.acquire_capture().unwrap();
        assert_eq!(old.slot, new.slot);
        assert!(!pool.publication_complete(old));
        assert!(!pool.fail(old));
        assert_eq!(pool.slots[new.slot].state, SlotState::Capturing);
    }

    #[test]
    fn replacement_generation_rejects_old_completion() {
        let mut old = ready_pool(8);
        let old_token = old.acquire_capture().unwrap();
        let mut replacement = ready_pool(9);
        let new_token = replacement.acquire_capture().unwrap();
        replacement.capture_complete(new_token);
        replacement.submit_latest();
        assert!(!replacement.publication_complete(old_token));
        assert_eq!(
            replacement.slots[new_token.slot].state,
            SlotState::Submitted
        );
    }

    #[test]
    fn retirement_waits_for_bootstrap_capture_and_publication() {
        let mut pool = SnapshotPool::new(1, [10, 11, 12]);
        let bootstrap = pool.bootstrap_tokens();
        pool.bootstrap_complete(bootstrap[0]);
        pool.bootstrap_complete(bootstrap[1]);
        let submitted = pool.acquire_capture().unwrap();
        pool.capture_complete(submitted);
        pool.submit_latest();
        let capturing = pool.acquire_capture().unwrap();
        pool.retire();
        assert!(pool.acquire_capture().is_none());
        assert!(pool.take_retired().is_empty());
        assert!(!pool.is_drained());
        pool.bootstrap_complete(bootstrap[2]);
        assert_eq!(pool.take_retired(), [12]);
        pool.capture_complete(capturing);
        assert_eq!(pool.take_retired(), [11]);
        pool.publication_complete(submitted);
        assert_eq!(pool.take_retired(), [10]);
        assert!(pool.is_drained());
        assert!(pool.take_retired().is_empty());
    }

    #[test]
    fn failed_readback_is_quarantined_even_after_retirement() {
        let mut pool = ready_pool(1);
        let token = pool.acquire_capture().unwrap();
        pool.capture_complete(token);
        pool.submit_latest();
        assert!(pool.fail(token));
        assert!(!pool.publication_complete(token));
        pool.retire();
        assert_eq!(pool.take_retired(), [11, 12]);
        assert!(!pool.is_drained());
        assert_eq!(pool.get(token), Some(&10));
    }

    #[test]
    fn rejected_unsubmitted_capture_can_be_cancelled() {
        let mut pool = ready_pool(1);
        let token = pool.acquire_capture().unwrap();
        assert!(pool.cancel_capture(token));
        assert!(!pool.cancel_capture(token));
        pool.retire();
        assert_eq!(pool.take_retired(), [10, 11, 12]);
        assert!(pool.is_drained());
    }
}
