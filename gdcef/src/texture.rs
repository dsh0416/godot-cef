//! TextureRectRd - A Godot TextureRect for GPU-accelerated texture rendering.
//!
//! This module provides a specialized TextureRect node that manages Godot-owned
//! textures suitable for GPU-to-GPU copying from external sources like CEF.

use godot::classes::image::Format as ImageFormat;
use godot::classes::texture_rect::ExpandMode;
use godot::classes::{ITextureRect, Image, ImageTexture, RenderingServer, TextureRect};
use godot::prelude::*;

/// A TextureRect that creates and manages a Godot-owned texture suitable for
/// GPU-to-GPU copying from external sources (like CEF shared textures).
///
/// This node creates an ImageTexture with a placeholder Image that has the correct
/// usage flags for Godot's rendering pipeline. The native handle of this texture
/// can be obtained via RenderingDevice::get_driver_resource() for direct GPU copying.
#[derive(GodotClass)]
#[class(base=TextureRect, internal)]
pub struct TextureRectRd {
    base: Base<TextureRect>,
    texture: Option<Gd<ImageTexture>>,
    width: u32,
    height: u32,
}

#[godot_api]
impl ITextureRect for TextureRectRd {
    fn init(base: Base<TextureRect>) -> Self {
        Self {
            base,
            texture: None,
            width: 0,
            height: 0,
        }
    }

    fn ready(&mut self) {
        self.base_mut().set_expand_mode(ExpandMode::IGNORE_SIZE);
    }
}

#[godot_api]
impl TextureRectRd {
    /// Creates or resizes the internal texture to the specified dimensions.
    /// Returns the RID of the texture for use with RenderingServer operations.
    #[func]
    pub fn ensure_texture_size(&mut self, width: i32, height: i32) -> Rid {
        let width = width.max(1) as u32;
        let height = height.max(1) as u32;

        if self.width == width
            && self.height == height
            && let Some(ref texture) = self.texture
        {
            return texture.get_rid();
        }

        self.width = width;
        self.height = height;

        let image = Image::create(width as i32, height as i32, false, ImageFormat::RGBA8);

        if let Some(image) = image {
            let mut texture = ImageTexture::new_gd();
            texture.set_image(&image);
            let rid = texture.get_rid();
            self.base_mut().set_texture(&texture);
            self.texture = Some(texture);

            rid
        } else {
            godot::global::godot_error!(
                "[TextureRectRd] Failed to create placeholder image {}x{}",
                width,
                height
            );
            Rid::Invalid
        }
    }

    /// Returns the RID of the internal texture, or Invalid if no texture exists.
    #[func]
    pub fn get_texture_rid(&self) -> Rid {
        self.texture
            .as_ref()
            .map(|t| t.get_rid())
            .unwrap_or(Rid::Invalid)
    }

    /// Returns the RenderingDevice RID for the texture, which can be used with
    /// get_driver_resource() to obtain the native handle.
    #[func]
    pub fn get_rd_texture_rid(&self) -> Rid {
        let texture_rid = self.get_texture_rid();
        if !texture_rid.is_valid() {
            return Rid::Invalid;
        }

        let rs = RenderingServer::singleton();
        rs.texture_get_rd_texture(texture_rid)
    }

    /// Returns the current texture width.
    #[func]
    pub fn get_texture_width(&self) -> i32 {
        self.width as i32
    }

    /// Returns the current texture height.
    #[func]
    pub fn get_texture_height(&self) -> i32 {
        self.height as i32
    }
}
