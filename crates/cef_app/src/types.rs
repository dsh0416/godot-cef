#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PhysicalSize<T> {
    pub width: T,
    pub height: T,
}

impl<T> PhysicalSize<T> {
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

#[derive(Default)]
pub struct FrameBuffer {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub dirty: bool,
}

impl FrameBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the buffer with new RGBA pixel data
    pub fn update(&mut self, data: Vec<u8>, width: u32, height: u32) {
        self.data = data;
        self.width = width;
        self.height = height;
        self.dirty = true;
    }

    /// Mark the buffer as consumed (not dirty)
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }
}

#[derive(Default, Clone)]
pub struct PopupState {
    pub visible: bool,
    pub rect: PopupRect,
    pub buffer: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub dirty: bool,
}

#[derive(Default, Clone, Copy)]
pub struct PopupRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PopupState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            self.buffer.clear();
            self.width = 0;
            self.height = 0;
        }
        self.dirty = true;
    }

    pub fn set_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.rect = PopupRect {
            x,
            y,
            width,
            height,
        };
        self.dirty = true;
    }

    pub fn update_buffer(&mut self, data: Vec<u8>, width: u32, height: u32) {
        self.buffer = data;
        self.width = width;
        self.height = height;
        self.dirty = true;
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorType {
    #[default]
    Arrow,
    IBeam,
    Hand,
    Cross,
    Wait,
    Help,
    Move,
    ResizeNS,
    ResizeEW,
    ResizeNESW,
    ResizeNWSE,
    NotAllowed,
    Progress,
}
