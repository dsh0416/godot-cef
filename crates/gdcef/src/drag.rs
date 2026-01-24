use godot::prelude::*;

use crate::browser::DragDataInfo as InternalDragDataInfo;

#[derive(GodotClass)]
#[class(no_init)]
pub struct DragDataInfo {
    base: Base<RefCounted>,

    #[var]
    pub is_link: bool,

    #[var]
    pub is_file: bool,

    #[var]
    pub is_fragment: bool,

    #[var]
    pub link_url: GString,

    #[var]
    pub link_title: GString,

    #[var]
    pub fragment_text: GString,

    #[var]
    pub fragment_html: GString,

    #[var]
    pub file_names: Array<GString>,
}

#[godot_api]
impl DragDataInfo {
    #[func]
    pub fn create() -> Gd<Self> {
        Gd::from_init_fn(|base| Self {
            base,
            is_link: false,
            is_file: false,
            is_fragment: false,
            link_url: GString::new(),
            link_title: GString::new(),
            fragment_text: GString::new(),
            fragment_html: GString::new(),
            file_names: Array::new(),
        })
    }
}

impl DragDataInfo {
    pub(crate) fn from_internal(data: &InternalDragDataInfo) -> Gd<Self> {
        let file_names: Array<GString> = data
            .file_names
            .iter()
            .map(|s| GString::from(s.as_str()))
            .collect();

        Gd::from_init_fn(|base| Self {
            base,
            is_link: data.is_link,
            is_file: data.is_file,
            is_fragment: data.is_fragment,
            link_url: GString::from(&data.link_url),
            link_title: GString::from(&data.link_title),
            fragment_text: GString::from(&data.fragment_text),
            fragment_html: GString::from(&data.fragment_html),
            file_names,
        })
    }
}

#[derive(GodotClass)]
#[class(no_init)]
pub struct DragOperation {
    base: Base<RefCounted>,
}

#[godot_api]
impl DragOperation {
    #[constant]
    const NONE: i32 = 0;

    #[constant]
    const COPY: i32 = 1;

    #[constant]
    const LINK: i32 = 2;

    #[constant]
    const MOVE: i32 = 16;

    #[constant]
    const EVERY: i32 = i32::MAX;
}
