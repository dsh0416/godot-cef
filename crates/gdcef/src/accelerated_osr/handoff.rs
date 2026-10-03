//! Browser-local capture/publication ownership. No CEF frame survives paint.

use super::publication::{Publication, SharedPublication};
use super::{GodotTextureImporter, NativeCaptureTarget, SnapshotFormat};
use crate::render;
use cef::{AcceleratedPaintInfo, PaintElementType};
use godot::prelude::*;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

struct Stream {
    active: SharedPublication,
    // At most two resource generations exist. Keep the old display until its
    // replacement has been bound; intermediate resizes wait for retirement.
    previous: Option<SharedPublication>,
    active_published: bool,
    active_retiring: bool,
    requested_size: (u32, u32, SnapshotFormat),
    bound_display: Rid,
}

#[derive(Debug, PartialEq, Eq)]
enum ResizeAction {
    Keep,
    WaitForBinding,
    ReplaceAndKeepPrevious,
    RetireActive,
    FinishRetirement,
}

fn resize_action(
    active_retiring: bool,
    size_matches: bool,
    has_previous: bool,
    active_published: bool,
) -> ResizeAction {
    // Retirement is irreversible. Requesting the same dimensions again still
    // needs a fresh generation after the outstanding callbacks have drained.
    if active_retiring {
        ResizeAction::FinishRetirement
    } else if size_matches {
        ResizeAction::Keep
    } else if !has_previous {
        ResizeAction::ReplaceAndKeepPrevious
    } else if active_published {
        ResizeAction::WaitForBinding
    } else {
        ResizeAction::RetireActive
    }
}

fn check_generation_failures(active_failed: bool, previous_failed: bool) -> Result<(), String> {
    if active_failed || previous_failed {
        Err("Snapshot completion failed; resources quarantined".into())
    } else {
        Ok(())
    }
}

fn pool_info(pool: &SharedPublication) -> Result<(Rid, (u32, u32, SnapshotFormat)), String> {
    let pool = pool
        .lock()
        .map_err(|_| "Snapshot publication lock poisoned")?;
    Ok((
        pool.display_rid(),
        (pool.size().0, pool.size().1, pool.format()),
    ))
}

fn create_publication(size: (u32, u32, SnapshotFormat)) -> Result<SharedPublication, String> {
    let display = render::create_rd_texture_rid(size.0 as i32, size.1 as i32, size.2.rd_format())
        .map_err(|error| error.to_string())?;
    match Publication::create(display, size.0, size.1, size.2) {
        Ok(pool) => Ok(pool),
        Err(error) => {
            render::free_rd_texture(display);
            Err(error)
        }
    }
}

impl Stream {
    fn new(size: (u32, u32, SnapshotFormat)) -> Result<Self, String> {
        Ok(Self {
            active: create_publication(size)?,
            previous: None,
            active_published: false,
            active_retiring: false,
            requested_size: size,
            bound_display: Rid::Invalid,
        })
    }

    fn prepare_resize(&mut self) -> Result<(), String> {
        self.check_completion_failures()?;
        if let Some(previous) = &self.previous {
            let (previous_rid, _) = pool_info(previous)?;
            if previous_rid != self.bound_display {
                Publication::retire(previous);
                if previous
                    .lock()
                    .map_err(|_| "Snapshot lock poisoned")?
                    .is_drained()
                {
                    self.previous = None;
                }
            }
        }
        match resize_action(
            self.active_retiring,
            pool_info(&self.active)?.1 == self.requested_size,
            self.previous.is_some(),
            self.active_published,
        ) {
            ResizeAction::Keep | ResizeAction::WaitForBinding => return Ok(()),
            ResizeAction::ReplaceAndKeepPrevious => {
                let replacement = create_publication(self.requested_size)?;
                self.previous = Some(std::mem::replace(&mut self.active, replacement));
                self.active_published = false;
                return Ok(());
            }
            ResizeAction::RetireActive => {
                // A new resize arrived before this generation was displayable.
                // Keep the previous display bound while this generation drains.
                self.active_retiring = true;
                Publication::retire(&self.active);
            }
            ResizeAction::FinishRetirement => {}
        }
        if !self
            .active
            .lock()
            .map_err(|_| "Snapshot lock poisoned")?
            .is_drained()
        {
            return Ok(());
        }
        // Allocation failure leaves the old pool marked retiring. Neither its
        // invalid display nor its retired slots may be revived by another size.
        let replacement = create_publication(self.requested_size)?;
        self.active = replacement;
        self.active_retiring = false;
        self.active_published = false;
        Ok(())
    }

    fn check_completion_failures(&self) -> Result<(), String> {
        let failed = |pool: &SharedPublication| {
            pool.lock()
                .map(|pool| pool.has_failed())
                .map_err(|_| "Snapshot publication lock poisoned".to_string())
        };
        // A completion can fail after resize moved its generation to previous.
        // Quarantined slots never drain, so retrying resize would otherwise keep
        // invalidating CEF forever while waiting for impossible retirement.
        check_generation_failures(
            failed(&self.active)?,
            self.previous
                .as_ref()
                .map(failed)
                .transpose()?
                .unwrap_or(false),
        )
    }

    fn retire(&mut self) {
        Publication::retire(&self.active);
        if let Some(previous) = self.previous.take() {
            Publication::retire(&previous);
        }
    }
}

pub struct AcceleratedRenderState {
    importer: Option<GodotTextureImporter>,
    view: Stream,
    popup: Option<Stream>,
    popup_requested_size: Option<(u32, u32, SnapshotFormat)>,
    pub dst_rd_rid: Rid,
    pub dst_width: u32,
    pub dst_height: u32,
    pub popup_rd_rid: Option<Rid>,
    pub popup_width: u32,
    pub popup_height: u32,
    pub popup_dirty: bool,
    pub popup_has_content: bool,
    pub has_pending_copy: bool,
    frame_queued: bool,
    closed: bool,
    failure: Option<String>,
    repaint_requests: Arc<AtomicU8>,
}

/// Only native importer initialization can select the software renderer. Once
/// resources are being created, failures remain accelerated initialization errors.
#[derive(Debug, PartialEq, Eq)]
pub enum AcceleratedInitializationError {
    Importer(String),
    Resources(String),
}

fn initialize_render_state<I, T>(
    create_importer: impl FnOnce() -> Result<I, String>,
    create_resources: impl FnOnce(I) -> Result<T, String>,
) -> Result<T, AcceleratedInitializationError> {
    let importer = create_importer().map_err(AcceleratedInitializationError::Importer)?;
    create_resources(importer).map_err(AcceleratedInitializationError::Resources)
}

#[cfg(test)]
mod initialization_tests {
    use super::{AcceleratedInitializationError, initialize_render_state};
    use std::cell::Cell;

    #[test]
    fn importer_failure_selects_fallback_before_any_snapshot_allocation() {
        let resource_creation_attempted = Cell::new(false);
        let result = initialize_render_state(
            || Err::<(), _>("external memory unavailable".into()),
            |()| {
                resource_creation_attempted.set(true);
                Ok(())
            },
        );
        assert_eq!(
            result,
            Err(AcceleratedInitializationError::Importer(
                "external memory unavailable".into()
            ))
        );
        assert!(!resource_creation_attempted.get());
    }

    #[test]
    fn resource_failure_drops_the_single_importer_without_selecting_fallback() {
        struct Importer<'a>(&'a Cell<bool>);
        impl Drop for Importer<'_> {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        let creations = Cell::new(0);
        let dropped = Cell::new(false);
        let result = initialize_render_state(
            || {
                creations.set(creations.get() + 1);
                Ok(Importer(&dropped))
            },
            |_importer| Err::<(), _>("snapshot allocation failed".into()),
        );
        assert_eq!(
            result,
            Err(AcceleratedInitializationError::Resources(
                "snapshot allocation failed".into()
            ))
        );
        assert_eq!(creations.get(), 1);
        assert!(dropped.get());
    }
}

#[cfg(test)]
mod resize_tests {
    use super::{ResizeAction, check_generation_failures, resize_action};
    use crate::accelerated_osr::snapshot_pool::SnapshotPool;

    #[test]
    fn oscillating_resize_finishes_retirement_before_reusing_same_dimensions() {
        // B is bound, then A is allocated as a replacement.
        assert_eq!(
            resize_action(false, false, false, true),
            ResizeAction::ReplaceAndKeepPrevious
        );
        // Before A can publish, a request for C retires A while B stays bound.
        assert_eq!(
            resize_action(false, false, true, false),
            ResizeAction::RetireActive
        );
        // A is requested again before its bootstrap callbacks finish. A retired
        // pool cannot become live simply because its dimensions match again.
        assert_eq!(
            resize_action(true, true, true, false),
            ResizeAction::FinishRetirement
        );
        // If replacement allocation fails, retry remains in retirement, even
        // if the previous display has meanwhile been detached.
        assert_eq!(
            resize_action(true, true, false, false),
            ResizeAction::FinishRetirement
        );
        // Only successful allocation of a live generation permits keeping it.
        assert_eq!(resize_action(false, true, true, false), ResizeAction::Keep);
    }

    #[test]
    fn published_replacement_is_preserved_until_main_thread_rebinds() {
        assert_eq!(
            resize_action(false, false, true, true),
            ResizeAction::WaitForBinding
        );
        assert_eq!(
            resize_action(false, false, false, true),
            ResizeAction::ReplaceAndKeepPrevious
        );
    }

    #[test]
    fn late_failure_in_previous_generation_stops_healthy_replacement() {
        let mut previous = SnapshotPool::new(1, [10]);
        let pending_readback = previous.bootstrap_tokens()[0];
        let mut active = SnapshotPool::new(2, [20]);
        let bootstrap = active.bootstrap_tokens()[0];
        assert!(active.bootstrap_complete(bootstrap));
        assert!(check_generation_failures(active.has_failed(), previous.has_failed()).is_ok());

        // Resize has replaced this generation before its readback reports an
        // error. It must remain quarantined while stopping the entire stream.
        previous.retire();
        assert!(previous.fail(pending_readback));
        assert!(previous.take_retired().is_empty());
        assert!(!previous.is_drained());
        assert!(!active.has_failed());
        assert!(check_generation_failures(active.has_failed(), previous.has_failed()).is_err());
    }
}

impl AcceleratedRenderState {
    /// Initialization occurs before browser construction, never in paint.
    pub fn create(
        width: u32,
        height: u32,
    ) -> Result<Arc<Mutex<Self>>, AcceleratedInitializationError> {
        render::on_render_thread_sync(move || {
            initialize_render_state(GodotTextureImporter::new, |importer| {
                let mut view = Stream::new((width, height, SnapshotFormat::Bgra8))?;
                let dst_rd_rid = pool_info(&view.active)?.0;
                view.bound_display = dst_rd_rid;
                Ok(Arc::new(Mutex::new(Self {
                    importer: Some(importer),
                    view,
                    popup: None,
                    popup_requested_size: None,
                    dst_rd_rid,
                    dst_width: width,
                    dst_height: height,
                    popup_rd_rid: None,
                    popup_width: 0,
                    popup_height: 0,
                    popup_dirty: false,
                    popup_has_content: false,
                    has_pending_copy: false,
                    frame_queued: false,
                    closed: false,
                    failure: None,
                    repaint_requests: Arc::new(AtomicU8::new(0)),
                })))
            })
        })
        .map_err(AcceleratedInitializationError::Resources)?
    }

    /// CEF UI callback. Never calls Godot/RD and never waits for pool capacity.
    pub fn capture(&mut self, kind: PaintElementType, info: &AcceleratedPaintInfo) -> bool {
        if self.closed
            || self.failure.is_some()
            || info.extra.coded_size.width <= 0
            || info.extra.coded_size.height <= 0
        {
            return false;
        }
        let format = match SnapshotFormat::from_cef(info) {
            Ok(format) => format,
            Err(error) => {
                self.fail(error);
                return false;
            }
        };
        let size = (
            info.extra.coded_size.width as u32,
            info.extra.coded_size.height as u32,
            format,
        );
        let stream = if kind == PaintElementType::VIEW {
            &mut self.view
        } else if kind == PaintElementType::POPUP {
            self.popup_requested_size = Some(size);
            let Some(stream) = &mut self.popup else {
                return false;
            };
            stream
        } else {
            return false;
        };
        stream.requested_size = size;
        let shared = Arc::clone(&stream.active);
        let slot = {
            let Ok(mut pool) = shared.lock() else {
                return false;
            };
            if (pool.size().0, pool.size().1, pool.format()) != size {
                return false;
            }
            let Some(slot) = pool.acquire_capture() else {
                return false;
            };
            slot
        };
        let Some(importer) = self.importer.as_mut() else {
            if let Ok(mut pool) = shared.lock() {
                pool.cancel_capture(slot.token);
            }
            return false;
        };
        let result = importer.capture(
            info,
            NativeCaptureTarget {
                native_handle: slot.native_handle,
                width: slot.width,
                height: slot.height,
                format: slot.format,
            },
        );
        if let Ok(mut pool) = shared.lock() {
            match &result {
                Ok(()) => {
                    pool.capture_complete(slot.token);
                }
                Err(_) => {
                    pool.capture_failed(slot.token);
                }
            }
        }
        match result {
            Ok(()) => {
                self.has_pending_copy = true;
                true
            }
            Err(error) => {
                self.fail(error);
                false
            }
        }
    }

    pub fn set_repaint_requests(&mut self, requests: Arc<AtomicU8>) {
        self.repaint_requests = requests;
    }

    /// Read on the main thread while holding render state, then release that
    /// lock before calling CEF. Keep retries pending until a capture succeeds.
    pub fn pending_repaints(&self) -> u8 {
        if self.closed || self.failure.is_some() {
            0
        } else {
            self.repaint_requests.load(Ordering::Acquire)
        }
    }

    fn fail(&mut self, error: String) {
        if self.failure.is_none() {
            godot::global::godot_error!(
                "[AcceleratedOSR] {error}; retaining the last published frame (no software fallback)"
            );
            self.failure = Some(error);
        }
    }

    /// Main-thread bindings are acknowledged before enqueuing retirement work.
    pub fn acknowledge_view(&mut self, rid: Rid) {
        self.view.bound_display = rid;
    }

    pub fn acknowledge_popup(&mut self, rid: Rid) {
        if let Some(popup) = &mut self.popup {
            popup.bound_display = rid;
        }
    }

    pub fn queue_frame(shared: &Arc<Mutex<Self>>) {
        {
            let Ok(mut state) = shared.lock() else { return };
            if state.closed || state.failure.is_some() || state.frame_queued {
                return;
            }
            state.frame_queued = true;
        }
        let shared = Arc::clone(shared);
        render::on_render_thread(move || {
            let Ok(mut state) = shared.lock() else { return };
            state.frame_queued = false;
            if state.closed || state.failure.is_some() {
                return;
            }
            if let Err(error) = state.publish_frame() {
                state.fail(error);
            }
        });
    }

    fn publish_frame(&mut self) -> Result<(), String> {
        self.view.prepare_resize()?;
        if let Some(size) = self.popup_requested_size {
            if let Some(popup) = &mut self.popup {
                popup.requested_size = size;
                popup.prepare_resize()?;
            } else {
                self.popup = Some(Stream::new(size)?);
            }
        }
        let Some(importer) = &self.importer else {
            return Ok(());
        };
        let ready = [&self.view].into_iter().chain(self.popup.iter()).try_fold(
            false,
            |ready, stream| {
                stream
                    .active
                    .lock()
                    .map(|pool| ready || pool.has_ready())
                    .map_err(|_| "Snapshot lock poisoned")
            },
        )?;
        if ready {
            importer.prepare_publication()?;
        }
        if Publication::publish(&self.view.active)? {
            self.view.active_published = true;
            let (rid, (width, height, _)) = pool_info(&self.view.active)?;
            self.dst_rd_rid = rid;
            self.dst_width = width;
            self.dst_height = height;
        }
        if let Some(popup) = &mut self.popup
            && Publication::publish(&popup.active)?
        {
            popup.active_published = true;
            let (rid, (width, height, _)) = pool_info(&popup.active)?;
            self.popup_rd_rid = Some(rid);
            self.popup_width = width;
            self.popup_height = height;
            self.popup_dirty = true;
            self.popup_has_content = true;
        }
        for stream in [&self.view].into_iter().chain(self.popup.iter()) {
            stream.check_completion_failures()?;
        }
        self.has_pending_copy = false;
        Ok(())
    }

    /// Caller detaches Texture2DRD bindings before this call. Godot tracks its
    /// display reads; snapshot callbacks retain each source until actual completion.
    pub fn shutdown(shared: &Arc<Mutex<Self>>) {
        {
            let Ok(mut state) = shared.lock() else { return };
            if state.closed {
                return;
            }
            state.closed = true;
        }
        let shared = Arc::clone(shared);
        render::on_render_thread(move || {
            if let Ok(mut state) = shared.lock() {
                state.view.retire();
                if let Some(popup) = &mut state.popup {
                    popup.retire();
                }
                state.importer.take();
            }
        });
    }
}
