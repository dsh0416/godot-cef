//! GPU canvas composition for consumers of the standalone Texture2D resource.
//!
//! The public proxy stays stable while routing either to the native view or to
//! a transparent canvas viewport. Popup pixels never cross back to the CPU.

use godot::classes::rendering_server::{ViewportClearMode, ViewportUpdateMode};
use godot::classes::{CanvasItemMaterial, Engine, RenderingServer, SceneTree, Texture2Drd};
use godot::prelude::*;

pub(super) struct PopupCompositor {
    proxy: Rid,
    view_texture: Rid,
    popup_texture: Gd<Texture2Drd>,
    bound_popup: Rid,
    // Canvas items borrow this material's RID.
    material: Gd<CanvasItemMaterial>,
    canvas: Option<PopupCanvas>,
    compositing: bool,
}

struct PopupCanvas {
    viewport: Rid,
    canvas: Rid,
    item: Rid,
    layout: Option<(Vector2i, Rect2)>,
}

impl PopupCompositor {
    pub(super) fn new(view_texture: Rid) -> Self {
        let proxy = RenderingServer::singleton().texture_proxy_create(view_texture);
        // CanvasItemMaterial applies blend-mode changes during SceneTree's idle
        // flush. Create it with the browser, before any popup can be displayed:
        // creating it lazily in frame_pre_draw uses straight-alpha blending for
        // the first composed frame and multiplies CEF's premultiplied RGB twice.
        let mut material = CanvasItemMaterial::new_gd();
        material.set_blend_mode(godot::classes::canvas_item_material::BlendMode::PREMULT_ALPHA);
        Self {
            proxy,
            view_texture,
            popup_texture: Texture2Drd::new_gd(),
            bound_popup: Rid::Invalid,
            material,
            canvas: None,
            compositing: false,
        }
    }

    pub(super) fn rid(&self) -> Rid {
        self.proxy
    }

    /// Popup dimensions and position are in physical pixels, like the view's
    /// backing RD texture. The viewport clips popups to the texture's bounds.
    pub(super) fn update(&mut self, size: Vector2i, popup: Option<(Rid, Rect2)>) -> bool {
        let mut server = RenderingServer::singleton();
        let Some((popup_rid, rect)) = popup else {
            if self.compositing {
                server.texture_proxy_update(self.proxy, self.view_texture);
                if let Some(canvas) = &self.canvas {
                    server.viewport_set_active(canvas.viewport, false);
                    // Release the canvas's references before retiring a popup
                    // generation. The persistent wrapper will be rebound later.
                    server.canvas_item_clear(canvas.item);
                }
                self.popup_texture.set_texture_rd_rid(Rid::Invalid);
                self.bound_popup = Rid::Invalid;
                self.compositing = false;
                return true;
            }
            return false;
        };

        // Godot 4.6 leaves get_texture_rd_rid() unchanged when detaching an
        // invalid RID, although its RenderingServer texture has been freed.
        // Track our binding explicitly so reopening the same popup rebinds it.
        if self.bound_popup != popup_rid {
            self.popup_texture.set_texture_rd_rid(popup_rid);
            self.bound_popup = popup_rid;
        }
        let material = self.material.get_rid();
        let canvas = self
            .canvas
            .get_or_insert_with(|| PopupCanvas::new(&mut server, material));
        if canvas.layout != Some((size, rect)) || !self.compositing {
            server.viewport_set_size(canvas.viewport, size.x, size.y);
            server.canvas_item_clear(canvas.item);
            server.canvas_item_add_texture_rect(
                canvas.item,
                Rect2::new(Vector2::ZERO, size.to_vector2()),
                self.view_texture,
            );
            server.canvas_item_add_texture_rect(canvas.item, rect, self.popup_texture.get_rid());
            canvas.layout = Some((size, rect));
        }
        if !self.compositing {
            server.viewport_set_active(canvas.viewport, true);
            let output = server.viewport_get_texture(canvas.viewport);
            server.texture_proxy_update(self.proxy, output);
            self.compositing = true;
            return true;
        }
        false
    }

    /// Called while RenderingServer is live, before detaching the native view
    /// and queuing snapshot retirement. No deferred destructor owns these RIDs.
    pub(super) fn dispose(mut self) {
        let mut server = RenderingServer::singleton();
        server.free_rid(self.proxy);
        if let Some(canvas) = self.canvas.take() {
            if self.compositing {
                server.viewport_set_active(canvas.viewport, false);
            }
            server.free_rid(canvas.item);
            server.free_rid(canvas.canvas);
            server.free_rid(canvas.viewport);
        }
        self.popup_texture.set_texture_rd_rid(Rid::Invalid);
    }
}

impl PopupCanvas {
    fn new(server: &mut RenderingServer, material: Rid) -> Self {
        let viewport = server.viewport_create();
        server.viewport_set_disable_3d(viewport, true);
        server.viewport_set_transparent_background(viewport, true);
        server.viewport_set_clear_mode(viewport, ViewportClearMode::ALWAYS);
        server.viewport_set_update_mode(viewport, ViewportUpdateMode::ALWAYS);
        if let Some(tree) = Engine::singleton()
            .get_main_loop()
            .and_then(|main_loop| main_loop.try_cast::<SceneTree>().ok())
            && let Some(root) = tree.get_root()
        {
            // Render the offscreen composition before its root-window consumer.
            server.viewport_set_parent_viewport(viewport, root.get_viewport_rid());
        }
        let canvas = server.canvas_create();
        server.viewport_attach_canvas(viewport, canvas);
        let item = server.canvas_item_create();
        server.canvas_item_set_parent(item, canvas);
        server.canvas_item_set_material(item, material);
        Self {
            viewport,
            canvas,
            item,
            layout: None,
        }
    }
}
