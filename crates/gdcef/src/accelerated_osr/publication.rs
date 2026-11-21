//! RenderingDevice publication and completion ownership for accelerated OSR.
//!
//! Creation, publication, and retirement run on Godot's rendering thread. CEF
//! only borrows a preallocated native target, completes its native capture, and
//! changes CPU metadata. No capture ever waits for a future Godot frame.

use super::SnapshotFormat;
use super::snapshot_pool::{SnapshotPool, SnapshotToken};
use godot::classes::rendering_device::{
    DataFormat, DriverResource, TextureSamples, TextureType, TextureUsageBits,
};
#[cfg(target_os = "windows")]
use godot::classes::rendering_device::{ShaderStage, UniformType};
#[cfg(target_os = "windows")]
use godot::classes::{RdSamplerState, RdShaderSpirv, RdUniform};
use godot::classes::{RdTextureFormat, RdTextureView, RenderingDevice, RenderingServer};
use godot::global::{Error, godot_error};
use godot::prelude::*;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Precompiled SPIR-V for the D3D12 BGRA8 -> RGBA8 display publish pass:
/// ```glsl
/// #version 450
/// layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;
/// layout(set = 0, binding = 0) uniform sampler2D src_tex;
/// layout(rgba8, set = 0, binding = 1) uniform restrict writeonly image2D dst_img;
/// void main() {
///     ivec2 pos = ivec2(gl_GlobalInvocationID.xy);
///     ivec2 size = imageSize(dst_img);
///     if (pos.x < size.x && pos.y < size.y) {
///         imageStore(dst_img, pos, texelFetch(src_tex, pos, 0));
///     }
/// }
/// ```
#[cfg(target_os = "windows")]
const BGRA_TO_RGBA_COMPUTE_SPIRV: &[u8] = include_bytes!("bgra_to_rgba.comp.spv");

const SNAPSHOT_SLOTS: usize = 3;
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static PENDING_READBACKS: AtomicUsize = AtomicUsize::new(0);

/// Tracks callbacks which still require live Godot bindings, not GPU completion.
/// Dropping a cancelled callback removes its registration without releasing any
/// slot: only the existing checked completion transitions can do that.
struct PendingReadback;

impl PendingReadback {
    fn new() -> Self {
        PENDING_READBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for PendingReadback {
    fn drop(&mut self) {
        PENDING_READBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

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
    #[cfg(target_os = "windows")]
    staging_sentinel: Rid,
    #[cfg(target_os = "windows")]
    uniform_set: Rid,
    native_handle: u64,
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy)]
struct D3D12BgraBlit {
    shader: Rid,
    pipeline: Rid,
    sampler: Rid,
}

#[cfg(target_os = "windows")]
impl D3D12BgraBlit {
    fn create(rd: &mut RenderingDevice) -> Result<Self, String> {
        let mut spirv = RdShaderSpirv::new_gd();
        let bytecode = PackedByteArray::from(BGRA_TO_RGBA_COMPUTE_SPIRV);
        spirv.set_stage_bytecode(ShaderStage::COMPUTE, &bytecode);
        let shader = rd.shader_create_from_spirv(&spirv);
        if !shader.is_valid() {
            return Err("Failed to create D3D12 BGRA->RGBA publish compute shader".to_string());
        }
        let pipeline = rd.compute_pipeline_create(shader);
        if !pipeline.is_valid() {
            rd.free_rid(shader);
            return Err("Failed to create D3D12 BGRA->RGBA publish compute pipeline".to_string());
        }
        let sampler = rd.sampler_create(&RdSamplerState::new_gd());
        if !sampler.is_valid() {
            rd.free_rid(pipeline);
            rd.free_rid(shader);
            return Err("Failed to create D3D12 BGRA->RGBA publish sampler".to_string());
        }
        Ok(Self {
            shader,
            pipeline,
            sampler,
        })
    }

    fn create_uniform_set(
        &self,
        rd: &mut RenderingDevice,
        staging: Rid,
        display: Rid,
    ) -> Result<Rid, String> {
        let mut src_uniform = RdUniform::new_gd();
        src_uniform.set_uniform_type(UniformType::SAMPLER_WITH_TEXTURE);
        src_uniform.set_binding(0);
        src_uniform.add_id(self.sampler);
        src_uniform.add_id(staging);

        let mut dst_uniform = RdUniform::new_gd();
        dst_uniform.set_uniform_type(UniformType::IMAGE);
        dst_uniform.set_binding(1);
        dst_uniform.add_id(display);

        let mut uniforms = Array::new();
        uniforms.push(&src_uniform);
        uniforms.push(&dst_uniform);

        let uniform_set = rd.uniform_set_create(&uniforms, self.shader, 0);
        if uniform_set.is_valid() {
            Ok(uniform_set)
        } else {
            Err("Failed to create D3D12 BGRA->RGBA publish uniform set".to_string())
        }
    }

    fn free(self, rd: &mut RenderingDevice) {
        if self.pipeline.is_valid() {
            rd.free_rid(self.pipeline);
        }
        if self.shader.is_valid() {
            rd.free_rid(self.shader);
        }
        if self.sampler.is_valid() {
            rd.free_rid(self.sampler);
        }
    }
}

pub struct Publication {
    pool: SnapshotPool<SlotTextures>,
    display: Rid,
    width: u32,
    height: u32,
    format: SnapshotFormat,
    #[cfg(target_os = "windows")]
    d3d12_blit: Option<D3D12BgraBlit>,
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
        d3d12_bgra: bool,
    ) -> Result<SharedPublication, String> {
        if !display.is_valid() || width == 0 || height == 0 {
            return Err("Invalid snapshot display or dimensions".to_string());
        }
        #[cfg(not(target_os = "windows"))]
        let _ = d3d12_bgra;
        let mut rd = rendering_device()?;
        #[cfg(target_os = "windows")]
        let d3d12_blit = if d3d12_bgra {
            Some(D3D12BgraBlit::create(&mut rd)?)
        } else {
            None
        };
        let mut resources = Vec::with_capacity(SNAPSHOT_SLOTS);
        for _ in 0..SNAPSHOT_SLOTS {
            #[cfg(target_os = "windows")]
            let created = create_slot(&mut rd, width, height, format, display, d3d12_blit);
            #[cfg(not(target_os = "windows"))]
            let created = create_slot(&mut rd, width, height, format);
            match created {
                Ok(slot) => resources.push(slot),
                Err(error) => {
                    for slot in resources {
                        free_slot(&mut rd, slot);
                    }
                    #[cfg(target_os = "windows")]
                    if let Some(blit) = d3d12_blit {
                        blit.free(&mut rd);
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
            #[cfg(target_os = "windows")]
            d3d12_blit,
        }));
        for (token, slot) in tokens.into_iter().zip(resources) {
            // The initial upload establishes an initialized Godot resource. The
            // copy leaves staging in COPY_SOURCE/TRANSFER_SRC and ties readback
            // completion to the bootstrap's last staging access.
            let transparent =
                PackedByteArray::from(vec![0; width as usize * height as usize * 4].as_slice());
            #[cfg(target_os = "windows")]
            let bootstrap_sentinel = if slot.staging_sentinel.is_valid() {
                slot.staging_sentinel
            } else {
                slot.sentinel
            };
            #[cfg(not(target_os = "windows"))]
            let bootstrap_sentinel = slot.sentinel;
            let result = check(
                rd.texture_update(slot.staging, 0, &transparent),
                "snapshot bootstrap initialization",
            )
            .and_then(|()| copy(&mut rd, slot.staging, bootstrap_sentinel, 1, 1))
            .and_then(|()| read_completion(&mut rd, &shared, token, bootstrap_sentinel, true));
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
        #[cfg(target_os = "windows")]
        let (token, slot, display, width, height, d3d12_blit) = {
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
                publication.d3d12_blit,
            )
        };
        #[cfg(not(target_os = "windows"))]
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
        #[cfg(target_os = "windows")]
        let transfer_result = if let Some(blit) = d3d12_blit {
            let list = rd.compute_list_begin();
            rd.compute_list_bind_compute_pipeline(list, blit.pipeline);
            rd.compute_list_bind_uniform_set(list, slot.uniform_set, 0);
            rd.compute_list_dispatch(list, width.div_ceil(8), height.div_ceil(8), 1);
            rd.compute_list_end();
            // Restore staging from SHADER_RESOURCE back to COPY_SOURCE for the
            // next native D3D12 capture before the frame fence completes.
            copy(&mut rd, slot.staging, slot.staging_sentinel, 1, 1)
        } else {
            copy(&mut rd, slot.staging, display, width, height)
        };
        #[cfg(not(target_os = "windows"))]
        let transfer_result = copy(&mut rd, slot.staging, display, width, height);
        let result = transfer_result
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

fn create_texture_with_format(
    rd: &mut RenderingDevice,
    width: u32,
    height: u32,
    data_format: DataFormat,
) -> Result<Rid, String> {
    let mut format = RdTextureFormat::new_gd();
    format.set_format(data_format);
    format.set_width(width);
    format.set_height(height);
    format.set_depth(1);
    format.set_array_layers(1);
    format.set_mipmaps(1);
    format.set_texture_type(TextureType::TYPE_2D);
    format.set_samples(TextureSamples::SAMPLES_1);
    // Bootstrap uses an initial zero upload, avoiding sRGB RTV/UAV clears.
    // Godot's texture_create always makes an image view, which Vulkan allows
    // only on an image with a view-compatible usage (VUID ...-image-04441).
    format.set_usage_bits(
        TextureUsageBits::SAMPLING_BIT
            | TextureUsageBits::CAN_COPY_FROM_BIT
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

fn create_texture(
    rd: &mut RenderingDevice,
    width: u32,
    height: u32,
    snapshot_format: SnapshotFormat,
) -> Result<Rid, String> {
    create_texture_with_format(rd, width, height, snapshot_format.rd_format())
}

#[cfg(target_os = "windows")]
fn create_slot(
    rd: &mut RenderingDevice,
    width: u32,
    height: u32,
    format: SnapshotFormat,
    display: Rid,
    d3d12_blit: Option<D3D12BgraBlit>,
) -> Result<SlotTextures, String> {
    let (staging, sentinel, staging_sentinel, uniform_set) = if let Some(blit) = d3d12_blit {
        let staging = create_texture_with_format(rd, width, height, DataFormat::B8G8R8A8_UNORM)?;
        let sentinel = match create_texture_with_format(rd, 1, 1, DataFormat::R8G8B8A8_UNORM) {
            Ok(rid) => rid,
            Err(error) => {
                rd.free_rid(staging);
                return Err(error);
            }
        };
        let staging_sentinel =
            match create_texture_with_format(rd, 1, 1, DataFormat::B8G8R8A8_UNORM) {
                Ok(rid) => rid,
                Err(error) => {
                    rd.free_rid(staging);
                    rd.free_rid(sentinel);
                    return Err(error);
                }
            };
        let uniform_set = match blit.create_uniform_set(rd, staging, display) {
            Ok(rid) => rid,
            Err(error) => {
                rd.free_rid(staging);
                rd.free_rid(sentinel);
                rd.free_rid(staging_sentinel);
                return Err(error);
            }
        };
        (staging, sentinel, staging_sentinel, uniform_set)
    } else {
        let staging = create_texture(rd, width, height, format)?;
        let sentinel = match create_texture(rd, 1, 1, format) {
            Ok(rid) => rid,
            Err(error) => {
                rd.free_rid(staging);
                return Err(error);
            }
        };
        (staging, sentinel, Rid::Invalid, Rid::Invalid)
    };
    // Godot 4.6 exposes each backend's native texture through TEXTURE. These
    // handles are borrowed; the RD RIDs keep their resources alive.
    // https://github.com/godotengine/godot/blob/89cea143987d564363e15d207438530651d943ac/drivers/d3d12/rendering_device_driver_d3d12.cpp#L5150-L5156
    let native_handle = rd.get_driver_resource(DriverResource::TEXTURE, staging, 0);
    if native_handle == 0 {
        free_slot(
            rd,
            SlotTextures {
                staging,
                sentinel,
                staging_sentinel,
                uniform_set,
                native_handle: 0,
            },
        );
        return Err("RenderingDevice returned a null snapshot native texture".to_string());
    }
    Ok(SlotTextures {
        staging,
        sentinel,
        staging_sentinel,
        uniform_set,
        native_handle,
    })
}

#[cfg(not(target_os = "windows"))]
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
    let mut pending = Some(PendingReadback::new());
    let callback = Callable::from_sync_fn("cef_snapshot_complete", move |args: &[&Variant]| {
        let valid = args
            .first()
            .and_then(|arg| arg.try_to::<PackedByteArray>().ok())
            .is_some_and(|bytes| bytes.len() == 4);
        // Godot currently invokes this after its frame fence on the rendering
        // thread. Route through its public thread API so cleanup retains that
        // contract even if callback dispatch changes in a later engine version.
        let completion = Arc::clone(&shared);
        let mut pending_completion = pending.take();
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
                // Keep the registration until retirement work itself completes.
                // The guard also accounts for rejected/dropped callbacks.
                drop(pending_completion.take());
            },
        ));
    });
    check(
        rd.texture_get_data_async(sentinel, 0, &callback),
        "snapshot readback",
    )
}

/// Finish pending retirement before Scene bindings are unloaded. Normal capture
/// and publication never invoke this blocking drain.
pub(crate) fn drain_pending_publications() -> Result<(), String> {
    // Software/headless runs and already-drained renderers need no RD calls.
    if PENDING_READBACKS.load(Ordering::Acquire) == 0 {
        return Ok(());
    }
    crate::render::on_render_thread_sync(|| {
        if PENDING_READBACKS.load(Ordering::Acquire) == 0 {
            return Ok(());
        }
        let mut rd = rendering_device()?;
        // A separate marker must outlive the synchronous read: the callbacks
        // drained inside it may free every sentinel belonging to a retired pool.
        let marker = create_texture(&mut rd, 1, 1, SnapshotFormat::Rgba8)?;
        check(
            rd.texture_update(marker, 0, &PackedByteArray::from(&[0u8; 4][..])),
            "shutdown marker initialization",
        )?;
        // In Godot 4.6 texture_get_data calls _flush_and_stall_for_all_frames:
        // it waits submitted frames, submits the current graph, then waits that
        // frame. _stall_for_frame invokes the pending asynchronous callbacks
        // only after the frame fence, while our bindings are still available.
        // https://github.com/godotengine/godot/blob/89cea143987d564363e15d207438530651d943ac/servers/rendering/rendering_device.cpp
        let marker_data = rd.texture_get_data(marker, 0);
        if marker_data.len() != 4 {
            // No completion proof: retain the marker and any faulted pool slots
            // until device teardown, as on ordinary readback failure.
            return Err("Snapshot shutdown drain did not complete its marker readback".into());
        }
        rd.free_rid(marker);
        let pending = PENDING_READBACKS.load(Ordering::Acquire);
        if pending != 0 {
            return Err(format!(
                "Snapshot shutdown drain left {pending} callbacks pending"
            ));
        }
        Ok(())
    })?
}

fn quarantine(shared: &SharedPublication, token: SnapshotToken, error: &str) {
    if let Ok(mut publication) = shared.lock() {
        publication.pool.fail(token);
    }
    godot_error!("[AcceleratedOSR] {error}; snapshot slot quarantined");
}

fn free_slot(rd: &mut RenderingDevice, slot: SlotTextures) {
    #[cfg(target_os = "windows")]
    if slot.uniform_set.is_valid() && rd.uniform_set_is_valid(slot.uniform_set) {
        rd.free_rid(slot.uniform_set);
    }
    rd.free_rid(slot.staging);
    rd.free_rid(slot.sentinel);
    #[cfg(target_os = "windows")]
    if slot.staging_sentinel.is_valid() {
        rd.free_rid(slot.staging_sentinel);
    }
}

fn collect_retired(shared: &SharedPublication) {
    let Ok(mut rd) = rendering_device() else {
        return;
    };
    #[cfg(target_os = "windows")]
    let (slots, display, d3d12_blit) = {
        let Ok(mut publication) = shared.lock() else {
            return;
        };
        let slots = publication.pool.take_retired();
        let (display, d3d12_blit) = if publication.pool.is_drained() {
            (
                std::mem::replace(&mut publication.display, Rid::Invalid),
                publication.d3d12_blit.take(),
            )
        } else {
            (Rid::Invalid, None)
        };
        (slots, display, d3d12_blit)
    };
    #[cfg(not(target_os = "windows"))]
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
    #[cfg(target_os = "windows")]
    if let Some(blit) = d3d12_blit {
        blit.free(&mut rd);
    }
}

// Deliberately no unconditional Drop/free_rid: failed native work/readback has no
// safe completion proof. Such faulted RIDs remain allocated until device teardown
// instead of turning an error path into GPU use-after-free.
