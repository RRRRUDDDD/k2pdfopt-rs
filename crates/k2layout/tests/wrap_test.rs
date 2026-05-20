//! `wrap_test` - Step 8.1 WrapState + WrapPipeline 集成测试。
//!
//! 覆盖：
//! - synthetic fixture：人工像素验证 add_word 拼接 / flush / hyphen erase
//! - LTR / RTL 双向
//! - 多 word 累积超出 max_region_width 触发 should_flush
//! - flush 产出的 FlushedLine 字段语义
//! - WrapPipeline 高层 API（add_word + flush 循环）
//!
//! C 对照：`wrapbmp.c::wrapbmp_add` (125-383) / `wrapbmp_flush` (386-576) /
//! `wrapbmp_hyphen_erase` (579-648)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_range_loop)]

use k2layout::wrap::{WrapPipeline, WrapPipelineSettings};
use k2layout::{AddRegion, HyphenInfo, MasterGapCarry, WRectMap, WrapState};
use k2types::{Bitmap, PixelFormat};

/// 帮助器：构造一个简单的 word region（灰度图，整行连续色块）。
fn synthetic_word<'a>(
    pixels: &'a [u8],
    full_width: u32,
    full_height: u32,
    c1: i32,
    c2: i32,
    r1: i32,
    r2: i32,
    rowbase: i32,
) -> AddRegion<'a> {
    AddRegion {
        pixels,
        src_full_width: full_width,
        src_full_height: full_height,
        format: PixelFormat::Gray8,
        c1,
        c2,
        r1,
        r2,
        rowbase,
        rowheight: (r2 - r1 + 1) + 4,
        gap: 2,
        gapblank: 1,
        bgcolor: 255,
        pageno: 0,
        dpi: 300.0,
        rotdeg: 0,
        hyphen: HyphenInfo::none(),
    }
}

/// 在指定 src bitmap 的 (c1..=c2, r1..=r2) 区域填入色值。
fn paint_region(buf: &mut [u8], full_width: u32, c1: i32, c2: i32, r1: i32, r2: i32, value: u8) {
    let fw = full_width as i32;
    for r in r1..=r2 {
        for c in c1..=c2 {
            let idx = (r * fw + c) as usize;
            if idx < buf.len() {
                buf[idx] = value;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// add_word: 首次 + 拼接
// ---------------------------------------------------------------------------

#[test]
fn add_word_first_call_blits_region_pixels() {
    let mut w = WrapState::new();
    w.set_color(false);
    // 8x6 src, 在 (2..=5, 1..=3) 涂值 50
    let mut src = vec![255u8; 48];
    paint_region(&mut src, 8, 2, 5, 1, 3, 50);
    let reg = synthetic_word(&src, 8, 6, 2, 5, 1, 3, 2);
    w.add_word(&reg, 0, 0x88, true, 0, 0.0).unwrap();
    let bmp = w.bitmap.as_ref().unwrap();
    assert_eq!(bmp.width, 4);
    // rh = rowbase-r1+1 = 2-1+1 = 2; th = rh + (r2-rowbase) = 2 + 1 = 3
    assert_eq!(bmp.height, 3);
    assert_eq!(w.base, 1);
    // 像素应该被复制：region row r=1 -> dst row (base + (r-rowbase)) = 1 + (1-2) = 0
    // dst row 0 应有 50（从 src row 1）
    assert_eq!(bmp.pixels[0..4], [50, 50, 50, 50]);
}

#[test]
fn add_word_ltr_concat_appends_right() {
    let mut w = WrapState::new();
    w.set_color(false);
    // 第一个 word：5x4 全 100，c=0..=2, r=0..=2, rowbase=1
    let src1 = vec![100u8; 20];
    let r1 = synthetic_word(&src1, 5, 4, 0, 2, 0, 2, 1);
    w.add_word(&r1, 0, 0x88, true, 0, 0.0).unwrap();
    // 第二个 word：同样大小，但内容 50
    let src2 = vec![50u8; 20];
    let r2 = synthetic_word(&src2, 5, 4, 0, 2, 0, 2, 1);
    w.add_word(&r2, 2, 0x88, true, 0, 0.0).unwrap();
    let bmp = w.bitmap.as_ref().unwrap();
    // 拼接：3 + 2 (colgap) + 3 = 8
    assert_eq!(bmp.width, 8);
    assert_eq!(bmp.height, 3);
    // 第 0 列应为 100（旧 wrap 内容），第 5 列应为 50（新拼入）
    // dst row 0 (= base 0 + r=0 - cur_base 1 = -1? 不对)
    // 重新算：旧 wrap bitmap 3x3，第二次 add 时 cur_base=1, new_base=1, dy=0
    // 旧 wrap row i -> tmp row (i + new_base - cur_base) = i
    // 旧 wrap[0][0..3] = 100, tmp[0][0..3] = 100
    // region row r=0 -> tmp row (0 + new_base - rowbase) = 0 + 1 - 1 = 0
    // tmp[0][5..8] = src2[0][0..3] = 50
    assert_eq!(bmp.pixels[0..3], [100, 100, 100]);
    assert_eq!(bmp.pixels[5..8], [50, 50, 50]);
    // 中间 colgap 区是 255
    assert_eq!(bmp.pixels[3], 255);
    assert_eq!(bmp.pixels[4], 255);
}

#[test]
fn add_word_rtl_concat_appends_left() {
    let mut w = WrapState::new();
    w.set_color(false);
    let src1 = vec![100u8; 20];
    let r1 = synthetic_word(&src1, 5, 4, 0, 2, 0, 2, 1);
    // RTL：src_left_to_right=false
    w.add_word(&r1, 0, 0x88, false, 0, 0.0).unwrap();
    let src2 = vec![50u8; 20];
    let r2 = synthetic_word(&src2, 5, 4, 0, 2, 0, 2, 1);
    w.add_word(&r2, 2, 0x88, false, 0, 0.0).unwrap();
    let bmp = w.bitmap.as_ref().unwrap();
    assert_eq!(bmp.width, 8);
    // RTL：旧 wrap 落在右侧 (width0 + colgap = 5 偏移)，新 region 落在左侧 [0..3]
    // tmp[0][0..3] = 50（新），tmp[0][5..8] = 100（旧）
    assert_eq!(bmp.pixels[0..3], [50, 50, 50]);
    assert_eq!(bmp.pixels[5..8], [100, 100, 100]);
}

#[test]
fn add_word_carry_absorbed_only_on_first_call() {
    let mut w = WrapState::new();
    w.set_color(false);
    let src = vec![100u8; 20];
    let r = synthetic_word(&src, 5, 4, 0, 2, 0, 2, 1);
    let o1 = w.add_word(&r, 0, 0x88, true, 7, 0.5).unwrap();
    assert_eq!(o1, MasterGapCarry::Absorbed);
    assert_eq!(w.mandatory_region_gap, 7);
    let o2 = w.add_word(&r, 1, 0x88, true, 99, 9.9).unwrap();
    assert_eq!(o2, MasterGapCarry::NotChanged);
    assert_eq!(w.mandatory_region_gap, 7); // 未被覆盖
}

// ---------------------------------------------------------------------------
// flush: 产出 + 重置
// ---------------------------------------------------------------------------

#[test]
fn flush_returns_flushed_line_with_bitmap() {
    let mut w = WrapState::new();
    w.set_color(false);
    let src = vec![100u8; 20];
    let r = synthetic_word(&src, 5, 4, 0, 2, 0, 2, 1);
    w.add_word(&r, 0, 0x88, true, 5, 0.5).unwrap();
    let line = w.flush(true, true).unwrap().expect("Some line");
    assert_eq!(line.bitmap.width, 3);
    assert_eq!(line.bitmap.height, 3);
    assert_eq!(line.base, 1);
    assert_eq!(line.mandatory_region_gap, 5);
    assert!((line.page_region_gap_in - 0.5).abs() < 1e-9);
    // 已 reset
    assert!(w.is_empty());
}

#[test]
fn flush_disables_full_justification() {
    let mut w = WrapState::new();
    w.set_color(false);
    let src = vec![100u8; 20];
    let r = synthetic_word(&src, 5, 4, 0, 2, 0, 2, 1);
    w.add_word(&r, 0, 0xff, true, 0, 0.0).unwrap();
    let line = w.flush(true, false).unwrap().unwrap();
    assert_eq!(line.just_flags, 0xef);
}

#[test]
fn flush_empty_returns_none_and_marks_flushed() {
    let mut w = WrapState::new();
    let res = w.flush(true, true).unwrap();
    assert!(res.is_none());
    assert_eq!(w.just_flushed_internal, 1);
}

// ---------------------------------------------------------------------------
// hyphen_erase
// ---------------------------------------------------------------------------

#[test]
fn hyphen_erase_ltr_truncates_to_c2_and_whites_hyphen() {
    let mut w = WrapState::new();
    let mut pixels = vec![100u8; 40]; // 10x4
                                      // 在 hyphen 行 1..=2，列 6..=8 涂 200（模拟 hyphen 像素）
    paint_region(&mut pixels, 10, 6, 8, 1, 2, 200);
    let bmp = Bitmap::from_raw(10, 4, 1.0, PixelFormat::Gray8, pixels).unwrap();
    w.bitmap = Some(bmp);
    w.hyphen = HyphenInfo {
        ch: 6,
        c2: 8,
        r1: 1,
        r2: 2,
    };
    w.wrectmaps.add(WRectMap::new());
    w.hyphen_erase(true).unwrap();
    let new_bmp = w.bitmap.as_ref().unwrap();
    // new_width = c2+1 = 9
    assert_eq!(new_bmp.width, 9);
    // hyphen 段 [ch=6 .. c2=8] 在行 1..=2 应是 255
    for r in 1..=2 {
        for c in 6..=8 {
            assert_eq!(new_bmp.pixels[(r * 9 + c) as usize], 255);
        }
    }
    assert!(!w.ends_in_hyphen());
}

#[test]
fn hyphen_erase_rtl_trims_left_and_keeps_right() {
    let mut w = WrapState::new();
    let mut pixels = vec![100u8; 40]; // 10x4
    paint_region(&mut pixels, 10, 1, 3, 1, 2, 200);
    let bmp = Bitmap::from_raw(10, 4, 1.0, PixelFormat::Gray8, pixels).unwrap();
    w.bitmap = Some(bmp);
    w.hyphen = HyphenInfo {
        ch: 3,
        c2: 1,
        r1: 1,
        r2: 2,
    };
    w.wrectmaps.add(WRectMap::new());
    w.hyphen_erase(false).unwrap();
    let new_bmp = w.bitmap.as_ref().unwrap();
    // RTL: new_w = cur_w - c2 = 10 - 1 = 9
    assert_eq!(new_bmp.width, 9);
    assert!(!w.ends_in_hyphen());
}

// ---------------------------------------------------------------------------
// WrapPipeline
// ---------------------------------------------------------------------------

#[test]
fn pipeline_flush_after_max_width_exceed() {
    let settings = WrapPipelineSettings {
        text_wrap: true,
        max_region_width_inches: 1.0,
        src_dpi: 100.0, // max_pix = 100
        src_left_to_right: true,
        allow_full_justification: true,
    };
    let mut p = WrapPipeline::new(settings, false);
    // 累加多个 30 像素宽的 word，到第 4 次时累计 > 100
    let src = vec![100u8; 30 * 4];
    let r = synthetic_word(&src, 30, 4, 0, 29, 0, 2, 1);
    let o1 = p.add_word(&r, 0, 0x88, 0, 0.0).unwrap();
    assert!(!o1.should_flush);
    let o2 = p.add_word(&r, 2, 0x88, 0, 0.0).unwrap();
    // 累计 30 + 2 + 30 = 62, remaining = 100 - 62 = 38 > 0
    assert!(!o2.should_flush);
    let o3 = p.add_word(&r, 2, 0x88, 0, 0.0).unwrap();
    // 累计 62 + 2 + 30 = 94, remaining = 6 > 0
    assert!(!o3.should_flush);
    let o4 = p.add_word(&r, 2, 0x88, 0, 0.0).unwrap();
    // 累计 94 + 2 + 30 = 126, remaining = -26 <= 0
    assert!(o4.should_flush);
    let line = p.flush().unwrap().expect("Some line");
    assert!(line.bitmap.width >= 100);
}

#[test]
fn pipeline_multiple_lines_cycle() {
    let mut p = WrapPipeline::new(WrapPipelineSettings::default(), false);
    let src = vec![100u8; 20];
    let r = synthetic_word(&src, 5, 4, 0, 2, 0, 2, 1);

    // Line 1
    p.add_word(&r, 0, 0x88, 0, 0.0).unwrap();
    let line1 = p.flush().unwrap().expect("line1");
    assert_eq!(line1.bitmap.width, 3);
    assert!(p.is_empty());

    // Line 2（reset 后状态干净，能再用）
    p.add_word(&r, 0, 0x88, 3, 0.7).unwrap();
    let line2 = p.flush().unwrap().expect("line2");
    assert_eq!(line2.bitmap.width, 3);
    assert_eq!(line2.mandatory_region_gap, 3);
    assert!((line2.page_region_gap_in - 0.7).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// is_empty / reset
// ---------------------------------------------------------------------------

#[test]
fn empty_state_is_empty_true() {
    let w = WrapState::new();
    assert!(w.is_empty());
}

#[test]
fn after_reset_is_empty_true() {
    let mut w = WrapState::new();
    w.set_color(false);
    let src = vec![100u8; 20];
    let r = synthetic_word(&src, 5, 4, 0, 2, 0, 2, 1);
    w.add_word(&r, 0, 0x88, true, 0, 0.0).unwrap();
    assert!(!w.is_empty());
    w.reset();
    assert!(w.is_empty());
    assert_eq!(w.rhmax, -1);
    assert_eq!(w.thmax, -1);
    assert_eq!(w.just_flushed_internal, 1);
}

// ---------------------------------------------------------------------------
// remaining
// ---------------------------------------------------------------------------

#[test]
fn remaining_decreases_as_words_accumulate() {
    let settings = WrapPipelineSettings {
        text_wrap: true,
        max_region_width_inches: 2.0,
        src_dpi: 100.0, // max_pix = 200
        src_left_to_right: true,
        allow_full_justification: true,
    };
    let mut p = WrapPipeline::new(settings, false);
    let src = vec![100u8; 80];
    let r = synthetic_word(&src, 20, 4, 0, 19, 0, 2, 1);
    p.add_word(&r, 0, 0x88, 0, 0.0).unwrap();
    assert_eq!(p.state.remaining(2.0, 100.0, true), 200 - 20);
    p.add_word(&r, 5, 0x88, 0, 0.0).unwrap();
    assert_eq!(p.state.remaining(2.0, 100.0, true), 200 - (20 + 5 + 20));
}
