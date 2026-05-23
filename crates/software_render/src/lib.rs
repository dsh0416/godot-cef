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

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(value: u8) -> [u8; 4] {
        [value, value, value, 255]
    }

    fn write_pixel(buffer: &mut [u8], width: u32, x: u32, y: u32, value: [u8; 4]) {
        let index = ((y * width + x) * 4) as usize;
        buffer[index..index + 4].copy_from_slice(&value);
    }

    fn read_pixel(buffer: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let index = ((y * width + x) * 4) as usize;
        [
            buffer[index],
            buffer[index + 1],
            buffer[index + 2],
            buffer[index + 3],
        ]
    }

    #[test]
    fn composites_popup_inside_destination() {
        let mut dst_data = vec![0; 4 * 4 * 4];
        let mut popup_data = vec![0; 2 * 2 * 4];
        write_pixel(&mut popup_data, 2, 0, 0, pixel(1));
        write_pixel(&mut popup_data, 2, 1, 0, pixel(2));
        write_pixel(&mut popup_data, 2, 0, 1, pixel(3));
        write_pixel(&mut popup_data, 2, 1, 1, pixel(4));

        composite_popup(
            &mut DestBuffer {
                data: &mut dst_data,
                width: 4,
                height: 4,
            },
            &PopupBuffer {
                data: &popup_data,
                width: 2,
                height: 2,
                x: 1,
                y: 1,
            },
        );

        assert_eq!(read_pixel(&dst_data, 4, 1, 1), pixel(1));
        assert_eq!(read_pixel(&dst_data, 4, 2, 1), pixel(2));
        assert_eq!(read_pixel(&dst_data, 4, 1, 2), pixel(3));
        assert_eq!(read_pixel(&dst_data, 4, 2, 2), pixel(4));
        assert_eq!(read_pixel(&dst_data, 4, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn clips_negative_popup_offsets() {
        let mut dst_data = vec![0; 2 * 2 * 4];
        let mut popup_data = vec![0; 3 * 3 * 4];
        for y in 0..3 {
            for x in 0..3 {
                write_pixel(&mut popup_data, 3, x, y, pixel((y * 3 + x) as u8));
            }
        }

        composite_popup(
            &mut DestBuffer {
                data: &mut dst_data,
                width: 2,
                height: 2,
            },
            &PopupBuffer {
                data: &popup_data,
                width: 3,
                height: 3,
                x: -1,
                y: -1,
            },
        );

        assert_eq!(read_pixel(&dst_data, 2, 0, 0), pixel(4));
        assert_eq!(read_pixel(&dst_data, 2, 1, 0), pixel(5));
        assert_eq!(read_pixel(&dst_data, 2, 0, 1), pixel(7));
        assert_eq!(read_pixel(&dst_data, 2, 1, 1), pixel(8));
    }

    #[test]
    fn clips_popup_at_destination_edge() {
        let mut dst_data = vec![0; 2 * 2 * 4];
        let popup_data = vec![9; 2 * 2 * 4];

        composite_popup(
            &mut DestBuffer {
                data: &mut dst_data,
                width: 2,
                height: 2,
            },
            &PopupBuffer {
                data: &popup_data,
                width: 2,
                height: 2,
                x: 1,
                y: 1,
            },
        );

        assert_eq!(read_pixel(&dst_data, 2, 1, 1), [9, 9, 9, 9]);
        assert_eq!(read_pixel(&dst_data, 2, 0, 0), [0, 0, 0, 0]);
        assert_eq!(read_pixel(&dst_data, 2, 1, 0), [0, 0, 0, 0]);
        assert_eq!(read_pixel(&dst_data, 2, 0, 1), [0, 0, 0, 0]);
    }

    #[test]
    fn leaves_destination_unchanged_when_popup_is_not_visible() {
        let mut dst_data = vec![7; 2 * 2 * 4];
        let original = dst_data.clone();
        let popup_data = vec![9; 2 * 2 * 4];

        composite_popup(
            &mut DestBuffer {
                data: &mut dst_data,
                width: 2,
                height: 2,
            },
            &PopupBuffer {
                data: &popup_data,
                width: 2,
                height: 2,
                x: 2,
                y: 0,
            },
        );

        assert_eq!(dst_data, original);
    }

    #[test]
    fn skips_rows_that_exceed_buffer_lengths() {
        let mut dst_data = vec![0; 2 * 2 * 4 - 1];
        let popup_data = vec![9; 2 * 2 * 4 - 1];

        composite_popup(
            &mut DestBuffer {
                data: &mut dst_data,
                width: 2,
                height: 2,
            },
            &PopupBuffer {
                data: &popup_data,
                width: 2,
                height: 2,
                x: 0,
                y: 0,
            },
        );

        assert_eq!(&dst_data[..8], &[9; 8]);
        assert_eq!(&dst_data[8..], &[0; 7]);
    }
}
