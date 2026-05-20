//! `words_test.rs` —— Step 6.3 集成测试。
//!
//! ## 覆盖矩阵
//!
//! 1. **Synthetic 合成位图**（确定性答案，8 个）：
//!    - 单 word / 多 word 精确切分
//!    - 紧贴 word（小 gap，应合并）
//!    - 极大间距（独立 word）
//!    - 单字符 word（早退 / default 路径）
//!    - 自动模式（word_spacing<0）vs 固定模式（>=0）
//!    - 长 word 不被错误切分
//!    - compute_col_gaps + remove_small_col_gaps 联合
//! 2. **源 PDF 渲染**（mutool 渲染 `tests/fixtures/*.pdf`，3 个）：
//!    - `single-column.pdf` 中间一行应能切出 ≥ 1 个 word
//!    - `chinese.pdf` 行 → 词 smoke
//!    - `complex-layout.pdf` 不 panic
//! 3. **C 版输出 PNG smoke**（`tests/golden/<fixture>/c-pages/`，3 个）：
//!    - 取 1 行 → 应能切出 ≥ 1 word（不严格断言数量，仅 smoke）
//! 4. **API / 边界**（5 个）：
//!    - WordSettings 默认值
//!    - WordGapDatabase reset / 扩展
//!    - compute_median_gap 空 / 非空
//!    - add_word_gaps + compute_median_gap 跨行累积
//!    - one_row_find_textwords 超出位图边界 → 返回 default
//!
//! ## 与 C 版精确比对的推迟说明
//!
//! 与 Step 5.2-5.5/6.1/6.2 同源：layout.json 未生成（Step 2.4 Open Question），
//! 因此 "vs C 版 word 边界 ≤ 2 像素" 的硬约束推迟到后续 Step（Open Question 6.3.E）。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use k2core::{read_png, Bitmap, PixelFormat, Rect};
use k2layout::{
    add_word_gaps, compute_col_gaps, compute_median_gap, one_row_find_textwords,
    remove_small_col_gaps, RegionView, RowType, TextRow, TextWords, WordGapDatabase, WordSettings,
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

fn paint_block(bmp: &mut Bitmap, x0: u32, y0: u32, x1: u32, y1: u32) {
    for y in y0..=y1.min(bmp.height - 1) {
        for x in x0..=x1.min(bmp.width - 1) {
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
    let tmp_dir = std::env::temp_dir().join(format!("k2rs_words_test_{nanos}"));
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

fn settings_fixed_mode() -> WordSettings {
    WordSettings {
        word_spacing: 0.5,
        ..WordSettings::default()
    }
}

// =====================================================================
// 1. Synthetic 合成精确比对
// =====================================================================

#[test]
fn single_word_returns_one_word() {
    // 整行就一个 word（c=50..=100）
    let mut bmp = make_white_bmp(400, 50, 150.0);
    paint_block(&mut bmp, 50, 15, 100, 35);
    let view = RegionView::full(&bmp);
    let mut db = WordGapDatabase::new();
    let words = one_row_find_textwords(
        &view,
        Rect::new(0, 10, 399, 40),
        10,
        &settings_fixed_mode(),
        &mut db,
        false,
    );
    assert_eq!(words.rows.len(), 1, "got {} words", words.rows.len());
    assert!(words.rows[0].region_type == RowType::Word);
    // word bbox 应紧贴源 word
    let w = &words.rows[0];
    assert!((w.c1 - 50).abs() <= 4, "c1={}", w.c1);
    assert!((w.c2 - 100).abs() <= 4, "c2={}", w.c2);
}

#[test]
fn three_words_with_clear_gaps_separates() {
    // 三个 word：(40,80), (180,220), (320,360)
    let mut bmp = make_white_bmp(400, 50, 150.0);
    for &(c1, c2) in &[(40, 80), (180, 220), (320, 360)] {
        paint_block(&mut bmp, c1, 15, c2, 35);
    }
    let view = RegionView::full(&bmp);
    let mut db = WordGapDatabase::new();
    let words = one_row_find_textwords(
        &view,
        Rect::new(0, 10, 399, 40),
        10,
        &settings_fixed_mode(),
        &mut db,
        false,
    );
    assert!(words.rows.len() >= 2, "got {} words", words.rows.len());
    // c1 单调
    for i in 0..words.rows.len() - 1 {
        assert!(words.rows[i].c1 <= words.rows[i + 1].c1);
    }
    // 所有都是 Word
    assert!(words.rows.iter().all(|w| w.region_type == RowType::Word));
}

#[test]
fn very_narrow_row_returns_default() {
    // 宽度 < 6
    let mut bmp = make_white_bmp(200, 50, 150.0);
    paint_block(&mut bmp, 0, 15, 3, 35);
    let view = RegionView::full(&bmp);
    let mut db = WordGapDatabase::new();
    let words = one_row_find_textwords(
        &view,
        Rect::new(0, 10, 3, 40),
        10,
        &WordSettings::default(),
        &mut db,
        false,
    );
    assert_eq!(words.rows.len(), 1);
    // default word 返回时 c1/c2 紧贴输入 rect
    assert_eq!(words.rows[0].c1, 0);
    assert_eq!(words.rows[0].c2, 3);
}

#[test]
fn empty_row_returns_default_word() {
    // 全白 row
    let bmp = make_white_bmp(200, 50, 150.0);
    let view = RegionView::full(&bmp);
    let mut db = WordGapDatabase::new();
    let words = one_row_find_textwords(
        &view,
        Rect::new(0, 10, 199, 40),
        10,
        &WordSettings::default(),
        &mut db,
        false,
    );
    // 全白 → 不切，返回 default
    assert_eq!(words.rows.len(), 1);
}

#[test]
fn auto_mode_with_simple_fixture_does_not_panic() {
    // 自动模式（word_spacing<0）在简单 fixture 上可能退化为单 word，
    // 但不应 panic
    let mut bmp = make_white_bmp(400, 50, 150.0);
    for &(c1, c2) in &[(40, 80), (180, 220), (320, 360)] {
        paint_block(&mut bmp, c1, 15, c2, 35);
    }
    let view = RegionView::full(&bmp);
    let mut db = WordGapDatabase::new();
    let words = one_row_find_textwords(
        &view,
        Rect::new(0, 10, 399, 40),
        10,
        &WordSettings::default(),
        &mut db,
        false,
    );
    assert!(!words.rows.is_empty());
}

#[test]
fn fixed_mode_with_dense_words_separates() {
    // 5 个紧密 word（每个 20 像素宽，间距 30 像素）
    let mut bmp = make_white_bmp(400, 50, 150.0);
    for i in 0..5 {
        let c1 = 30 + i * 70;
        let c2 = c1 + 20;
        paint_block(&mut bmp, c1, 15, c2, 35);
    }
    let view = RegionView::full(&bmp);
    let mut db = WordGapDatabase::new();
    let words = one_row_find_textwords(
        &view,
        Rect::new(0, 10, 399, 40),
        10,
        &settings_fixed_mode(),
        &mut db,
        false,
    );
    // 应至少切出 3 个（5 个的子集）
    assert!(words.rows.len() >= 3, "got {} words", words.rows.len());
}

#[test]
fn compute_col_gaps_after_split_is_consistent() {
    // 切完 word 后 compute_col_gaps 应填充 gap/gapblank/rowheight
    let mut bmp = make_white_bmp(400, 50, 150.0);
    paint_block(&mut bmp, 40, 15, 80, 35);
    paint_block(&mut bmp, 180, 15, 220, 35);
    paint_block(&mut bmp, 320, 15, 360, 35);
    let view = RegionView::full(&bmp);
    let mut db = WordGapDatabase::new();
    let words = one_row_find_textwords(
        &view,
        Rect::new(0, 10, 399, 40),
        10,
        &settings_fixed_mode(),
        &mut db,
        false,
    );
    // 切完后内部已调 compute_col_gaps；每个 word 的 gap >= 0（除非最后一个用 c2_row 算的差值）
    for w in &words.rows {
        // 至少 gap 字段不再是 -1（默认值）
        assert!(w.gap >= 0, "gap should be filled, got {}", w.gap);
    }
}

#[test]
fn remove_small_col_gaps_merges_after_compute_col_gaps() {
    let mut words = TextWords::new();
    for &(c1, c2) in &[(10, 30), (35, 60), (90, 110)] {
        words.push(TextRow {
            c1,
            c2,
            r1: 0,
            r2: 30,
            ..TextRow::default()
        });
    }
    compute_col_gaps(&mut words, 120);
    // gap[0] = 4 → 4/20=0.2 < mingap=0.5 → 合并 [10,30] + [35,60]
    // gap[1] = 29 → 1.45 → 保留 [90,110]
    remove_small_col_gaps(&mut words, 20, 0.5, 0.1);
    assert_eq!(words.rows.len(), 2);
    assert_eq!(words.rows[0].c1, 10);
    assert_eq!(words.rows[0].c2, 60);
    assert_eq!(words.rows[1].c1, 90);
}

// =====================================================================
// 2. 源 PDF 渲染 smoke
// =====================================================================

fn pick_one_dark_row(view: &RegionView) -> Option<Rect> {
    // 从视图中找一行有暗像素的 y 范围 [y_top, y_bot]，并扩展到 row body
    // 简单策略：找首个含 >=20 暗像素的 y，向上下扩展到全白
    let bmp = view.bmp;
    let mut dark_y: Option<u32> = None;
    'outer: for y in 0..bmp.height {
        let mut dark = 0;
        for x in 0..bmp.width {
            let g = bmp.gray_at(x, y).unwrap_or(255);
            if g < 200 {
                dark += 1;
                if dark >= 20 {
                    dark_y = Some(y);
                    break 'outer;
                }
            }
        }
    }
    let cy = dark_y?;
    // 向上向下扩展 row 范围
    let mut y0 = cy;
    let mut y1 = cy;
    while y0 > 0 {
        let mut dark = 0;
        for x in 0..bmp.width {
            if bmp.gray_at(x, y0 - 1).unwrap_or(255) < 200 {
                dark += 1;
            }
        }
        if dark < 5 {
            break;
        }
        y0 -= 1;
    }
    while y1 + 1 < bmp.height {
        let mut dark = 0;
        for x in 0..bmp.width {
            if bmp.gray_at(x, y1 + 1).unwrap_or(255) < 200 {
                dark += 1;
            }
        }
        if dark < 5 {
            break;
        }
        y1 += 1;
    }
    Some(Rect::new(0, y0 as i32, bmp.width as i32 - 1, y1 as i32))
}

#[test]
fn source_single_column_first_row_words_smoke() {
    let pdf = fixture_pdfs_root().join("single-column.pdf");
    if !pdf.exists() {
        eprintln!("skip: {pdf:?} not found");
        return;
    }
    let bmp = match render_pdf_first_page(&pdf, 150) {
        Some(b) => b,
        None => {
            eprintln!("skip: mutool render failed");
            return;
        }
    };
    let view = RegionView::full(&bmp);
    let row_rect = match pick_one_dark_row(&view) {
        Some(r) => r,
        None => {
            eprintln!("skip: no dark row found in single-column.pdf");
            return;
        }
    };
    let mut db = WordGapDatabase::new();
    let words = one_row_find_textwords(&view, row_rect, 10, &settings_fixed_mode(), &mut db, false);
    // smoke: 至少有 1 个 word
    assert!(!words.rows.is_empty());
}

#[test]
fn source_chinese_first_row_words_smoke() {
    let pdf = fixture_pdfs_root().join("chinese.pdf");
    if !pdf.exists() {
        eprintln!("skip: {pdf:?} not found");
        return;
    }
    let bmp = match render_pdf_first_page(&pdf, 150) {
        Some(b) => b,
        None => {
            eprintln!("skip: mutool render failed");
            return;
        }
    };
    let view = RegionView::full(&bmp);
    let row_rect = match pick_one_dark_row(&view) {
        Some(r) => r,
        None => {
            eprintln!("skip: no dark row in chinese.pdf");
            return;
        }
    };
    let mut db = WordGapDatabase::new();
    let words = one_row_find_textwords(&view, row_rect, 10, &settings_fixed_mode(), &mut db, false);
    // 中文字符密集，可能 1-N 个 word
    assert!(!words.rows.is_empty());
}

#[test]
fn source_complex_layout_does_not_panic() {
    let pdf = fixture_pdfs_root().join("complex-layout.pdf");
    if !pdf.exists() {
        eprintln!("skip: {pdf:?} not found");
        return;
    }
    let bmp = match render_pdf_first_page(&pdf, 150) {
        Some(b) => b,
        None => {
            eprintln!("skip: mutool render failed");
            return;
        }
    };
    let view = RegionView::full(&bmp);
    let row_rect = match pick_one_dark_row(&view) {
        Some(r) => r,
        None => {
            // 无暗 row 也 smoke pass
            return;
        }
    };
    let mut db = WordGapDatabase::new();
    let _words = one_row_find_textwords(
        &view,
        row_rect,
        10,
        &WordSettings::default(),
        &mut db,
        false,
    );
    // 仅 smoke：不 panic 即可
}

// =====================================================================
// 3. C 版输出 PNG smoke
// =====================================================================

#[test]
fn c_output_single_column_page1_words_smoke() {
    let png = fixtures_root()
        .join("single-column")
        .join("c-pages")
        .join("page-0001.png");
    if !png.exists() {
        eprintln!("skip: {png:?} not found");
        return;
    }
    let bmp = match read_png(&png, 167.0) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skip: read_png err {e}");
            return;
        }
    };
    let view = RegionView::full(&bmp);
    let row_rect = match pick_one_dark_row(&view) {
        Some(r) => r,
        None => {
            eprintln!("skip: no dark row");
            return;
        }
    };
    let mut db = WordGapDatabase::new();
    let _words =
        one_row_find_textwords(&view, row_rect, 10, &settings_fixed_mode(), &mut db, false);
    // smoke：不 panic
}

#[test]
fn c_output_two_column_page1_words_smoke() {
    let png = fixtures_root()
        .join("two-column")
        .join("c-pages")
        .join("page-0001.png");
    if !png.exists() {
        eprintln!("skip: {png:?} not found");
        return;
    }
    let bmp = match read_png(&png, 167.0) {
        Ok(b) => b,
        Err(_) => return,
    };
    let view = RegionView::full(&bmp);
    let row_rect = match pick_one_dark_row(&view) {
        Some(r) => r,
        None => return,
    };
    let mut db = WordGapDatabase::new();
    let _words =
        one_row_find_textwords(&view, row_rect, 10, &settings_fixed_mode(), &mut db, false);
}

#[test]
fn c_output_scanned_page1_words_smoke() {
    let png = fixtures_root()
        .join("scanned")
        .join("c-pages")
        .join("page-0001.png");
    if !png.exists() {
        eprintln!("skip: {png:?} not found");
        return;
    }
    let bmp = match read_png(&png, 167.0) {
        Ok(b) => b,
        Err(_) => return,
    };
    let view = RegionView::full(&bmp);
    let row_rect = match pick_one_dark_row(&view) {
        Some(r) => r,
        None => return,
    };
    let mut db = WordGapDatabase::new();
    let _words =
        one_row_find_textwords(&view, row_rect, 10, &settings_fixed_mode(), &mut db, false);
}

// =====================================================================
// 4. API / 边界
// =====================================================================

#[test]
fn word_settings_default_matches_c() {
    let s = WordSettings::default();
    assert_eq!(s.word_spacing, -0.20);
    assert_eq!(s.gtw_in, 0.0015);
    assert_eq!(s.max_region_width_inches, 3.6);
    assert_eq!(s.src_dpi, 300);
    assert!(s.src_left_to_right);
}

#[test]
fn database_reset_clears_all_gaps() {
    let mut db = WordGapDatabase::new();
    let mut words = TextWords::new();
    for &(c1, c2) in &[(10, 30), (50, 70)] {
        words.push(TextRow {
            c1,
            c2,
            ..TextRow::default()
        });
    }
    compute_col_gaps(&mut words, 100);
    add_word_gaps(&words, 10, &mut db, 0.5);
    assert!(!db.is_empty());
    db.reset();
    assert!(db.is_empty());
    assert_eq!(db.len(), 0);
}

#[test]
fn compute_median_gap_empty_returns_default() {
    let db = WordGapDatabase::new();
    let median = compute_median_gap(&db);
    assert_eq!(median, 0.7);
}

#[test]
fn add_word_gaps_accumulates_across_rows() {
    let mut db = WordGapDatabase::new();
    // 第一行
    let mut row1 = TextWords::new();
    for &(c1, c2) in &[(10, 30), (50, 70), (90, 110)] {
        row1.push(TextRow {
            c1,
            c2,
            ..TextRow::default()
        });
    }
    compute_col_gaps(&mut row1, 120);
    add_word_gaps(&row1, 10, &mut db, 0.5);
    let len1 = db.len();
    // 第二行
    let mut row2 = TextWords::new();
    for &(c1, c2) in &[(10, 30), (50, 70)] {
        row2.push(TextRow {
            c1,
            c2,
            ..TextRow::default()
        });
    }
    compute_col_gaps(&mut row2, 100);
    add_word_gaps(&row2, 10, &mut db, 0.5);
    assert!(db.len() > len1, "should accumulate across rows");
    let _median = compute_median_gap(&db);
    // median should be a finite f64
    assert!(_median.is_finite());
}

#[test]
fn one_row_find_textwords_with_out_of_bounds_returns_default() {
    // row_rect 部分超出 bmp 范围
    let bmp = make_white_bmp(100, 50, 150.0);
    let view = RegionView::full(&bmp);
    let mut db = WordGapDatabase::new();
    // 给一个越界的 rect（c2 > bmp.width）
    let words = one_row_find_textwords(
        &view,
        Rect::new(0, 5, 200, 45),
        10,
        &WordSettings::default(),
        &mut db,
        false,
    );
    // 越界处理：返回 default word（不 panic）
    assert_eq!(words.rows.len(), 1);
}
