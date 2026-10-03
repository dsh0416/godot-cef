//! RenderingDevice publication and completion ownership for accelerated OSR.
//!
//! Creation, publication, and retirement run on Godot's rendering thread. CEF
//! only borrows a preallocated native target, completes its native capture, and
//! changes CPU metadata. No capture ever waits for a future Godot frame.

use super::SnapshotFormat;
use super::snapshot_pool::{SnapshotPool, SnapshotToken};
use godot::classes::rendering_device::{
    DriverResource, TextureSamples, TextureType, TextureUsageBits,
};
use godot::classes::{RdTextureFormat, RdTextureView, RenderingDevice, RenderingServer};
use godot::global::{Error, godot_error};
use godot::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const SNAPSHOT_SLOTS: usize = 3;
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

pub type SharedPublication = Arc<Mutex<Publication>>;

#[derive(Clone, Copy)]
pub struct CaptureSlot {
    pub token: SnapshotToken,
    pub native_handle: u64,
    pub width: u32,
    pub height: u32,
    pub format: SnapshotFormat,
}

#[derive(Clone, Copy)]
struct SlotTextures {
    staging: Rid,
    sentinel: Rid,
    native_handle: u64,
}

pub struct Publication {
    pool: SnapshotPool<SlotTextures>,
    display: Rid,
    width: u32,
    height: u32,
    format: SnapshotFormat,
}

impl Publication {
    /// Must run on the rendering thread. Ownership of `display` transfers only
    /// on success. It must support both CAN_COPY_FROM and CAN_COPY_TO.
    ///
    /// A failed bootstrap stays quarantined; an API error or an empty readback
    /// never licenses reusing/freeing a potentially in-flight resource.
    pub fn create(
        display: Rid,
        width: u32,
        height: u32,
        format: SnapshotFormat,
    ) -> Result<SharedPublication, String> {
        if !display.is_valid() || width == 0 || height == 0 {
            return Err("Invalid snapshot display or dimensions".to_string());
        }
        let mut rd = rendering_device()?;
        let mut resources = Vec::with_capacity(SNAPSHOT_SLOTS);
        for _ in 0..SNAPSHOT_SLOTS {
            match create_slot(&mut rd, width, height, format) {
                Ok(slot) => resources.push(slot),
                Err(error) => {
                    for slot in resources {
                        free_slot(&mut rd, slot);
                    }
                    return Err(error);
                }
            }
        }
        let pool = SnapshotPool::new(
            NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            resources.iter().copied(),
        );
        let tokens = pool.bootstrap_tokens();
        let shared = Arc::new(Mutex::new(Self {
            pool,
            display,
            width,
            height,
            format,
        }));
        for (token, slot) in tokens.into_iter().zip(resources) {
            // The initial upload establishes an initialized Godot resource. The
            // copy leaves staging in COPY_SOURCE/TRANSFER_SRC and ties readback
            // completion to the bootstrap's last staging access.
            let transparent =
                PackedByteArray::from(vec![0; width as usize * height as usize * 4].as_slice());
            let result = check(
                rd.texture_update(slot.staging, 0, &transparent),
                "snapshot bootstrap initialization",
            )
            .and_then(|()| copy(&mut rd, slot.staging, slot.sentinel, 1, 1))
            .and_then(|()| read_completion(&mut rd, &shared, token, slot.sentinel, true));
            if let Err(error) = result {
                quarantine(&shared, token, &error);
            }
        }
        Ok(shared)
    }

    pub fn display_rid(&self) -> Rid {
        self.display
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn format(&self) -> SnapshotFormat {
        self.format
    }

    pub fn has_ready(&self) -> bool {
        self.pool.has_ready()
    }

    pub fn is_drained(&self) -> bool {
        self.pool.is_drained()
    }

    pub fn has_failed(&self) -> bool {
        self.pool.has_failed()
    }

    pub fn acquire_capture(&mut self) -> Option<CaptureSlot> {
        let token = self.pool.acquire_capture()?;
        let slot = self.pool.get(token)?;
        Some(CaptureSlot {
            token,
            native_handle: slot.native_handle,
            width: self.width,
            height: self.height,
            format: self.format,
        })
    }

    /// The producer must finish its copy and restore staging's native state
    /// before calling this. Visibility to Godot's queue is backend-specific.
    pub fn capture_complete(&mut self, token: SnapshotToken) -> bool {
        self.pool.capture_complete(token)
    }

    pub fn capture_failed(&mut self, token: SnapshotToken) -> bool {
        self.pool.fail(token)
    }

    /// Only valid when capture rejected the source before submitting native work.
    pub fn cancel_capture(&mut self, token: SnapshotToken) -> bool {
        self.pool.cancel_capture(token)
    }

    /// Called by a frame_pre_draw task on the rendering thread, after the native
    /// backend has established producer visibility to Godot's queue.
    pub fn publish(shared: &SharedPublication) -> Result<bool, String> {
        let mut rd = rendering_device()?;
        let (token, slot, display, width, height) = {
            let mut publication = shared
                .lock()
                .map_err(|_| "Snapshot publication lock poisoned")?;
            let Some(token) = publication.pool.submit_latest() else {
                return Ok(false);
            };
            let slot = publication
                .pool
                .get(token)
                .copied()
                .ok_or("Snapshot slot missing")?;
            (
                token,
                slot,
                publication.display,
                publication.width,
                publication.height,
            )
        };
        // The intermediate display read is intentional: Godot may reorder two
        // independent reads of staging. Reading the copy's destination gives the
        // sentinel/readback a real dependency on completion of the full copy.
        let result = copy(&mut rd, slot.staging, display, width, height)
            .and_then(|()| copy(&mut rd, display, slot.sentinel, 1, 1))
            .and_then(|()| read_completion(&mut rd, shared, token, slot.sentinel, false));
        match result {
            Ok(()) => Ok(true),
            Err(error) => {
                quarantine(shared, token, &error);
                Err(error)
            }
        }
    }

    /// Run after the display has been detached, on the rendering thread. Readback
    /// callbacks hold their own Arc until retirement is safe; no frame delay is
    /// used as completion evidence. Repeating this is harmless.
    pub fn retire(shared: &SharedPublication) {
        if let Ok(mut publication) = shared.lock() {
            publication.pool.retire();
        }
        collect_retired(shared);
    }
}

fn rendering_device() -> Result<Gd<RenderingDevice>, String> {
    RenderingServer::singleton()
        .get_rendering_device()
        .ok_or_else(|| "RenderingDevice unavailable for snapshot publication".to_string())
}

fn create_texture(
    rd: &mut RenderingDevice,
    width: u32,
    height: u32,
    snapshot_format: SnapshotFormat,
) -> Result<Rid, String> {
    let mut format = RdTextureFormat::new_gd();
    format.set_format(snapshot_format.rd_format());
    format.set_width(width);
    format.set_height(height);
    format.set_depth(1);
    format.set_array_layers(1);
    format.set_mipmaps(1);
    format.set_texture_type(TextureType::TYPE_2D);
    format.set_samples(TextureSamples::SAMPLES_1);
    // Bootstrap uses an initial zero upload, avoiding sRGB RTV/UAV clears.
    format.set_usage_bits(
        TextureUsageBits::CAN_COPY_FROM_BIT
            | TextureUsageBits::CAN_COPY_TO_BIT
            | TextureUsageBits::CAN_UPDATE_BIT,
    );
    let rid = rd.texture_create(&format, &RdTextureView::new_gd());
    if rid.is_valid() {
        Ok(rid)
    } else {
        Err(format!(
            "Failed to create snapshot texture {width}x{height}"
        ))
    }
}

fn create_slot(
    rd: &mut RenderingDevice,
    width: u32,
    height: u32,
    format: SnapshotFormat,
) -> Result<SlotTextures, String> {
    let staging = create_texture(rd, width, height, format)?;
    let sentinel = match create_texture(rd, 1, 1, format) {
        Ok(rid) => rid,
        Err(error) => {
            rd.free_rid(staging);
            return Err(error);
        }
    };
    // Godot 4.6 exposes each backend's native texture through TEXTURE. These
    // handles are borrowed; the RD RIDs keep their resources alive.
    // https://github.com/godotengine/godot/blob/89cea143987d564363e15d207438530651d943ac/drivers/d3d12/rendering_device_driver_d3d12.cpp#L5150-L5156
    let native_handle = rd.get_driver_resource(DriverResource::TEXTURE, staging, 0);
    if native_handle == 0 {
        rd.free_rid(staging);
        rd.free_rid(sentinel);
        return Err("RenderingDevice returned a null snapshot native texture".to_string());
    }
    Ok(SlotTextures {
        staging,
        sentinel,
        native_handle,
    })
}

fn copy(
    rd: &mut RenderingDevice,
    from: Rid,
    to: Rid,
    width: u32,
    height: u32,
) -> Result<(), String> {
    check(
        rd.texture_copy(
            from,
            to,
            Vector3::ZERO,
            Vector3::ZERO,
            Vector3::new(width as f32, height as f32, 1.0),
            0,
            0,
            0,
            0,
        ),
        "snapshot texture copy",
    )
}

fn check(result: Error, operation: &str) -> Result<(), String> {
    if result == Error::OK {
        Ok(())
    } else {
        Err(format!("{operation} failed: {result:?}"))
    }
}

fn read_completion(
    rd: &mut RenderingDevice,
    shared: &SharedPublication,
    token: SnapshotToken,
    sentinel: Rid,
    bootstrap: bool,
) -> Result<(), String> {
    let shared = Arc::clone(shared);
    let callback = Callable::from_sync_fn("cef_snapshot_complete", move |args: &[&Variant]| {
        let valid = args
            .first()
            .and_then(|arg| arg.try_to::<PackedByteArray>().ok())
            .is_some_and(|bytes| bytes.len() == 4);
        // Godot currently invokes this after its frame fence on the rendering
        // thread. Route through its public thread API so cleanup retains that
        // contract even if callback dispatch changes in a later engine version.
        let completion = Arc::clone(&shared);
        RenderingServer::singleton().call_on_render_thread(&Callable::from_sync_fn(
            "cef_snapshot_reclaim",
            move |_: &[&Variant]| {
                if let Ok(mut publication) = completion.lock() {
                    if valid {
                        if bootstrap {
                            publication.pool.bootstrap_complete(token);
                        } else {
                            publication.pool.publication_complete(token);
                        }
                    } else {
                        publication.pool.fail(token);
                        godot_error!(
                            "[AcceleratedOSR] Invalid snapshot completion; slot quarantined"
                        );
                    }
                }
                collect_retired(&completion);
            },
        ));
    });
    check(
        rd.texture_get_data_async(sentinel, 0, &callback),
        "snapshot readback",
    )
}

fn quarantine(shared: &SharedPublication, token: SnapshotToken, error: &str) {
    if let Ok(mut publication) = shared.lock() {
        publication.pool.fail(token);
    }
    godot_error!("[AcceleratedOSR] {error}; snapshot slot quarantined");
}

fn free_slot(rd: &mut RenderingDevice, slot: SlotTextures) {
    rd.free_rid(slot.staging);
    rd.free_rid(slot.sentinel);
}

fn collect_retired(shared: &SharedPublication) {
    let Ok(mut rd) = rendering_device() else {
        return;
    };
    let (slots, display) = {
        let Ok(mut publication) = shared.lock() else {
            return;
        };
        let slots = publication.pool.take_retired();
        let display = if publication.pool.is_drained() {
            std::mem::replace(&mut publication.display, Rid::Invalid)
        } else {
            Rid::Invalid
        };
        (slots, display)
    };
    for slot in slots {
        free_slot(&mut rd, slot);
    }
    if display.is_valid() {
        rd.free_rid(display);
    }
}

// Deliberately no unconditional Drop/free_rid: failed native work/readback has no
// safe completion proof. Such faulted RIDs remain allocated until device teardown
// instead of turning an error path into GPU use-after-free.
