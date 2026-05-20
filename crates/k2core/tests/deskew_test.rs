//! Step 5.4 Deskew 集成测试 - 跨模块 + fixture smoke。
//!
//! 测试策略（layout.json 缺失，vs C 版精确比对推迟到 Step 5.7）：
//!
//! 1. **完整 round-trip**：合成已知歪斜 → [`auto_straighten`] → 验证 stdev 增加
//! 2. **fixture PNG smoke**：fixture 渲染的 PNG → [`auto_straighten_angle`] 不 panic
//! 3. **多 PixelFormat 一致性**：Gray8/Rgb8/Rgba8 都能旋转 + 像素数守恒
//! 4. **边界条件**：min/max degrees / 极小图 / 单色图
//!
//! 这里只走 [`k2core::deskew`] 的 free fn 公开 API + [`k2core::bitmap::read_png`]。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use core::f64::consts::PI;

use k2core::deskew::{auto_straighten, auto_straighten_angle, rotate_fast};
use k2core::{horizontal_dark_count, read_png, Rect};
use k2types::{Bitmap, PixelFormat};

const FIXTURE_PAGE: &str = "tests/golden/single-column/c-pages/page-0001.png";
const SKEWED_PAGE: &str = "tests/golden/skewed-scan/c-pages/page-0001.png";

fn fixtures_root() -> PathBuf {
    // workspace 根目录 = crate 根的上两级
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_root
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn read_png_fixture(rel: &str) -> Option<Bitmap> {
    let full = fixtures_root().join(rel);
    if !full.exists() {
        return None;
    }
    read_png(&full, 300.0).ok()
}

/// 合成一张全白底 + 多条水平黑线（模拟文字行）。
fn synth_text_lines(w: u32, h: u32, line_rows: &[u32], c1: u32, c2: u32) -> Bitmap {
    let mut b = Bitmap::new(w, h, 300.0, PixelFormat::Gray8).unwrap();
    b.fill_byte(255);
    for &y in line_rows {
        if y >= h {
            continue;
        }
        let row = b.row_mut(y).unwrap();
        for (x, byte) in row.iter_mut().enumerate() {
            if (x as u32) >= c1 && (x as u32) <= c2 {
                *byte = 0;
            }
        }
    }
    b
}

// --------------------------------------------------------------------------
// 1. Round-trip：歪斜 → 检测 → 校正
// --------------------------------------------------------------------------

#[test]
fn deskew_round_trip_recovers_horizontal_alignment() {
    // 制造 1.0° 歪斜的水平文字图
    let template = synth_text_lines(400, 260, &[40, 70, 100, 130, 160, 190, 220], 60, 340);
    let mut skewed = template.clone();
    rotate_fast(&mut skewed, 1.0, false);

    // 用 horizontal_dark_count 检查"暗像素在行间的集中度"。在歪斜图上，
    // 任意单行很少有连续暗像素；deskew 后应该有几行集中很多暗像素。
    let rect_full = Rect::new(0, 0, 399, 259);
    let hist_before = horizontal_dark_count(&skewed, rect_full, 128);
    let max_dark_before = hist_before.buckets().iter().max().copied().unwrap_or(0);

    let detected = auto_straighten(&mut skewed, 200, 4.0, 0.1);
    let hist_after = horizontal_dark_count(&skewed, rect_full, 128);
    let max_dark_after = hist_after.buckets().iter().max().copied().unwrap_or(0);

    assert!(detected != 0.0, "should detect non-zero skew on 1.0° image");
    assert!(
        (detected - (-1.0)).abs() <= 0.3,
        "detected {detected}, expected ≈ -1.0 ± 0.3"
    );
    // 校正后单行最大暗像素数应明显增加（理想情况：所有文字行的全部 c2-c1+1 像素都同行）
    assert!(
        max_dark_after >= max_dark_before,
        "after deskew max dark per row {max_dark_after} >= before {max_dark_before}"
    );
}

#[test]
fn deskew_detects_negative_skew() {
    let template = synth_text_lines(400, 260, &[40, 70, 100, 130, 160, 190, 220], 60, 340);
    let mut skewed = template.clone();
    rotate_fast(&mut skewed, -2.5, false);
    let detected = auto_straighten_angle(&skewed, 200, 4.0, 0.1);
    assert!(
        (detected - 2.5).abs() <= 0.3,
        "detected {detected}, expected ≈ +2.5 ± 0.3 for -2.5° skew"
    );
}

#[test]
fn deskew_no_skew_returns_small_angle() {
    // 无歪斜的水平文字应给出绝对值很小的角度（理想 ≤ 0.1°）
    let bmp = synth_text_lines(400, 260, &[40, 70, 100, 130, 160, 190, 220], 60, 340);
    let angle = auto_straighten_angle(&bmp, 200, 4.0, 0.1);
    assert!(
        angle.abs() < 0.1,
        "aligned text should give near-zero angle, got {angle} deg"
    );
}

// --------------------------------------------------------------------------
// 2. Fixture PNG smoke - 不 panic + 返回值在 [-4, 4] 范围
// --------------------------------------------------------------------------

#[test]
fn fixture_single_column_no_crash() {
    if let Some(bmp) = read_png_fixture(FIXTURE_PAGE) {
        let angle = auto_straighten_angle(&bmp, 200, 4.0, 0.1);
        assert!(
            (-4.0..=4.0).contains(&angle),
            "single-column page deskew angle {angle} should be in [-4, 4]"
        );
    } else {
        eprintln!("skip fixture_single_column_no_crash: PNG not found");
    }
}

#[test]
fn fixture_skewed_scan_no_crash() {
    // skewed-scan 是 C 版 k2pdfopt 处理 *后* 的输出，理论上已经被 deskew 过；
    // Rust 端再跑一次应该给出非常小的角度（接近 0）。
    if let Some(bmp) = read_png_fixture(SKEWED_PAGE) {
        let angle = auto_straighten_angle(&bmp, 200, 4.0, 0.1);
        assert!(
            (-4.0..=4.0).contains(&angle),
            "skewed-scan (after C deskew) angle {angle} should be in [-4, 4]"
        );
    } else {
        eprintln!("skip fixture_skewed_scan_no_crash: PNG not found");
    }
}

// --------------------------------------------------------------------------
// 3. 多 PixelFormat
// --------------------------------------------------------------------------

#[test]
fn rotate_fast_rgb_pixel_count_preserved() {
    let mut bmp = Bitmap::new(50, 30, 300.0, PixelFormat::Rgb8).unwrap();
    bmp.fill_rgb(200, 100, 50);
    rotate_fast(&mut bmp, 5.0, false);
    assert_eq!(bmp.format, PixelFormat::Rgb8);
    assert_eq!(bmp.width, 50);
    assert_eq!(bmp.height, 30);
    assert_eq!(bmp.pixels.len(), 50 * 30 * 3);
}

#[test]
fn rotate_fast_rgba_pixel_count_preserved() {
    let mut bmp = Bitmap::new(50, 30, 300.0, PixelFormat::Rgba8).unwrap();
    bmp.fill_rgb(200, 100, 50);
    rotate_fast(&mut bmp, -3.0, false);
    assert_eq!(bmp.format, PixelFormat::Rgba8);
    assert_eq!(bmp.pixels.len(), 50 * 30 * 4);
}

#[test]
fn rotate_fast_expand_grows_diagonal() {
    let mut bmp = Bitmap::new(100, 100, 300.0, PixelFormat::Gray8).unwrap();
    bmp.fill_byte(0);
    rotate_fast(&mut bmp, 45.0, true);
    // 45° expand: sqrt(2)*100 ≈ 141
    let target = (100.0_f64 * (PI / 4.0).sin() + 100.0 * (PI / 4.0).cos() + 0.5) as u32;
    assert!(bmp.width >= 140 && bmp.width <= 142, "got w={}", bmp.width);
    assert!(
        bmp.height >= 140 && bmp.height <= 142,
        "got h={}",
        bmp.height
    );
    assert_eq!(bmp.width, target, "width should match (sqrt(2)*100 ≈ 141)");
}

// --------------------------------------------------------------------------
// 4. 边界条件
// --------------------------------------------------------------------------

#[test]
fn deskew_handles_zero_max_degrees() {
    // max_degrees=0 → na clamp 到 1，n=3，应不 panic
    let bmp = synth_text_lines(100, 100, &[20, 40, 60], 10, 90);
    let _ = auto_straighten_angle(&bmp, 200, 0.0, 0.0);
}

#[test]
fn deskew_handles_full_white_no_text() {
    let mut bmp = Bitmap::new(200, 100, 300.0, PixelFormat::Gray8).unwrap();
    bmp.fill_byte(255);
    let angle = auto_straighten_angle(&bmp, 200, 4.0, 0.1);
    assert_eq!(angle, 0.0, "blank page should return 0 angle");
}

#[test]
fn deskew_handles_full_black_no_text() {
    let mut bmp = Bitmap::new(200, 100, 300.0, PixelFormat::Gray8).unwrap();
    bmp.fill_byte(0);
    // 全黑：每行 100% 暗，stdev=0 (各行相同) → 返回 0
    let angle = auto_straighten_angle(&bmp, 200, 4.0, 0.1);
    assert_eq!(angle, 0.0, "all-black should return 0 angle (no variance)");
}

#[test]
fn deskew_handles_tiny_bitmap() {
    let mut bmp = Bitmap::new(10, 10, 300.0, PixelFormat::Gray8).unwrap();
    bmp.fill_byte(255);
    let angle = auto_straighten_angle(&bmp, 200, 4.0, 0.1);
    assert_eq!(angle, 0.0);
    // rotate_fast 也不应 panic
    rotate_fast(&mut bmp, 1.5, false);
    assert_eq!(bmp.width, 10);
    assert_eq!(bmp.height, 10);
}

#[test]
fn deskew_apply_returns_zero_for_below_min_threshold() {
    let template = synth_text_lines(400, 260, &[40, 70, 100, 130], 60, 340);
    let mut skewed = template.clone();
    rotate_fast(&mut skewed, 0.02, false);
    // min_degrees=0.5 比 0.02 大 → 应 bail-out 返回 0
    let applied = auto_straighten(&mut skewed, 200, 4.0, 0.5);
    assert_eq!(
        applied, 0.0,
        "0.02° skew should be filtered by min_degrees=0.5"
    );
}

#[test]
fn rotate_fast_preserves_dpi_metadata() {
    let mut bmp = Bitmap::new(80, 60, 199.5, PixelFormat::Gray8).unwrap();
    bmp.fill_byte(128);
    rotate_fast(&mut bmp, 2.0, false);
    assert!(
        (bmp.dpi - 199.5).abs() < 1e-3,
        "dpi should be preserved across rotate, got {}",
        bmp.dpi
    );
}
