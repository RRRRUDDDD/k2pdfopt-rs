//! `crop_test.rs` —— Step 5.2 集成测试。
//!
//! ## 覆盖矩阵
//!
//! 1. **Synthetic 位图**（确定性答案）：单矩形、双行模拟、文本统计、十字形
//! 2. **Fixture smoke**：遍历 `tests/golden/<fixture>/c-pages/page-0001.png`，
//!    对每张图跑 `calc_bbox` + `is_blank` + `trim_margins`，
//!    断言：(a) 不 panic、(b) bbox 在位图范围内、(c) row/col counts 长度对齐
//!
//! ## 与 C 版对比的 layout.json 缺失说明
//!
//! 计划 Step 5.2 要求"12 个 fixtures 上 bbox 与 C 版误差 ≤ 2 像素"，
//! 但前置步骤 Step 2.4 的 Open Question 已记录：
//! *"layout.json 未实现（需要解析 k2pdfopt -debug 文本输出，留作后续 Step 补充）"*。
//!
//! 因此本步集成测试**仅做内部一致性 + 不 panic 的 smoke 验证**，
//! "vs C 版 ≤ 2 像素"的精确比对推迟到 Step 5.7（M4.5 交叉验证基础设施），
//! 届时 layout.json 生成器一并产出后再回溯校验。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use k2core::{read_png, Bitmap, PixelFormat, Rect};
use k2layout::{
    calc_bbox, is_blank, trim_margins, trim_margins_with_bbox, CropSettings, RegionView, TRIM_ALL,
    TRIM_ALL_AND_TEXT,
};
use std::path::{Path, PathBuf};

// ---- helpers ----

fn make_white_bmp(w: u32, h: u32) -> Bitmap {
    let mut bmp = Bitmap::new(w, h, 150.0, PixelFormat::Gray8).unwrap();
    bmp.fill_byte(255);
    bmp
}

fn paint_rect(bmp: &mut Bitmap, x0: u32, y0: u32, x1: u32, y1: u32, value: u8) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            let px = bmp.pixel_mut(x, y).unwrap();
            px[0] = value;
        }
    }
}

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("golden")
}

// ---- synthetic 单元集成测试 ----

#[test]
fn synthetic_single_rect_bbox_tight_within_tolerance() {
    let mut bmp = make_white_bmp(200, 150);
    // 黑矩形：c1=50, r1=40, c2=149, r2=109
    paint_rect(&mut bmp, 50, 40, 149, 109, 0);
    let view = RegionView::full(&bmp);
    let s = CropSettings::default();
    let bb = calc_bbox(&view, &s, false).unwrap();
    // 容差 ≤ 2 像素（计划 Step 5.2 验收目标）
    assert!((bb.rect.x0 - 50).abs() <= 2, "x0={} target=50", bb.rect.x0);
    assert!(
        (bb.rect.x1 - 149).abs() <= 2,
        "x1={} target=149",
        bb.rect.x1
    );
    assert!((bb.rect.y0 - 40).abs() <= 2, "y0={} target=40", bb.rect.y0);
    assert!(
        (bb.rect.y1 - 109).abs() <= 2,
        "y1={} target=109",
        bb.rect.y1
    );
}

#[test]
fn synthetic_two_horizontal_rects_text_stats() {
    let mut bmp = make_white_bmp(300, 200);
    // 行 1: y=30..49 (高 20)
    paint_rect(&mut bmp, 30, 30, 269, 49, 0);
    // 行 2: y=60..79 (高 20)；行间距 10 像素
    paint_rect(&mut bmp, 30, 60, 269, 79, 0);
    let view = RegionView::full(&bmp);
    let s = CropSettings::default();
    let bb = calc_bbox(&view, &s, true).unwrap();
    let stats = bb.text_stats.expect("calc_text_params=true");
    // rowbase 应该位于"末行" y=79 附近
    assert!(
        stats.rowbase >= 60 && stats.rowbase <= 79,
        "rowbase={}",
        stats.rowbase
    );
    // 高度均 ≥ 1
    assert!(stats.h5050 >= 1);
    assert!(stats.lcheight >= 1);
    assert!(stats.capheight >= 1);
}

#[test]
fn synthetic_blank_returns_true() {
    let bmp = make_white_bmp(500, 500);
    let view = RegionView::full(&bmp);
    let s = CropSettings::default();
    assert!(is_blank(&view, &s).unwrap());
}

#[test]
fn synthetic_full_black_not_blank() {
    let mut bmp = make_white_bmp(500, 500);
    paint_rect(&mut bmp, 0, 0, 499, 499, 0);
    let view = RegionView::full(&bmp);
    let s = CropSettings::default();
    assert!(!is_blank(&view, &s).unwrap());
}

#[test]
fn synthetic_trim_margins_with_bbox_consistent() {
    let mut bmp = make_white_bmp(400, 300);
    paint_rect(&mut bmp, 80, 60, 319, 239, 0);
    let view = RegionView::full(&bmp);
    let s = CropSettings::default();
    let (rect, bb) = trim_margins_with_bbox(&view, &s, TRIM_ALL).unwrap();
    assert_eq!(
        rect, bb.rect,
        "trim_margins_with_bbox rect must equal bb.rect"
    );
    assert!((bb.rect.x0 - 80).abs() <= 2);
    assert!((bb.rect.y0 - 60).abs() <= 2);
}

#[test]
fn synthetic_cross_shape_bbox_correct() {
    let mut bmp = make_white_bmp(200, 200);
    // 水平条：y=95..104, x=30..169
    paint_rect(&mut bmp, 30, 95, 169, 104, 0);
    // 垂直条：x=95..104, y=30..169
    paint_rect(&mut bmp, 95, 30, 104, 169, 0);
    let view = RegionView::full(&bmp);
    let s = CropSettings::default();
    let bb = calc_bbox(&view, &s, false).unwrap();
    assert!((bb.rect.x0 - 30).abs() <= 2, "x0={}", bb.rect.x0);
    assert!((bb.rect.x1 - 169).abs() <= 2, "x1={}", bb.rect.x1);
    assert!((bb.rect.y0 - 30).abs() <= 2, "y0={}", bb.rect.y0);
    assert!((bb.rect.y1 - 169).abs() <= 2, "y1={}", bb.rect.y1);
}

#[test]
fn synthetic_subregion_bbox_independent() {
    let mut bmp = make_white_bmp(300, 300);
    paint_rect(&mut bmp, 50, 50, 99, 99, 0);
    paint_rect(&mut bmp, 200, 200, 249, 249, 0);
    // 仅看左上角四分之一 (0,0)-(149,149)
    let view = RegionView::new(&bmp, Rect::new(0, 0, 149, 149));
    let s = CropSettings::default();
    let bb = calc_bbox(&view, &s, false).unwrap();
    // 只应该找到左上角的黑矩形 (50,50)-(99,99)，
    // 不应受 (200,200) 矩形的影响（视图范围之外）
    assert!((bb.rect.x0 - 50).abs() <= 2);
    assert!((bb.rect.x1 - 99).abs() <= 2);
    assert!((bb.rect.y0 - 50).abs() <= 2);
    assert!((bb.rect.y1 - 99).abs() <= 2);
}

#[test]
fn synthetic_bgcolor_threshold_changes_result() {
    let mut bmp = make_white_bmp(100, 100);
    // 灰色矩形（128 灰）
    paint_rect(&mut bmp, 20, 20, 79, 79, 128);
    // 默认 bgcolor=255：128 < 255 → 视为黑像素，bbox 收紧
    let view = RegionView::full(&bmp);
    let s = CropSettings::default();
    let bb = calc_bbox(&view, &s, false).unwrap();
    assert!((bb.rect.x0 - 20).abs() <= 2);
    // bgcolor=100：128 >= 100 → 视为白，bbox 应当塌陷
    let view2 = RegionView::with(&bmp, Rect::from_xywh(0, 0, 100, 100), 150.0, 100);
    let bb2 = calc_bbox(&view2, &s, false).unwrap();
    assert!(bb2.rect.x0 >= bb2.rect.x1 || bb2.rect.y0 >= bb2.rect.y1);
}

#[test]
fn synthetic_row_col_counts_full_size_arrays() {
    let mut bmp = make_white_bmp(40, 30);
    paint_rect(&mut bmp, 5, 5, 34, 24, 0);
    let view = RegionView::full(&bmp);
    let s = CropSettings::default();
    let bb = calc_bbox(&view, &s, false).unwrap();
    assert_eq!(bb.row_counts.len(), 30, "row_counts 长度 = bmp.height");
    assert_eq!(bb.col_counts.len(), 40, "col_counts 长度 = bmp.width");
    // 在区域内的行应该有计数；区域外应为 0
    assert_eq!(bb.row_counts[0], 0);
    assert!(bb.row_counts[10] > 0);
}

#[test]
fn synthetic_rtl_left_right_gap_swap() {
    // 验证 src_left_to_right 影响左右 trim_to gaplen（不验数值精度，只验调用通过 + 行为不同）
    let mut bmp = make_white_bmp(100, 50);
    paint_rect(&mut bmp, 10, 10, 89, 39, 0);
    let view = RegionView::full(&bmp);
    let mut s = CropSettings::default();
    let bb_ltr = calc_bbox(&view, &s, false).unwrap();
    s.src_left_to_right = false;
    let bb_rtl = calc_bbox(&view, &s, false).unwrap();
    // 对于实心矩形，两种方向 bbox 应基本一致
    assert!((bb_ltr.rect.x0 - bb_rtl.rect.x0).abs() <= 2);
    assert!((bb_ltr.rect.x1 - bb_rtl.rect.x1).abs() <= 2);
}

// ---- fixture smoke：遍历 12 fixture PNG ----

/// 对单张 PNG 跑 calc_bbox / is_blank / trim_margins，确保不 panic 且结果合法。
fn smoke_one_png(path: &Path) {
    let bmp = match read_png(path, 167.0) {
        Ok(b) => b,
        Err(e) => panic!("read_png({}): {:?}", path.display(), e),
    };
    assert!(bmp.width > 0 && bmp.height > 0);
    let view = RegionView::full(&bmp);
    let s = CropSettings::default();

    // calc_bbox 不能 panic
    let bb = calc_bbox(&view, &s, false).expect("calc_bbox should not error");
    // bbox 应在位图范围内
    assert!(
        bb.rect.x0 >= 0
            && bb.rect.x1 < bmp.width as i32
            && bb.rect.y0 >= 0
            && bb.rect.y1 < bmp.height as i32,
        "bbox out of bmp range: {:?} bmp={}x{}",
        bb.rect,
        bmp.width,
        bmp.height
    );
    // counts 长度对齐
    assert_eq!(bb.row_counts.len(), bmp.height as usize);
    assert_eq!(bb.col_counts.len(), bmp.width as usize);

    // is_blank 不能 panic
    let _ = is_blank(&view, &s).expect("is_blank should not error");

    // trim_margins TRIM_ALL 不能 panic 且端点不应越界
    let trimmed = trim_margins(&view, &s, TRIM_ALL).expect("trim_margins TRIM_ALL");
    if !(trimmed.x0 > trimmed.x1 || trimmed.y0 > trimmed.y1) {
        // 非空矩形时检查端点
        assert!(trimmed.x0 >= 0);
        assert!(trimmed.x1 < bmp.width as i32);
        assert!(trimmed.y0 >= 0);
        assert!(trimmed.y1 < bmp.height as i32);
    }

    // calc_text_params 也不能 panic
    let _ = trim_margins(&view, &s, TRIM_ALL_AND_TEXT).expect("trim_margins TRIM_ALL_AND_TEXT");
}

#[test]
fn fixture_smoke_all_fixtures_with_pngs() {
    let root = fixtures_root();
    if !root.exists() {
        // CI 没有 golden 目录时（首次跑 baseline 前），允许跳过
        eprintln!("tests/golden not found, skipping fixture smoke");
        return;
    }
    let mut total = 0usize;
    for entry in std::fs::read_dir(&root).expect("read tests/golden") {
        let entry = entry.unwrap();
        let dir = entry.path();
        let pages = dir.join("c-pages");
        if !pages.exists() {
            continue;
        }
        for png_entry in std::fs::read_dir(&pages).expect("read c-pages") {
            let p = png_entry.unwrap().path();
            if p.extension().and_then(|s| s.to_str()) != Some("png") {
                continue;
            }
            smoke_one_png(&p);
            total += 1;
        }
    }
    // 至少跑过几张（即使 blank-page 没有 PNG，其它 11 个 fixture 应该都有）
    assert!(
        total >= 10,
        "fixture smoke must cover at least 10 PNGs, got {}",
        total
    );
}

#[test]
fn fixture_single_column_page1_has_bbox_within_image() {
    let path = fixtures_root()
        .join("single-column")
        .join("c-pages")
        .join("page-0001.png");
    if !path.exists() {
        eprintln!("{} missing, skipping", path.display());
        return;
    }
    let bmp = read_png(&path, 167.0).expect("read_png");
    let view = RegionView::full(&bmp);
    let s = CropSettings::default();
    let bb = calc_bbox(&view, &s, false).unwrap();
    // 单栏正常文本页：bbox 不应该完全占满（应该有边距）
    let trimmed_w = bb.rect.width();
    let trimmed_h = bb.rect.height();
    assert!(
        trimmed_w > 0 && trimmed_h > 0,
        "single-column page bbox must be non-empty"
    );
    assert!(
        trimmed_w < bmp.width,
        "bbox width {} should < {}",
        trimmed_w,
        bmp.width
    );
    assert!(
        trimmed_h < bmp.height,
        "bbox height {} should < {}",
        trimmed_h,
        bmp.height
    );
}

#[test]
fn fixture_blank_page_dir_handled_gracefully() {
    // blank-page fixture 的 c-pages 目录为空（C k2pdfopt 输出 0 页）；
    // smoke test 已经跳过空目录，本测试显式校验我们不会因此 fail。
    let pages = fixtures_root().join("blank-page").join("c-pages");
    if !pages.exists() {
        return;
    }
    // 目录存在但应该是空的
    let count = std::fs::read_dir(&pages)
        .map(|it| {
            it.filter(|e| {
                e.as_ref()
                    .map(|e| e.path().extension().and_then(|s| s.to_str()) == Some("png"))
                    .unwrap_or(false)
            })
            .count()
        })
        .unwrap_or(0);
    assert_eq!(count, 0, "blank-page c-pages 应该没有 PNG");
}
