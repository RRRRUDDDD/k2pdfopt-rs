//! `breakpoints_test.rs` —— Step 7.1 集成测试（垂直分页点检测）。
//!
//! ## 覆盖矩阵
//!
//! 1. **Synthetic 合成位图**（确定性答案）：
//!    - 早退路径：rows<maxsize / fit=-2 / fit=0 接近一页
//!    - fit_to_page = 0 / -1 / -2 / >0 四模式扫描
//!    - 多文本带 master 找正确切分位置
//!    - row0 偏移（mid-page 起切）
//!    - apply_page_break_marks（BREAKPAGE / NOBREAK 组合）
//! 2. **源 PDF 渲染 smoke**（用 mutool 渲染 `tests/fixtures/*.pdf`）：
//!    - `single-column.pdf` 作为 master smoke
//!    - `chinese.pdf` smoke
//! 3. **C 版输出 PNG smoke**（`tests/golden/<fixture>/c-pages/`）：
//!    - 拿 C 版输出页 PNG 当 master，验证 break_point 不 panic
//! 4. **API / 边界**：
//!    - mark 上限 32 / OutputPaginator 集成 / 越界 row0 / 空 master
//!    - BreakSettings 默认值

#![allow(clippy::unwrap_used, clippy::expect_used)]

use k2core::{read_png, Bitmap, PixelFormat};
use k2layout::{
    apply_page_break_marks, find_break_point, find_break_point_ignoring_marks, BreakSettings,
    OutputPaginator, PageBreakMark, RowSettings, MARK_TYPE_BREAKPAGE, MARK_TYPE_DISABLED,
    MARK_TYPE_NOBREAK, MAX_PAGE_BREAK_MARKS,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

// ---- helpers ----

fn make_white_bmp(w: u32, h: u32, dpi: f32) -> Bitmap {
    let mut bmp = Bitmap::new(w, h, dpi, PixelFormat::Gray8).unwrap();
    bmp.fill_byte(255);
    bmp
}

fn paint_band(bmp: &mut Bitmap, y_start: u32, height: u32, col0: u32, col1: u32) {
    for y in y_start..(y_start + height).min(bmp.height) {
        for x in col0..col1.min(bmp.width) {
            if let Some(px) = bmp.pixel_mut(x, y) {
                px[0] = 0;
            }
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

fn fixture_pdfs_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("fixtures")
}

fn render_pdf_first_page(pdf: &Path, dpi: u32) -> Option<Bitmap> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let tmp_dir = std::env::temp_dir().join(format!("k2rs_breakpoints_test_{nanos}"));
    std::fs::create_dir_all(&tmp_dir).ok()?;
    let out = tmp_dir.join("page.png");
    let res = Command::new("mutool")
        .args([
            "draw",
            "-o",
            out.to_str()?,
            "-r",
            &dpi.to_string(),
            "-c",
            "gray",
            pdf.to_str()?,
            "1",
        ])
        .output()
        .ok()?;
    if !res.status.success() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return None;
    }
    let bmp = read_png(&out, dpi as f32).ok();
    let _ = std::fs::remove_dir_all(&tmp_dir);
    bmp
}

fn settings_default() -> BreakSettings {
    BreakSettings::default()
}

// =====================================================================
// 1. Synthetic 早退路径
// =====================================================================

#[test]
fn synthetic_rows_less_than_maxsize_early_returns() {
    let bmp = make_white_bmp(300, 200, 167.0);
    let r = find_break_point_ignoring_marks(&bmp, 100, 0, 200, &settings_default()).unwrap();
    // rows=100 < maxsize=200，直接返回 rows
    assert_eq!(r, 100);
}

#[test]
fn synthetic_fit_minus_two_returns_full_rows_even_when_huge() {
    let bmp = make_white_bmp(300, 4000, 167.0);
    let settings = BreakSettings {
        fit_to_page: -2,
        ..settings_default()
    };
    let r = find_break_point_ignoring_marks(&bmp, 3500, 0, 100, &settings).unwrap();
    assert_eq!(r, 3500);
}

#[test]
fn synthetic_fit_zero_near_one_page_returns_rows() {
    let bmp = make_white_bmp(300, 800, 167.0);
    let r = find_break_point_ignoring_marks(&bmp, 500, 0, 500, &settings_default()).unwrap();
    assert_eq!(r, 500);
}

// =====================================================================
// 2. Synthetic 多文本带切分
// =====================================================================

#[test]
fn synthetic_three_bands_split_returns_positive_count() {
    // master: 3 个 30px 高文本带，分别在 y=50/150/250
    let mut bmp = make_white_bmp(400, 500, 167.0);
    paint_band(&mut bmp, 50, 30, 20, 380);
    paint_band(&mut bmp, 150, 30, 20, 380);
    paint_band(&mut bmp, 250, 30, 20, 380);
    let r = find_break_point_ignoring_marks(&bmp, 400, 0, 200, &settings_default()).unwrap();
    // maxsize=200，期望切到第二个带之后
    assert!(r > 0, "rowcount={r} 应 > 0");
    assert!(
        r <= 280, // scanheight=200, scan_actual=280
        "rowcount={r} 应 <= scan_actual=280"
    );
}

#[test]
fn synthetic_row0_offset_skips_top_content() {
    // master 顶部有内容（被跳过），row0=100 后才参与扫描
    let mut bmp = make_white_bmp(400, 500, 167.0);
    paint_band(&mut bmp, 30, 30, 20, 380); // 顶部带（被 row0 跳过）
    paint_band(&mut bmp, 200, 30, 20, 380);
    paint_band(&mut bmp, 300, 30, 20, 380);
    let r = find_break_point_ignoring_marks(&bmp, 400, 100, 200, &settings_default()).unwrap();
    // rows = 400-100 = 300 > maxsize=200 → 真扫描
    assert!(r > 0);
}

#[test]
fn synthetic_empty_master_falls_back_to_scanheight() {
    let bmp = make_white_bmp(400, 600, 167.0);
    let r = find_break_point_ignoring_marks(&bmp, 500, 0, 200, &settings_default()).unwrap();
    // 全白 master，find_textrows 空 → fallback = scanheight = maxsize = 200
    assert_eq!(r, 200);
}

#[test]
fn synthetic_fit_to_page_minus_one_uses_full_rows_as_scanheight() {
    let mut bmp = make_white_bmp(400, 600, 167.0);
    paint_band(&mut bmp, 100, 30, 20, 380);
    paint_band(&mut bmp, 250, 30, 20, 380);
    paint_band(&mut bmp, 400, 30, 20, 380);
    let settings = BreakSettings {
        fit_to_page: -1,
        ..settings_default()
    };
    let r = find_break_point_ignoring_marks(&bmp, 500, 0, 100, &settings).unwrap();
    assert!(r > 0);
}

#[test]
fn synthetic_fit_to_page_positive_scales_scanheight() {
    let mut bmp = make_white_bmp(400, 600, 167.0);
    paint_band(&mut bmp, 50, 30, 20, 380);
    paint_band(&mut bmp, 200, 30, 20, 380);
    let settings = BreakSettings {
        fit_to_page: 30, // scanheight = 1.3*maxsize
        ..settings_default()
    };
    let r = find_break_point_ignoring_marks(&bmp, 500, 0, 150, &settings).unwrap();
    assert!(r > 0);
}

// =====================================================================
// 3. apply_page_break_marks 集成
// =====================================================================

#[test]
fn synthetic_apply_marks_breakpage_forces_split() {
    let bmp = make_white_bmp(400, 600, 167.0);
    let settings = settings_default();
    let mut marks = vec![PageBreakMark {
        row: 75,
        mark_type: MARK_TYPE_BREAKPAGE,
    }];
    // master 全白 → fallback=200，但 mark.row=75 < 200 → 强制 75
    let r = find_break_point(&bmp, 500, 0, 200, &settings, &mut marks).unwrap();
    assert_eq!(r, 75);
    assert_eq!(marks[0].mark_type, MARK_TYPE_DISABLED);
}

#[test]
fn synthetic_apply_marks_nobreak_pair_rewinds() {
    let bmp = make_white_bmp(400, 600, 167.0);
    let settings = settings_default();
    // NOBREAK 区间 [50, 250] 围住 rowcount=200 → 回退到 50
    let mut marks = vec![
        PageBreakMark {
            row: 50,
            mark_type: MARK_TYPE_NOBREAK,
        },
        PageBreakMark {
            row: 250,
            mark_type: MARK_TYPE_NOBREAK,
        },
    ];
    let r = find_break_point(&bmp, 500, 0, 200, &settings, &mut marks).unwrap();
    assert_eq!(r, 50);
}

#[test]
fn synthetic_apply_marks_with_row0_offset() {
    let bmp = make_white_bmp(400, 600, 167.0);
    let settings = settings_default();
    let mut marks = vec![PageBreakMark {
        row: 150,
        mark_type: MARK_TYPE_BREAKPAGE,
    }];
    // row0=50, mark.row=150 → rowcount = 150-50 = 100
    let r = find_break_point(&bmp, 500, 50, 200, &settings, &mut marks).unwrap();
    assert_eq!(r, 100);
}

#[test]
fn synthetic_apply_marks_beyond_page_ignored() {
    let bmp = make_white_bmp(400, 600, 167.0);
    let settings = settings_default();
    let mut marks = vec![PageBreakMark {
        row: 400,
        mark_type: MARK_TYPE_BREAKPAGE,
    }];
    // mark.row=400 > rowcount+row0=200+0 → 不消费
    let r = find_break_point(&bmp, 500, 0, 200, &settings, &mut marks).unwrap();
    assert_eq!(r, 200);
    // mark 仍未消费
    assert_eq!(marks[0].mark_type, MARK_TYPE_BREAKPAGE);
}

#[test]
fn synthetic_apply_marks_empty_uses_short_circuit() {
    let bmp = make_white_bmp(400, 300, 167.0);
    let settings = settings_default();
    let mut marks: Vec<PageBreakMark> = Vec::new();
    let r = find_break_point(&bmp, 200, 0, 300, &settings, &mut marks).unwrap();
    // rows=200 < maxsize=300 → 早退 200
    assert_eq!(r, 200);
}

// =====================================================================
// 4. 源 PDF 渲染 smoke
// =====================================================================

#[test]
fn source_single_column_pdf_break_point_smoke() {
    let pdf = fixture_pdfs_root().join("single-column.pdf");
    if !pdf.exists() {
        eprintln!("SKIP: 缺少 fixture {pdf:?}");
        return;
    }
    let Some(bmp) = render_pdf_first_page(&pdf, 150) else {
        eprintln!("SKIP: mutool 渲染失败");
        return;
    };
    let settings = BreakSettings {
        dst_dpi: 150,
        ..settings_default()
    };
    // 用 PDF 第一页当 master canvas，maxsize=300 找下一页
    let r = find_break_point_ignoring_marks(&bmp, bmp.height, 0, 300, &settings).unwrap();
    assert!(r > 0, "rowcount={r} 应 > 0");
    assert!(
        r <= bmp.height,
        "rowcount={r} 应 <= bitmap height {}",
        bmp.height
    );
}

#[test]
fn source_chinese_pdf_break_point_smoke() {
    let pdf = fixture_pdfs_root().join("chinese.pdf");
    if !pdf.exists() {
        eprintln!("SKIP: 缺少 fixture {pdf:?}");
        return;
    }
    let Some(bmp) = render_pdf_first_page(&pdf, 150) else {
        eprintln!("SKIP: mutool 渲染失败");
        return;
    };
    let settings = BreakSettings {
        dst_dpi: 150,
        ..settings_default()
    };
    let r = find_break_point_ignoring_marks(&bmp, bmp.height, 0, 400, &settings).unwrap();
    assert!(r > 0);
}

#[test]
fn source_complex_layout_pdf_break_point_smoke() {
    let pdf = fixture_pdfs_root().join("complex-layout.pdf");
    if !pdf.exists() {
        eprintln!("SKIP: 缺少 fixture {pdf:?}");
        return;
    }
    let Some(bmp) = render_pdf_first_page(&pdf, 150) else {
        eprintln!("SKIP: mutool 渲染失败");
        return;
    };
    let settings = BreakSettings {
        dst_dpi: 150,
        ..settings_default()
    };
    let r = find_break_point_ignoring_marks(&bmp, bmp.height, 0, 500, &settings).unwrap();
    assert!(r > 0);
}

// =====================================================================
// 5. C 版输出 PNG smoke
// =====================================================================

fn c_output_first_page(fixture_name: &str) -> Option<Bitmap> {
    let pages_dir = fixtures_root().join(fixture_name).join("c-pages");
    if !pages_dir.exists() {
        return None;
    }
    let entries: Vec<_> = std::fs::read_dir(&pages_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("png"))
                .unwrap_or(false)
        })
        .collect();
    let mut paths: Vec<_> = entries.into_iter().map(|e| e.path()).collect();
    paths.sort();
    let first = paths.first()?;
    read_png(first, 167.0).ok()
}

#[test]
fn c_output_single_column_break_point_smoke() {
    let Some(bmp) = c_output_first_page("single-column") else {
        eprintln!("SKIP: 缺少 c-pages 输出");
        return;
    };
    let settings = settings_default();
    let r = find_break_point_ignoring_marks(&bmp, bmp.height, 0, 600, &settings).unwrap();
    assert!(r > 0);
}

#[test]
fn c_output_two_column_break_point_smoke() {
    let Some(bmp) = c_output_first_page("two-column") else {
        eprintln!("SKIP: 缺少 c-pages 输出");
        return;
    };
    let settings = settings_default();
    let r = find_break_point_ignoring_marks(&bmp, bmp.height, 0, 600, &settings).unwrap();
    assert!(r > 0);
}

#[test]
fn c_output_scanned_break_point_smoke() {
    let Some(bmp) = c_output_first_page("scanned") else {
        eprintln!("SKIP: 缺少 c-pages 输出");
        return;
    };
    let settings = settings_default();
    let r = find_break_point_ignoring_marks(&bmp, bmp.height, 0, 600, &settings).unwrap();
    assert!(r > 0);
}

// =====================================================================
// 6. API / 边界
// =====================================================================

#[test]
fn output_paginator_integration_add_mark_then_apply() {
    // OutputPaginator + apply_page_break_marks 串联：模拟 publisher 工作流
    let mut paginator = OutputPaginator::new();
    paginator.add_breakpage_mark(75);
    paginator.add_nobreak_mark(200);
    paginator.add_nobreak_mark(280);

    assert_eq!(paginator.pagebreak_marks.len(), 3);

    // 用 apply_page_break_marks 直接消费
    let adjusted = apply_page_break_marks(150, 0, &mut paginator.pagebreak_marks);
    // 第一个 mark.row=75 是 BREAKPAGE 且 75 < rowcount+row0=150 → 强制 75
    assert_eq!(adjusted, 75);
    assert_eq!(
        paginator.pagebreak_marks[0].mark_type, MARK_TYPE_DISABLED,
        "BREAKPAGE 应被消费"
    );
}

#[test]
fn output_paginator_max_marks_drops_extras() {
    let mut paginator = OutputPaginator::new();
    for i in 0..MAX_PAGE_BREAK_MARKS {
        assert!(
            paginator.add_pagebreak_mark(i as u32, MARK_TYPE_BREAKPAGE),
            "mark #{i} 应成功"
        );
    }
    // 第 33 个应被丢弃
    assert!(!paginator.add_pagebreak_mark(999, MARK_TYPE_BREAKPAGE));
    assert_eq!(paginator.pagebreak_marks.len(), MAX_PAGE_BREAK_MARKS);
}

#[test]
fn settings_default_round_trips() {
    let s = BreakSettings::default();
    assert_eq!(s.fit_to_page, 0);
    assert_eq!(s.dst_dpi, 167);
    assert!(!s.join_figure_captions);
    assert_eq!(s.bgcolor, 255);
    // RowSettings 默认值应等价
    let rs = RowSettings::default();
    assert_eq!(s.row_settings.src_left_to_right, rs.src_left_to_right);
}

#[test]
fn zero_row0_zero_maxsize_does_not_panic() {
    // 极端边界：row0=0, maxsize=0
    // rows=10 > maxsize=0 → 进入 scanheight 计算；scanheight=0; r2<=scanheight=0
    // → r2=0; rowcount=0 if 0<0/4=0 false → rowcount=r1=0; <=2 fallback=scan_i=0
    // 不应 panic，结果合理（即使为 0）
    let bmp = make_white_bmp(100, 100, 167.0);
    let settings = settings_default();
    let r = find_break_point_ignoring_marks(&bmp, 10, 0, 0, &settings).unwrap();
    // 返回的 rowcount 应非负
    let _ = r;
}

#[test]
fn row0_equals_master_rows_returns_zero() {
    // row0 == master_rows → rows=0 → 早退返回 0
    let bmp = make_white_bmp(100, 100, 167.0);
    let settings = settings_default();
    let r = find_break_point_ignoring_marks(&bmp, 50, 50, 100, &settings).unwrap();
    assert_eq!(r, 0);
}
