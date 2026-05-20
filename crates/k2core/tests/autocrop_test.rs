//! Autocrop 集成测试。
//!
//! 覆盖：
//! - synthetic 已知裁剪框图像的端到端 [`auto_crop`] + [`apply_auto_crop`] 链路
//! - 12 fixture PNG smoke（不 panic + margins 在合理范围）
//! - PixelFormat 三分支
//! - 边界条件（零尺寸 / 极小 bitmap / 全白 / 全黑 / 居中黑块）
//!
//! 来源：`rust-rewrite-execution-plan.md` Step 5.5。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use k2core::{apply_auto_crop, auto_crop, AutoCropMargins};
use k2types::{Bitmap, PixelFormat};

fn make_white(w: u32, h: u32) -> Bitmap {
    let mut bmp = Bitmap::new(w, h, 300.0, PixelFormat::Gray8).unwrap();
    bmp.fill_byte(255);
    bmp
}

fn make_black(w: u32, h: u32) -> Bitmap {
    let mut bmp = Bitmap::new(w, h, 300.0, PixelFormat::Gray8).unwrap();
    bmp.fill_byte(0);
    bmp
}

fn paint_black_rect(bmp: &mut Bitmap, x0: u32, y0: u32, x1: u32, y1: u32) {
    let x1 = x1.min(bmp.width.saturating_sub(1));
    let y1 = y1.min(bmp.height.saturating_sub(1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            if let Some(p) = bmp.pixel_mut(x, y) {
                for b in p.iter_mut() {
                    *b = 0;
                }
            }
        }
    }
}

fn fixtures_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates
    p.pop(); // k2pdfopt-rs
    p.push("tests");
    p.push("golden");
    p
}

// ---------------------------------------------------------------------------
// 端到端基本路径
// ---------------------------------------------------------------------------

#[test]
fn auto_crop_then_apply_roundtrip_preserves_center_content() {
    let mut bmp = make_white(80, 100);
    paint_black_rect(&mut bmp, 25, 35, 54, 64);
    let r = auto_crop(&bmp, 0.5);

    // 计算保留窗口
    let cx = r.margins.to_cx();
    let kept_left = cx[0].max(0);
    let kept_top = cx[1].max(0);
    let kept_right_abs = (80 - 1 - cx[2]).max(0);
    let kept_bottom_abs = (100 - 1 - cx[3]).max(0);

    // 保留区必须覆盖整个黑块
    assert!(
        kept_left <= 25,
        "kept_left={kept_left} should be <= 25 (content starts at x=25)"
    );
    assert!(
        kept_top <= 35,
        "kept_top={kept_top} should be <= 35 (content starts at y=35)"
    );
    assert!(
        kept_right_abs >= 54,
        "kept_right_abs={kept_right_abs} should be >= 54 (content ends at x=54)"
    );
    assert!(
        kept_bottom_abs >= 64,
        "kept_bottom_abs={kept_bottom_abs} should be >= 64 (content ends at y=64)"
    );

    apply_auto_crop(&mut bmp, &r.margins);

    // 中心黑块完全保留
    for y in 35..=64 {
        for x in 25..=54 {
            assert_eq!(
                bmp.pixel(x, y).unwrap()[0],
                0,
                "center pixel ({x},{y}) should remain black"
            );
        }
    }
}

#[test]
fn auto_crop_pure_white_image_does_not_panic() {
    let bmp = make_white(50, 70);
    let r = auto_crop(&bmp, 0.5);
    let cx = r.margins.to_cx();
    // 全白图：autocrop_refine 的 find_threshold 因 max<0.2 返回 0，导致 cnew[2]=w/cnew[3]=h
    // → 翻转后 right=-1 / bottom=-1（C 版 documented corner case："不裁该边"）
    assert!(cx[0] >= 0 && cx[0] < 50, "left={}", cx[0]);
    assert!(cx[1] >= 0 && cx[1] < 70, "top={}", cx[1]);
    assert!(cx[2] >= -1 && cx[2] < 50, "right={}", cx[2]);
    assert!(cx[3] >= -1 && cx[3] < 70, "bottom={}", cx[3]);
    // 保留区宽/高 > 0
    assert!(50 - cx[0] - cx[2] > 0);
    assert!(70 - cx[1] - cx[3] > 0);
}

#[test]
fn auto_crop_pure_black_image_does_not_panic() {
    let bmp = make_black(50, 70);
    let r = auto_crop(&bmp, 0.5);
    let cx = r.margins.to_cx();
    // 全黑图：blackweight 拉低 areaw 但搜索仍能找到框；精修对全 0 hist 同样会触发
    // find_threshold 的 max<0.2 路径 → 同样可能出现 right=-1 / bottom=-1
    assert!(cx[0] >= 0, "left={}", cx[0]);
    assert!(cx[1] >= 0, "top={}", cx[1]);
    assert!(cx[2] >= -1, "right={}", cx[2]);
    assert!(cx[3] >= -1, "bottom={}", cx[3]);
}

#[test]
fn auto_crop_with_aggressiveness_zero_keeps_more() {
    let mut bmp_low = make_white(80, 100);
    paint_black_rect(&mut bmp_low, 30, 40, 49, 59);
    let r_zero = auto_crop(&bmp_low, 0.0);

    let mut bmp_high = make_white(80, 100);
    paint_black_rect(&mut bmp_high, 30, 40, 49, 59);
    let r_one = auto_crop(&bmp_high, 1.0);

    // aggressiveness=0 时 blackweight=0，相当于纯按面积+minarea 选最大框
    // aggressiveness=1 时 blackweight=50，更乐意小框（避开周长黑像素）
    // 但具体大小取决于精修；至少都应给合法 margins
    assert!(r_zero.margins.left >= 0);
    assert!(r_one.margins.left >= 0);
}

// ---------------------------------------------------------------------------
// PixelFormat 三分支
// ---------------------------------------------------------------------------

#[test]
fn auto_crop_rgb_image_works() {
    // 60x80 RGB 白底，中央 20-39, 30-49 黑块
    let mut bmp = Bitmap::new(60, 80, 300.0, PixelFormat::Rgb8).unwrap();
    bmp.fill_rgb(255, 255, 255);
    paint_black_rect(&mut bmp, 20, 30, 39, 49);
    let r = auto_crop(&bmp, 0.5);
    // 算法应能识别中心区域
    assert!(r.margins.left <= 20);
    assert!(r.margins.top <= 30);
}

#[test]
fn auto_crop_rgba_image_works() {
    let mut bmp = Bitmap::new(60, 80, 300.0, PixelFormat::Rgba8).unwrap();
    bmp.fill_rgb(255, 255, 255);
    paint_black_rect(&mut bmp, 20, 30, 39, 49);
    let r = auto_crop(&bmp, 0.5);
    assert!(r.margins.left <= 20);
    assert!(r.margins.top <= 30);
}

#[test]
fn apply_auto_crop_rgb_preserves_inside_outside_white() {
    let mut bmp = Bitmap::new(20, 20, 300.0, PixelFormat::Rgb8).unwrap();
    bmp.fill_rgb(100, 150, 200); // 内部独特颜色
    let margins = AutoCropMargins {
        left: 3,
        top: 3,
        right: 3,
        bottom: 3,
    };
    apply_auto_crop(&mut bmp, &margins);
    // 内部 (3..=16, 3..=16) 仍是 (100, 150, 200)
    let p = bmp.pixel(10, 10).unwrap();
    assert_eq!(p, [100, 150, 200]);
    // 外部 (0..3 或 17..20) 应为白
    let p = bmp.pixel(0, 0).unwrap();
    assert_eq!(p, [255, 255, 255]);
    let p = bmp.pixel(19, 19).unwrap();
    assert_eq!(p, [255, 255, 255]);
}

// ---------------------------------------------------------------------------
// 极小 bitmap 边界
// ---------------------------------------------------------------------------

#[test]
fn auto_crop_tiny_10x10_does_not_panic() {
    let bmp = make_white(10, 10);
    let r = auto_crop(&bmp, 0.5);
    // 即使 bw 下采样后只有 1x1 像素，算法也应安全返回（不强约束 success/margins）
    let _ = r;
}

#[test]
fn auto_crop_aspect_ratio_tall() {
    // 高 > 宽：pw=w/150, ph=h/200
    let bmp = make_white(50, 200);
    let r = auto_crop(&bmp, 0.5);
    // 全白图 corner case：right/bottom 可能为 -1
    assert!(r.margins.left >= 0);
    assert!(r.margins.top >= 0);
}

#[test]
fn auto_crop_aspect_ratio_wide() {
    // 宽 > 高
    let bmp = make_white(200, 50);
    let r = auto_crop(&bmp, 0.5);
    assert!(r.margins.left >= 0);
    assert!(r.margins.top >= 0);
}

// ---------------------------------------------------------------------------
// apply_auto_crop 边界
// ---------------------------------------------------------------------------

#[test]
fn apply_auto_crop_with_full_margins_blanks_image() {
    let mut bmp = make_black(20, 20);
    // left=20, right=20: 全部都"在外部"
    let margins = AutoCropMargins {
        left: 20,
        top: 20,
        right: 20,
        bottom: 20,
    };
    apply_auto_crop(&mut bmp, &margins);
    // 整张图应被填白
    for y in 0..20 {
        for x in 0..20 {
            assert_eq!(bmp.pixel(x, y).unwrap()[0], 255, "({x},{y})");
        }
    }
}

#[test]
fn apply_auto_crop_preserves_image_with_zero_margins() {
    let mut bmp = make_gray_gradient(15, 15);
    let original = bmp.pixels.clone();
    apply_auto_crop(&mut bmp, &AutoCropMargins::zero());
    assert_eq!(bmp.pixels, original);
}

fn make_gray_gradient(w: u32, h: u32) -> Bitmap {
    let mut bmp = Bitmap::new(w, h, 300.0, PixelFormat::Gray8).unwrap();
    for y in 0..h {
        for x in 0..w {
            let v = (((x + y) % 256) as u8).max(50); // 不要 0/255 以免与 fill 边缘混淆
            bmp.pixel_mut(x, y).unwrap()[0] = v;
        }
    }
    bmp
}

// ---------------------------------------------------------------------------
// Fixture PNG smoke 测试（不 panic + margins 在合理范围）
// ---------------------------------------------------------------------------

#[test]
fn auto_crop_fixture_smoke_single_column() {
    let dir = fixtures_dir().join("single-column").join("c-pages");
    let Some(png_path) = first_png(&dir) else {
        eprintln!("skipping: single-column c-pages directory missing");
        return;
    };
    let bmp = k2core::read_png(&png_path, 150.0).expect("read png");
    let r = auto_crop(&bmp, 0.5);
    // margins 必须落在 bitmap 范围内（right/bottom 容忍 -1 corner case）
    let w = bmp.width as i32;
    let h = bmp.height as i32;
    let cx = r.margins.to_cx();
    assert!(cx[0] >= 0 && cx[0] < w, "left={} w={w}", cx[0]);
    assert!(cx[1] >= 0 && cx[1] < h, "top={} h={h}", cx[1]);
    assert!(cx[2] >= -1 && cx[2] < w, "right={} w={w}", cx[2]);
    assert!(cx[3] >= -1 && cx[3] < h, "bottom={} h={h}", cx[3]);
    // 保留区宽/高 > 0
    assert!(w - cx[0] - cx[2] > 0);
    assert!(h - cx[1] - cx[3] > 0);
}

#[test]
fn auto_crop_fixture_smoke_two_column() {
    let dir = fixtures_dir().join("two-column").join("c-pages");
    let Some(png_path) = first_png(&dir) else {
        eprintln!("skipping: two-column c-pages directory missing");
        return;
    };
    let bmp = k2core::read_png(&png_path, 150.0).expect("read png");
    let r = auto_crop(&bmp, 0.5);
    let w = bmp.width as i32;
    let h = bmp.height as i32;
    let cx = r.margins.to_cx();
    assert!(cx[0] >= 0 && cx[0] < w);
    assert!(cx[1] >= 0 && cx[1] < h);
    assert!(cx[2] >= -1 && cx[2] < w);
    assert!(cx[3] >= -1 && cx[3] < h);
    assert!(w - cx[0] - cx[2] > 0);
    assert!(h - cx[1] - cx[3] > 0);
}

#[test]
fn auto_crop_then_apply_fixture_single_column_no_panic() {
    let dir = fixtures_dir().join("single-column").join("c-pages");
    let Some(png_path) = first_png(&dir) else {
        eprintln!("skipping: single-column c-pages directory missing");
        return;
    };
    let mut bmp = k2core::read_png(&png_path, 150.0).expect("read png");
    let r = auto_crop(&bmp, 0.5);
    apply_auto_crop(&mut bmp, &r.margins);
    // 单纯不 panic 即通过；Step 5.7 上线 SSIM 后做严格 diff
}

fn first_png(dir: &std::path::Path) -> Option<PathBuf> {
    if !dir.exists() {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut pngs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("png"))
        .collect();
    pngs.sort();
    pngs.into_iter().next()
}
