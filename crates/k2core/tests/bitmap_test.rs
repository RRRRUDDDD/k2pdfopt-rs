//! Step 5.1 集成测试：覆盖 `Bitmap` 算法层、`Rect` 几何、`Histogram` 投影、I/O
//! 三个模块的 cross-module 行为，并校验与 C 版字段语义的对齐。
//!
//! 单元测试已经在每个模块的 `#[cfg(test)] mod tests` 内覆盖；本文件做集成层
//! "组合测试"，证明 Bitmap + Rect + Histogram 三者能协同工作。

#![allow(clippy::unwrap_used)]

use k2core::{
    horizontal_dark_count, horizontal_projection, read_pam, read_png, vertical_dark_count,
    vertical_projection, write_pam, write_png, Bitmap, Histogram, PixelFormat, Rect,
};

/// 构造一张 8x4 Gray8 位图，画一条水平黑线在 y=2，其他全白
fn synthesize_text_line_bitmap() -> Bitmap {
    let mut b = Bitmap::new(8, 4, 100.0, PixelFormat::Gray8).unwrap();
    b.fill_byte(255);
    let row = b.row_mut(2).unwrap();
    for px in row.iter_mut() {
        *px = 0;
    }
    b
}

#[test]
fn integration_horizontal_projection_finds_dark_row() {
    let bmp = synthesize_text_line_bitmap();
    let rect = Rect::from_xywh(0, 0, bmp.width, bmp.height);
    let proj = horizontal_projection(&bmp, rect);
    assert_eq!(proj.len(), 4);
    // y=0,1,3 是白色：8 * 255 = 2040
    assert_eq!(proj.get(0), 8 * 255);
    assert_eq!(proj.get(1), 8 * 255);
    assert_eq!(proj.get(3), 8 * 255);
    // y=2 是黑色行：0
    assert_eq!(proj.get(2), 0);
    // argmax 应该是 y=0（第一个最大）
    let (peak, _) = proj.argmax().unwrap();
    assert_eq!(peak, 0);
}

#[test]
fn integration_dark_count_with_threshold() {
    let bmp = synthesize_text_line_bitmap();
    let rect = Rect::from_xywh(0, 0, bmp.width, bmp.height);
    // threshold=128: 暗 < 128 的像素
    let dark = horizontal_dark_count(&bmp, rect, 128);
    assert_eq!(dark.get(0), 0); // 全白
    assert_eq!(dark.get(2), 8); // 全黑 8 个像素
}

#[test]
fn integration_rect_clip_to_bitmap() {
    let bmp = synthesize_text_line_bitmap();
    // 故意构造超出边界的矩形：(-2, -2, 100, 100)
    let big = Rect::new(-2, -2, 100, 100);
    let proj = horizontal_projection(&bmp, big);
    // 应当被钳制为 (0,0,7,3)，长度 4
    assert_eq!(proj.len(), 4);
    assert_eq!(proj.get(2), 0); // y=2 仍然全黑
}

#[test]
fn integration_vertical_projection_finds_dark_column() {
    // 4x6 Gray8，col=1 全黑，其余全白
    let mut bmp = Bitmap::new(4, 6, 100.0, PixelFormat::Gray8).unwrap();
    bmp.fill_byte(255);
    for y in 0..6u32 {
        bmp.pixel_mut(1, y).unwrap()[0] = 0;
    }
    let rect = Rect::from_xywh(0, 0, 4, 6);
    let proj = vertical_projection(&bmp, rect);
    assert_eq!(proj.len(), 4);
    assert_eq!(proj.get(0), 6 * 255);
    assert_eq!(proj.get(1), 0);
    assert_eq!(proj.get(2), 6 * 255);
    assert_eq!(proj.get(3), 6 * 255);

    let dark = vertical_dark_count(&bmp, rect, 128);
    assert_eq!(dark.get(1), 6);
    assert_eq!(dark.get(0), 0);
}

#[test]
fn integration_bitmap_grayscale_after_fill_rgb_neutral() {
    let mut b = Bitmap::new(4, 4, 100.0, PixelFormat::Rgb8).unwrap();
    b.fill_rgb(120, 120, 120);
    assert!(b.is_grayscale());
    // 改一个像素为彩色 → 不再是灰度
    b.pixel_mut(0, 0).unwrap().copy_from_slice(&[255, 0, 0]);
    assert!(!b.is_grayscale());
}

#[test]
fn integration_png_roundtrip_preserves_pixels_exact() {
    let tmp = std::env::temp_dir().join("k2core_int_png.png");
    let mut orig = Bitmap::new(7, 5, 96.0, PixelFormat::Rgb8).unwrap();
    for y in 0..5u32 {
        for x in 0..7u32 {
            let px = orig.pixel_mut(x, y).unwrap();
            px[0] = (x * 30) as u8;
            px[1] = (y * 50) as u8;
            px[2] = ((x + y) * 10) as u8;
        }
    }
    write_png(&orig, &tmp).unwrap();
    let loaded = read_png(&tmp, 96.0).unwrap();
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(loaded.width, orig.width);
    assert_eq!(loaded.height, orig.height);
    assert_eq!(loaded.format, orig.format);
    assert_eq!(loaded.pixels, orig.pixels);
}

#[test]
fn integration_pam_roundtrip_preserves_pixels_exact() {
    let mut orig = Bitmap::new(4, 3, 200.0, PixelFormat::Rgba8).unwrap();
    for y in 0..3u32 {
        for x in 0..4u32 {
            let px = orig.pixel_mut(x, y).unwrap();
            px[0] = (x * 60) as u8;
            px[1] = (y * 80) as u8;
            px[2] = 100;
            px[3] = 255;
        }
    }
    let mut buf = Vec::new();
    write_pam(&orig, &mut buf).unwrap();
    let loaded = read_pam(&buf, 200.0).unwrap();
    assert_eq!(loaded.width, 4);
    assert_eq!(loaded.height, 3);
    assert_eq!(loaded.format, PixelFormat::Rgba8);
    assert_eq!(loaded.pixels, orig.pixels);
}

#[test]
fn integration_rect_operations_chained() {
    let big = Rect::new(0, 0, 99, 99);
    let inner = Rect::from_xywh(10, 10, 20, 20);
    assert!(big.contains_rect(inner));
    assert_eq!(inner.area(), 400);

    let translated = inner.translate(5, 5);
    assert_eq!(translated, Rect::from_xywh(15, 15, 20, 20));

    let outside = Rect::from_xywh(150, 150, 10, 10);
    assert!(!big.intersects(outside));
    assert!(outside.clamp_to(big).is_empty());

    let half_overlap = Rect::new(90, 90, 110, 110);
    let inter = big.intersection(half_overlap);
    assert_eq!(inter, Rect::new(90, 90, 99, 99));
    assert_eq!(inter.area(), 100);
}

#[test]
fn integration_histogram_stats_after_projection() {
    let bmp = synthesize_text_line_bitmap();
    let proj = horizontal_projection(&bmp, Rect::from_xywh(0, 0, bmp.width, bmp.height));
    // sum = 2040 * 3 + 0 = 6120
    assert_eq!(proj.sum(), 6120);
    assert_eq!(proj.max(), 2040);
    assert_eq!(proj.min(), 0);
    // mean = 6120/4 = 1530
    assert_eq!(proj.mean(), 1530);
}

#[test]
fn integration_empty_histogram_for_disjoint_rect() {
    let bmp = synthesize_text_line_bitmap();
    let disjoint = Rect::new(100, 100, 200, 200);
    let proj = horizontal_projection(&bmp, disjoint);
    assert!(proj.is_empty());
    assert_eq!(proj.argmax(), None);
}

#[test]
fn integration_bitmap_field_layout_matches_c_semantics() {
    // 与 C 版 WILLUSBITMAP (willus.h:488-500) 字段语义对照：
    //   C int width  ↔ Rust u32 width
    //   C int height ↔ Rust u32 height
    //   C int bpp = {8, 24} ↔ Rust PixelFormat = {Gray8, Rgb8(+Rgba8 扩展)}
    //   C bmp_rowptr_from_top(bmp, y) (native, top-to-bottom)
    //     ↔ Rust Bitmap::row(y) (top-to-bottom，y=0 是顶行)
    //   C bmp_bytewidth(bmp) = bpp==24 ? width*3 : width
    //     ↔ Rust Bitmap::bytes_per_row() = width * bytes_per_pixel
    let g = Bitmap::new(10, 5, 96.0, PixelFormat::Gray8).unwrap();
    assert_eq!(g.bytes_per_row(), 10); // 等价于 C bpp=8: width
    let r = Bitmap::new(10, 5, 96.0, PixelFormat::Rgb8).unwrap();
    assert_eq!(r.bytes_per_row(), 30); // 等价于 C bpp=24: width*3
    let a = Bitmap::new(10, 5, 96.0, PixelFormat::Rgba8).unwrap();
    assert_eq!(a.bytes_per_row(), 40); // Rust 扩展：RGBA = width*4

    // 行索引 top-to-bottom（top=0 是顶行）
    let mut b = Bitmap::new(2, 2, 96.0, PixelFormat::Gray8).unwrap();
    b.row_mut(0).unwrap().copy_from_slice(&[100, 101]);
    b.row_mut(1).unwrap().copy_from_slice(&[200, 201]);
    // 顶行（y=0）应当被先存储
    assert_eq!(b.pixels[0], 100);
    assert_eq!(b.pixels[1], 101);
    assert_eq!(b.pixels[2], 200);
    assert_eq!(b.pixels[3], 201);
}

#[test]
fn integration_histogram_via_constructor() {
    // 验证 Histogram::new + buckets_mut 链式赋值
    let mut h = Histogram::new(4);
    let buckets = h.buckets_mut();
    buckets[0] = 100;
    buckets[1] = 200;
    buckets[2] = 50;
    buckets[3] = 200; // 与 buckets[1] 相同
    let (peak, val) = h.argmax().unwrap();
    assert_eq!(peak, 1); // 第一个最大值索引
    assert_eq!(val, 200);
    assert_eq!(h.sum(), 550);
}
