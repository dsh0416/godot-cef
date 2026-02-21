use super::*;

impl CefTexture2D {
    pub(super) fn make_placeholder_texture(size: Vector2i) -> Gd<ImageTexture> {
        let width = size.x.max(1);
        let height = size.y.max(1);
        let pixel_count = (width as usize) * (height as usize);
        let bytes = vec![0u8; pixel_count * 4];
        let byte_array = PackedByteArray::from(bytes.as_slice());

        let mut texture = ImageTexture::new_gd();
        if let Some(image) =
            Image::create_from_data(width, height, false, ImageFormat::RGBA8, &byte_array)
        {
            texture.set_image(&image);
        }

        texture
    }

    pub(super) fn update_texture(&mut self) {
        let Some(state) = &mut self.runtime.app_mut().state else {
            return;
        };

        let software_had_dirty = match &state.render_mode {
            RenderMode::Software { frame_buffer, .. } => {
                let fb_dirty = frame_buffer.lock().ok().is_some_and(|fb| fb.dirty);
                let popup_dirty =
                    state.popup_state.lock().ok().is_some_and(|popup| {
                        popup.visible && popup.dirty && !popup.buffer.is_empty()
                    });
                fb_dirty || popup_dirty
            }
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            RenderMode::Accelerated { .. } => false,
        };

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        let had_pending_copy =
            if let RenderMode::Accelerated { render_state, .. } = &state.render_mode {
                render_state
                    .lock()
                    .ok()
                    .is_some_and(|rs| rs.has_pending_copy)
            } else {
                false
            };
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        let had_pending_copy = false;

        let replacement = self.runtime.update_primary_texture("CefTexture2D");
        let has_replacement = replacement.is_some();

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            if let Some(new_t2d) = replacement
                && let Some(ref mut stable) = self.stable_texture_2d_rd
            {
                let new_rd_rid = new_t2d.get_texture_rd_rid();
                stable.set_texture_rd_rid(new_rd_rid);
                if let Some(state) = self.runtime.app_mut().state.as_mut()
                    && let RenderMode::Accelerated { texture_2d_rd, .. } = &mut state.render_mode
                {
                    *texture_2d_rd = stable.clone();
                }
            }
        }

        let _ = replacement;
        let should_emit_changed = software_had_dirty || has_replacement || had_pending_copy;
        if should_emit_changed {
            self.base_mut().emit_changed();
        }
    }
}
