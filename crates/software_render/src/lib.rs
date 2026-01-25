pub struct DestBuffer<'a> {
    pub data: &'a mut [u8],
    pub width: u32,
    pub height: u32,
}

pub struct PopupBuffer<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
}

pub fn composite_popup(dst: &mut DestBuffer, popup: &PopupBuffer) {
    let start_x = popup.x.max(0) as u32;
    let start_y = popup.y.max(0) as u32;

    let skip_x = if popup.x < 0 { (-popup.x) as u32 } else { 0 };
    let skip_y = if popup.y < 0 { (-popup.y) as u32 } else { 0 };

    let visible_width = (popup.width.saturating_sub(skip_x)).min(dst.width.saturating_sub(start_x));
    let visible_height =
        (popup.height.saturating_sub(skip_y)).min(dst.height.saturating_sub(start_y));

    if visible_width == 0 || visible_height == 0 {
        return;
    }

    for row in 0..visible_height {
        let src_row = skip_y + row;
        let dst_row = start_y + row;

        let src_row_start = ((src_row * popup.width + skip_x) * 4) as usize;
        let dst_row_start = ((dst_row * dst.width + start_x) * 4) as usize;

        let copy_bytes = (visible_width * 4) as usize;

        if src_row_start + copy_bytes <= popup.data.len()
            && dst_row_start + copy_bytes <= dst.data.len()
        {
            dst.data[dst_row_start..dst_row_start + copy_bytes]
                .copy_from_slice(&popup.data[src_row_start..src_row_start + copy_bytes]);
        }
    }
}
