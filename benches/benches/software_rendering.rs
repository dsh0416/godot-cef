//! Benchmarks for software rendering buffer operations.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use software_render::{DestBuffer, PopupBuffer, composite_popup};
use std::hint::black_box;

/// Optimized: precompute strides outside the loop.
fn composite_popup_chunks(dst: &mut DestBuffer, popup: &PopupBuffer) {
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

    let copy_bytes = (visible_width * 4) as usize;
    let src_row_stride = (popup.width * 4) as usize;
    let dst_row_stride = (dst.width * 4) as usize;

    let src_start = ((skip_y * popup.width + skip_x) * 4) as usize;
    let dst_start = ((start_y * dst.width + start_x) * 4) as usize;

    for row in 0..visible_height as usize {
        let src_offset = src_start + row * src_row_stride;
        let dst_offset = dst_start + row * dst_row_stride;

        if src_offset + copy_bytes <= popup.data.len() && dst_offset + copy_bytes <= dst.data.len()
        {
            dst.data[dst_offset..dst_offset + copy_bytes]
                .copy_from_slice(&popup.data[src_offset..src_offset + copy_bytes]);
        }
    }
}

fn create_test_buffers(
    dst_width: u32,
    dst_height: u32,
    popup_width: u32,
    popup_height: u32,
) -> (Vec<u8>, Vec<u8>) {
    let dst_size = (dst_width * dst_height * 4) as usize;
    let popup_size = (popup_width * popup_height * 4) as usize;

    let dst_buffer: Vec<u8> = (0..dst_size).map(|i| (i % 256) as u8).collect();
    let popup_buffer: Vec<u8> = (0..popup_size).map(|i| ((i * 7) % 256) as u8).collect();

    (dst_buffer, popup_buffer)
}

fn bench_buffer_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("buffer_clone");

    // Common resolutions: 720p, 1080p, 1440p, 4K
    let resolutions = [
        (1280, 720, "720p"),
        (1920, 1080, "1080p"),
        (2560, 1440, "1440p"),
        (3840, 2160, "4K"),
    ];

    for (width, height, name) in resolutions {
        let buffer_size = (width * height * 4) as u64;
        group.throughput(Throughput::Bytes(buffer_size));

        let (buffer, _) = create_test_buffers(width, height, 0, 0);

        group.bench_with_input(BenchmarkId::new("clone", name), &buffer, |b, buffer| {
            b.iter(|| {
                let cloned = black_box(buffer.clone());
                black_box(cloned)
            })
        });
    }

    group.finish();
}

fn bench_composite_popup(c: &mut Criterion) {
    let mut group = c.benchmark_group("composite_popup");

    // Test with 1080p destination and various popup sizes
    let dst_width = 1920u32;
    let dst_height = 1080u32;

    // Typical popup sizes (dropdowns, menus, tooltips)
    let popup_sizes = [
        (200, 300, "small_dropdown"),
        (400, 500, "medium_menu"),
        (800, 600, "large_dialog"),
        (300, 40, "wide_autocomplete"),
    ];

    for (popup_width, popup_height, name) in popup_sizes {
        let popup_bytes = (popup_width * popup_height * 4) as u64;
        group.throughput(Throughput::Bytes(popup_bytes));

        let (mut dst_buffer, popup_buffer) =
            create_test_buffers(dst_width, dst_height, popup_width, popup_height);

        // Popup positioned in center
        let popup_x = ((dst_width - popup_width) / 2) as i32;
        let popup_y = ((dst_height - popup_height) / 2) as i32;

        group.bench_with_input(
            BenchmarkId::new("baseline", name),
            &popup_buffer,
            |b, popup_data| {
                b.iter(|| {
                    let mut dst = DestBuffer {
                        data: &mut dst_buffer,
                        width: dst_width,
                        height: dst_height,
                    };
                    let popup = PopupBuffer {
                        data: popup_data,
                        width: popup_width,
                        height: popup_height,
                        x: popup_x,
                        y: popup_y,
                    };
                    composite_popup(black_box(&mut dst), black_box(&popup));
                })
            },
        );

        let (mut dst_buffer2, _) =
            create_test_buffers(dst_width, dst_height, popup_width, popup_height);

        group.bench_with_input(
            BenchmarkId::new("chunks", name),
            &popup_buffer,
            |b, popup_data| {
                b.iter(|| {
                    let mut dst = DestBuffer {
                        data: &mut dst_buffer2,
                        width: dst_width,
                        height: dst_height,
                    };
                    let popup = PopupBuffer {
                        data: popup_data,
                        width: popup_width,
                        height: popup_height,
                        x: popup_x,
                        y: popup_y,
                    };
                    composite_popup_chunks(black_box(&mut dst), black_box(&popup));
                })
            },
        );
    }

    group.finish();
}

fn bench_composite_popup_edge_cases(c: &mut Criterion) {
    let mut group = c.benchmark_group("composite_popup_edge_cases");

    let dst_width = 1920u32;
    let dst_height = 1080u32;
    let popup_width = 400u32;
    let popup_height = 300u32;

    let (mut dst_buffer, popup_buffer) =
        create_test_buffers(dst_width, dst_height, popup_width, popup_height);

    // Popup partially off-screen (negative x, y)
    let test_cases = [
        (-100, -50, "partial_top_left"),
        (1700, 900, "partial_bottom_right"),
        (0, 0, "at_origin"),
        (760, 390, "centered"),
    ];

    for (popup_x, popup_y, name) in test_cases {
        group.bench_with_input(
            BenchmarkId::new("baseline", name),
            &popup_buffer,
            |b, popup_data| {
                b.iter(|| {
                    let mut dst = DestBuffer {
                        data: &mut dst_buffer,
                        width: dst_width,
                        height: dst_height,
                    };
                    let popup = PopupBuffer {
                        data: popup_data,
                        width: popup_width,
                        height: popup_height,
                        x: popup_x,
                        y: popup_y,
                    };
                    composite_popup(black_box(&mut dst), black_box(&popup));
                })
            },
        );
    }

    group.finish();
}

fn bench_full_update_cycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_update_cycle");

    let resolutions = [(1920, 1080, "1080p"), (2560, 1440, "1440p")];

    for (width, height, name) in resolutions {
        let buffer_size = (width * height * 4) as u64;
        group.throughput(Throughput::Bytes(buffer_size));

        let popup_width = 300u32;
        let popup_height = 200u32;
        let (frame_buffer, popup_buffer) =
            create_test_buffers(width, height, popup_width, popup_height);

        // Simulate the full update path: clone + composite
        group.bench_with_input(
            BenchmarkId::new("with_popup", name),
            &(&frame_buffer, &popup_buffer),
            |b, (fb, popup_data)| {
                b.iter(|| {
                    let mut composited = fb.to_vec();
                    let mut dst = DestBuffer {
                        data: &mut composited,
                        width,
                        height,
                    };
                    let popup = PopupBuffer {
                        data: popup_data,
                        width: popup_width,
                        height: popup_height,
                        x: 100,
                        y: 100,
                    };
                    composite_popup(&mut dst, &popup);

                    black_box(composited)
                })
            },
        );

        // Without popup (just clone)
        group.bench_with_input(
            BenchmarkId::new("without_popup", name),
            &frame_buffer,
            |b, fb| {
                b.iter(|| {
                    let cloned = fb.clone();
                    black_box(cloned)
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_buffer_clone,
    bench_composite_popup,
    bench_composite_popup_edge_cases,
    bench_full_update_cycle,
);

criterion_main!(benches);
