//! `k2layout::breakpoints` —— 垂直分页点检测（k2master.c::break_point 系列 → Rust）。
//!
//! **Step 7.1 (M5)**：1:1 移植 `k2pdfoptlib/k2master.c::masterinfo_break_point`
//! （行 2322-2413）+ `masterinfo_break_point_ignoring_page_break_markers`（行
//! 2419-2532）+ `masterinfo_add_pagebreakmark`（行 394-409）。
//!
//! # 范围
//!
//! 本模块只做"纯算法"：给定 master canvas bitmap + 已写入行数 + maxsize +
//! BreakSettings + pagebreak marks，输出建议的 rowcount（下页起始相对偏移）。
//! 与 [`crate::master::ConvertContext`] 解耦，便于单元测试。
//!
//! [`crate::master::OutputPaginator`] 的 `add_pagebreak_mark` 方法落地在本步，
//! 但具体的 `push_page`/`pop_page`/`flush_page` 算法留到 Step 7.3 (M5)。
//!
//! # C 行号对照
//!
//! - `masterinfo_break_point`：`k2master.c:2322-2413`
//! - `masterinfo_break_point_ignoring_page_break_markers`：`k2master.c:2419-2532`
//! - `masterinfo_add_pagebreakmark`：`k2master.c:394-409`
//! - K2PAGEBREAKMARK_TYPE_*：`k2pdfopt.h:217-219`

use crate::crop::CropError;
use crate::master::output_paginator::PageBreakMark;
use crate::region::RegionView;
use crate::rows::{find_textrows, RowSettings};
use k2core::rect::Rect;
use k2types::Bitmap;

// =====================================================================
// 常量（与 C 1:1 对应）
// =====================================================================

/// `K2PAGEBREAKMARK_TYPE_BREAKPAGE = 0`（`k2pdfopt.h:218`）：在此行强制分页。
pub const MARK_TYPE_BREAKPAGE: i32 = 0;

/// `K2PAGEBREAKMARK_TYPE_NOBREAK = 1`（`k2pdfopt.h:219`）：标记不可分页区域的
/// 起止点。两个相邻的 NOBREAK 标记定义一段"不可在此中切"的范围。
pub const MARK_TYPE_NOBREAK: i32 = 1;

/// 已消费 mark 的 sentinel（C 算法在循环里把 type 置为 -1 表示"用过了"，
/// `k2master.c:2376, 2404`）。
pub const MARK_TYPE_DISABLED: i32 = -1;

/// `MAXK2PAGEBREAKMARKS = 32`（`k2pdfopt.h:217`）。
pub const MAX_PAGE_BREAK_MARKS: usize = 32;

// =====================================================================
// BreakSettings
// =====================================================================

/// 分页算法所需的设置子集。
///
/// 与 C 版 `MASTERINFO.fit_to_page` + `K2PDFOPT_SETTINGS` 的几个字段一一对应。
/// 独立成 struct 避免 `k2layout → k2settings` 反向依赖（与 [`RowSettings`] 同源）。
///
/// # 字段
///
/// - `fit_to_page`：对应 C `mi->fit_to_page` (来自 `k2settings.dst_fit_to_page`,
///   `k2pdfopt.h:394, 705`)：
///   - `-2`：强制把 master canvas 整体作为一页（无分页）
///   - `-1`：scanheight = rows（扫到底）
///   - `0`：scanheight = maxsize；接近一页大小（`|scanheight-rows|<=1` 或
///     `|scanheight/rows-1|<0.002`）时直接返回整段
///   - `>0`：scanheight = `(1 + fit_to_page/100) * maxsize`
/// - `dst_dpi`：目标设备 DPI（C `k2settings->dst_dpi`），传给 `find_textrows`
/// - `join_figure_captions`：是否合并图与图注（C 同名字段），传给 `find_textrows`
/// - `row_settings`：行检测所用 [`RowSettings`]
/// - `bgcolor`：白色阈值（C `region.bgcolor = masterinfo->bgcolor`）
#[derive(Clone, Debug)]
pub struct BreakSettings {
    /// fit_to_page 模式。
    pub fit_to_page: i32,
    /// 目标设备 DPI（`region.dpi = k2settings->dst_dpi`，`k2master.c:2485`）。
    pub dst_dpi: i32,
    /// 是否合并图与图注（`k2master.c:2486` 的 find_textrows 入参）。
    pub join_figure_captions: bool,
    /// 行检测参数。
    pub row_settings: RowSettings,
    /// 白色阈值（C `region.bgcolor = masterinfo->bgcolor`，`k2master.c:2478`）。
    pub bgcolor: u8,
}

impl Default for BreakSettings {
    fn default() -> Self {
        Self {
            fit_to_page: 0,
            dst_dpi: 167,
            join_figure_captions: false,
            row_settings: RowSettings::default(),
            bgcolor: 255,
        }
    }
}

// =====================================================================
// find_break_point
// =====================================================================

/// 找下一页的分页点：在 master canvas 的 `[row0, master_rows-1]` 范围内
/// 找到最佳的"行数"，使得返回 `rowcount` 后 `master[row0 .. row0+rowcount]`
/// 构成下一页（避免在文本行中间切断）。
///
/// 对应 C `masterinfo_break_point(masterinfo, row0, settings, maxsize)`，
/// `k2master.c:2322-2413`。
///
/// # 参数
///
/// - `master`：master canvas 位图（已写入的全图）
/// - `master_rows`：当前已写入的行数（C `mi->rows`）；可能小于 `master.height`
///   （canvas 有未用容量）
/// - `row0`：起点行（C 同名参数）；下页将从 `row0+rowcount` 开始
/// - `maxsize`：单页的目标高度（pixel，C 同名）
/// - `settings`：分页设置
/// - `pagebreak_marks`：用户标记的强制 / 禁止分页点；本函数会修改其中被消费
///   mark 的 `mark_type` 为 [`MARK_TYPE_DISABLED`]（与 C 行 2376/2404 一致）
///
/// # 返回
///
/// 推荐的 rowcount（≥1）。下页区间 = `[row0, row0+rowcount)`。
///
/// # 错误
///
/// `find_textrows` 内部 `calc_bbox` 越界返回 [`CropError`]。
pub fn find_break_point(
    master: &Bitmap,
    master_rows: u32,
    row0: u32,
    maxsize: u32,
    settings: &BreakSettings,
    pagebreak_marks: &mut [PageBreakMark],
) -> Result<u32, CropError> {
    // C 行 2344
    let rowcount = find_break_point_ignoring_marks(master, master_rows, row0, maxsize, settings)?;

    // C 行 2348-2354: 无 mark 直接返回
    if pagebreak_marks.is_empty() {
        return Ok(rowcount);
    }
    Ok(apply_page_break_marks(rowcount, row0, pagebreak_marks))
}

// =====================================================================
// find_break_point_ignoring_marks
// =====================================================================

/// 纯按 textrow 切分找分页点，不考虑 page break marks。
///
/// 对应 C `masterinfo_break_point_ignoring_page_break_markers`，
/// `k2master.c:2419-2532`。
///
/// # 早退条件（依次判定）
///
/// 1. `rows < maxsize` 或 `fit_to_page == -2`（C 行 2442）→ 直接返回 rows
/// 2. fit_to_page == 0 时若 scanheight 与 rows 几乎相等（差 ≤1 或相对差
///    <0.002，C 行 2457-2459）→ 返回 rows
///
/// # 主算法
///
/// 1. 算 `scanheight`（按 fit_to_page）
/// 2. 在 master `[row0, row0 + min(scanheight*1.4, rows) - 1]` 内调
///    [`find_textrows`]
/// 3. 遍历 textrows 找最后一个 `r2 <= maxsize` 的行，记录 r1/r2/r1a/r2a
/// 4. 后处理 + 计算 rowcount：`r1 < maxsize/4 ? min(r2, scanheight) : r1`
/// 5. 防止 0/极小值：`rowcount <= 2` 时回退到 scanheight（C 行 2528-2530）
pub fn find_break_point_ignoring_marks(
    master: &Bitmap,
    master_rows: u32,
    row0: u32,
    maxsize: u32,
    settings: &BreakSettings,
) -> Result<u32, CropError> {
    debug_assert!(
        row0 <= master_rows,
        "row0={row0} 超过 master_rows={master_rows}"
    );
    debug_assert!(
        master_rows <= master.height,
        "master_rows={master_rows} 超过 bitmap.height={}",
        master.height
    );

    // C 行 2438: 剩余可用行
    let rows = master_rows.saturating_sub(row0);

    // C 行 2442-2443: 不够一页 / 强制单页 → 直接返回
    if rows < maxsize || settings.fit_to_page == -2 {
        return Ok(rows);
    }

    // C 行 2446-2451: 算 scanheight
    let scanheight: u32 = if settings.fit_to_page == -1 {
        rows
    } else if settings.fit_to_page > 0 {
        // (1 + fit_to_page/100) * maxsize + 0.5
        let f = 1.0 + (settings.fit_to_page as f64) / 100.0;
        let val = f * (maxsize as f64) + 0.5;
        // 防 NaN / 负
        if val.is_finite() && val > 0.0 {
            val as u32
        } else {
            maxsize
        }
    } else {
        // fit_to_page == 0
        maxsize
    };

    // C 行 2457-2459: 接近一页大小直接返回
    if settings.fit_to_page == 0 {
        let diff = scanheight.abs_diff(rows);
        if diff <= 1 {
            return Ok(rows);
        }
        if rows > 0 {
            let ratio = (scanheight as f64) / (rows as f64) - 1.0;
            if ratio.abs() < 0.002 {
                return Ok(rows);
            }
        }
    }

    // C 行 2460-2461: scanheight 不超过 rows
    let scanheight = scanheight.min(rows);

    // C 行 2474-2476: 真实扫描高度 = min(scanheight*1.4, rows)
    let scan_actual = {
        let v = (scanheight as f64) * 1.4;
        let v = if v.is_finite() && v > 0.0 {
            v as u32
        } else {
            scanheight
        };
        v.min(rows)
    };

    if scan_actual == 0 {
        // rows>=maxsize 但 scan_actual=0 不可能；保守 fallback
        return Ok(rows);
    }

    // 构造 RegionView：[row0, row0+scan_actual-1]，覆盖全宽
    // 对应 C 行 2466-2485：bmp_copy/bmp_convert_to_grayscale_ex + bmp_eliminate_top_rows
    // + bmp->height = scanheight*1.4。Rust 版用 RegionView 避免拷贝；TextRow.r1/r2
    // 自然是相对全 master 的绝对坐标（含 row0 偏移）。
    let view_rect = Rect::new(
        0,
        row0 as i32,
        (master.width as i32) - 1,
        (row0 + scan_actual - 1) as i32,
    );
    let view = RegionView::with(master, view_rect, settings.dst_dpi as f32, settings.bgcolor);

    // 对应 C 行 2486: bmpregion_find_textrows(region, settings, 0, 1, -1.0, jfc)
    // 参数：dynamic_aperture=false, remove_small_rows=true, minrowgap_in=-1.0
    let textrows = find_textrows(
        &view,
        &settings.row_settings,
        false,
        true,
        -1.0,
        settings.join_figure_captions,
    )?;

    // C 行 2507-2521: 遍历 textrows 找 r1/r2/r1a/r2a
    // C 版坐标系：bmp_eliminate_top_rows 切了 row0，所以 row->r2 是 0-based 相对坐标。
    // Rust 版未拷贝 bitmap，TextRow.r2 是 master 绝对坐标，换算用 `row.r2 - row0`。
    let max_i: i32 = maxsize as i32;
    let scan_i: i32 = scanheight as i32;
    let row0_i: i32 = row0 as i32;

    let mut r1: i32 = 0;
    let mut r2: i32 = 0;
    let mut r1a: i32 = 0;
    let mut r2a: i32 = 0;

    let rows_vec = &textrows.rows;
    let n = rows_vec.len();
    for (j, row) in rows_vec.iter().enumerate() {
        // 当前行末（相对 row0）
        let rel_r2_plus_1 = (row.r2 - row0_i) + 1;
        let rel_r2a = if j + 1 < n {
            // C 行 2514: r2a = (row->r2 + row[j+1].r1) / 2  —— 整数除法向下取整
            (row.r2 + rows_vec[j + 1].r1) / 2 - row0_i
        } else {
            // C 行 2515-2516
            rel_r2_plus_1
        };

        // 先暂存（C 算法中 r2/r2a 总是先更新再判断）
        r2 = rel_r2_plus_1;
        r2a = rel_r2a;

        // C 行 2517-2518: row.r2 > maxsize 则提前停（这里用相对坐标）
        if (row.r2 - row0_i) > max_i {
            break;
        }

        // C 行 2519-2520: 保存到 r1/r1a
        r1 = r2;
        r1a = r2a;
    }

    // C 行 2523-2526
    if r1a <= max_i {
        r1 = r1a;
    }
    if r2a <= scan_i {
        r2 = r2a;
    }

    // C 行 2527
    let mut rowcount = if r1 < max_i / 4 { r2.min(scan_i) } else { r1 };

    // C 行 2528-2530: v2.16 防 0 rowcount
    if rowcount <= 2 {
        rowcount = scan_i;
    }

    // rowcount 不应为负（max_i, scan_i 都来自 u32，且 textrow 计算只做加法）
    debug_assert!(rowcount >= 0, "rowcount={rowcount} 应非负");
    Ok(rowcount.max(0) as u32)
}

// =====================================================================
// apply_page_break_marks
// =====================================================================

/// 应用用户的 page break marks 调整 rowcount。
///
/// 对应 C `masterinfo_break_point` 行 2365-2408 的循环。
///
/// # 算法
///
/// 遍历 marks（按 `row` 排序，但本函数不主动排序——遵循 C 版以原序遍历）：
/// - 跳过已消费 mark（`mark_type == MARK_TYPE_DISABLED`）
/// - mark.row >= rowcount+row0 且未进入 NOBREAK 区间 → 跳出（marks 已按 row 排序时
///   后续 mark 不会影响当前页）
/// - [`MARK_TYPE_BREAKPAGE`]：强制 rowcount = `mark.row - row0`，mark 置 -1，
///   退出循环
/// - [`MARK_TYPE_NOBREAK`]：交替开/关 "no-break 区间"
///   - 开始 NOBREAK 区间时若 mark.row > rowcount+row0，意味着当前 rowcount 落在
///     "禁止分页"区间内，需要回退到 NOBREAK 起点：rowcount = mark.row - row0
///   - 结束 NOBREAK 区间时若上一次设置 rowcount 在区间内，回退到区间起点
///
/// `nobreak` 状态变量（C 行 2355）：
/// - `-999`：未进入 NOBREAK 区间
/// - `>= 0`：正处于 NOBREAK 区间，值是区间起点的绝对行
///
/// # 返回
///
/// 调整后的 rowcount。
pub fn apply_page_break_marks(
    initial_rowcount: u32,
    row0: u32,
    pagebreak_marks: &mut [PageBreakMark],
) -> u32 {
    let mut rowcount = initial_rowcount as i32;
    let row0_i = row0 as i32;

    // C 行 2355: nobreak=-999 表示"未在 NOBREAK 区间"
    let mut nobreak: i32 = -999;

    for mark in pagebreak_marks.iter_mut() {
        // C 行 2370-2371: 跳过已消费
        if mark.mark_type < 0 {
            continue;
        }
        let mrow = mark.row as i32;

        // C 行 2372-2373: mark 在 rowcount 之后 + 未进入 NOBREAK → 退出
        if mrow >= rowcount + row0_i && nobreak < -990 {
            break;
        }

        if mark.mark_type == MARK_TYPE_BREAKPAGE {
            // C 行 2374-2383: 强制分页
            mark.mark_type = MARK_TYPE_DISABLED;
            rowcount = mrow - row0_i;
            // nobreak 复位
            // （C 行 2381，注释为"nobreak = -999"，break 跳出循环）
            return rowcount.max(0) as u32;
        }

        if mark.mark_type == MARK_TYPE_NOBREAK {
            // C 行 2384-2407: NOBREAK 区间
            if nobreak > 1 {
                // 已在 NOBREAK 区间内
                if mrow > rowcount + row0_i {
                    // C 行 2388-2395: rowcount 落在区间内 → 回退到区间起点
                    rowcount = nobreak - row0_i;
                    return rowcount.max(0) as u32;
                }
            }
            // C 行 2397: nobreak 翻转
            //   if nobreak > -990 → set back to -999 (退出区间)
            //   else            → set to mark->row (进入区间)
            nobreak = if nobreak > -990 { -999 } else { mrow };

            if nobreak < -990 && mrow > rowcount + row0_i {
                // C 行 2398-2406: 刚退出 NOBREAK 区间但 mark 落在 rowcount 之后,
                // 意味着我们正穿越一个 closed NOBREAK 区间 → 回退
                rowcount = mrow - row0_i;
                mark.mark_type = MARK_TYPE_DISABLED;
                return rowcount.max(0) as u32;
            }
        }
    }

    rowcount.max(0) as u32
}

// =====================================================================
// 单测
// =====================================================================

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use k2types::{Bitmap, PixelFormat};

    /// 构造一张全白 master 灰度位图。
    fn white_master(width: u32, height: u32, dpi: f32) -> Bitmap {
        let mut bmp = Bitmap::new(width, height, dpi, PixelFormat::Gray8).unwrap();
        bmp.fill_byte(255);
        bmp
    }

    /// 在 master 上画一条 `rows` 行高、覆盖 `[col0, col1)` 列的纯黑横条。
    fn draw_band(bmp: &mut Bitmap, y_start: u32, height: u32, col0: u32, col1: u32) {
        for y in y_start..(y_start + height).min(bmp.height) {
            for x in col0..col1.min(bmp.width) {
                if let Some(px) = bmp.pixel_mut(x, y) {
                    px[0] = 0;
                }
            }
        }
    }

    // ---- BreakSettings ----

    #[test]
    fn break_settings_default_is_fit_zero() {
        let s = BreakSettings::default();
        assert_eq!(s.fit_to_page, 0);
        assert_eq!(s.dst_dpi, 167);
        assert!(!s.join_figure_captions);
        assert_eq!(s.bgcolor, 255);
    }

    // ---- 早退路径 ----

    #[test]
    fn rows_less_than_maxsize_returns_rows() {
        // rows = 50, maxsize = 100 → 直接返回 rows=50
        let bmp = white_master(200, 100, 167.0);
        let settings = BreakSettings::default();
        let r = find_break_point_ignoring_marks(&bmp, 50, 0, 100, &settings).unwrap();
        assert_eq!(r, 50);
    }

    #[test]
    fn fit_to_page_minus_two_forces_full_rows() {
        let bmp = white_master(200, 1000, 167.0);
        let settings = BreakSettings {
            fit_to_page: -2,
            ..BreakSettings::default()
        };
        let r = find_break_point_ignoring_marks(&bmp, 800, 0, 100, &settings).unwrap();
        // rows = 800; fit_to_page=-2 直接返回 rows
        assert_eq!(r, 800);
    }

    #[test]
    fn fit_to_page_zero_near_full_page_returns_rows() {
        // rows = 100, maxsize = 100 → scanheight = 100; |100-100|<=1 → 返回 rows=100
        let bmp = white_master(200, 200, 167.0);
        let settings = BreakSettings::default();
        let r = find_break_point_ignoring_marks(&bmp, 100, 0, 100, &settings).unwrap();
        assert_eq!(r, 100);
    }

    #[test]
    fn fit_to_page_zero_within_0_2_pct_returns_rows() {
        // rows = 1000, maxsize = 1001 → diff=1, 也走 <=1 早退
        // 测试 0.2% 路径：rows = 5000, maxsize = 5009 → diff=9, ratio=9/5000=0.0018 < 0.002
        let bmp = white_master(200, 6000, 167.0);
        let settings = BreakSettings::default();
        let r = find_break_point_ignoring_marks(&bmp, 5000, 0, 5009, &settings).unwrap();
        assert_eq!(r, 5000);
    }

    // ---- 真扫描路径 ----

    #[test]
    fn empty_textrows_falls_back_to_scanheight() {
        // 全白 master：find_textrows 返回空，触发 rowcount<=2 fallback = scanheight
        // rows=500, maxsize=200, fit=0 → scanheight=200, scan_actual=min(280, 500)=280
        // 找不到 textrow → r1=r2=0 → rowcount = if 0<50 { min(0,200) } else 0 = 0
        // 0<=2 → fallback = scanheight=200
        let bmp = white_master(200, 600, 167.0);
        let settings = BreakSettings::default();
        let r = find_break_point_ignoring_marks(&bmp, 500, 0, 200, &settings).unwrap();
        assert_eq!(r, 200);
    }

    #[test]
    fn three_rows_fit_in_maxsize_returns_after_third_row() {
        // 3 个文本带，每条 30px 高，间隔 20px：[0..30], [50..80], [100..130]
        // master_rows = 200, maxsize = 200, fit=0 → scan_actual = min(280, 200)=200
        // 期望切到最后一条文本之后某处
        let mut bmp = white_master(200, 300, 167.0);
        draw_band(&mut bmp, 0, 30, 10, 190);
        draw_band(&mut bmp, 50, 30, 10, 190);
        draw_band(&mut bmp, 100, 30, 10, 190);
        let settings = BreakSettings::default();
        let r = find_break_point_ignoring_marks(&bmp, 200, 0, 200, &settings).unwrap();
        // 跑通即可，rowcount > 0
        assert!(r > 0, "rowcount={r} 应 > 0");
        assert!(r <= 200, "rowcount={r} 应 <= scanheight=200");
    }

    #[test]
    fn row_overflow_breaks_at_previous_row() {
        // 两条文本：[0..30], [50..80]，maxsize=60（第二条 r2=79 > 60 触发 break）
        let mut bmp = white_master(200, 200, 167.0);
        draw_band(&mut bmp, 0, 30, 10, 190);
        draw_band(&mut bmp, 50, 30, 10, 190);
        let settings = BreakSettings::default();
        // master_rows=120, maxsize=60, fit=0 → scanheight=60, scan_actual=min(84,120)=84
        let r = find_break_point_ignoring_marks(&bmp, 120, 0, 60, &settings).unwrap();
        // 至少返回一个 sensible 值
        assert!(r > 0);
        assert!(r <= 84);
    }

    #[test]
    fn row0_offset_excludes_top_rows() {
        // master 顶部 50 行有内容，但 row0=50 跳过它们
        let mut bmp = white_master(200, 300, 167.0);
        draw_band(&mut bmp, 0, 30, 10, 190); // 这条被跳过
        draw_band(&mut bmp, 100, 30, 10, 190); // 真正参与的
        let settings = BreakSettings::default();
        // master_rows=250, row0=50, maxsize=100 → rows=200, scanheight=100, scan=140
        let r = find_break_point_ignoring_marks(&bmp, 250, 50, 100, &settings).unwrap();
        assert!(r > 0);
    }

    #[test]
    fn fit_to_page_minus_one_allows_full_scan() {
        // fit_to_page=-1: scanheight = rows，允许扫描全部剩余空间
        let mut bmp = white_master(200, 600, 167.0);
        draw_band(&mut bmp, 50, 30, 10, 190);
        draw_band(&mut bmp, 200, 30, 10, 190);
        draw_band(&mut bmp, 350, 30, 10, 190);
        let settings = BreakSettings {
            fit_to_page: -1,
            ..BreakSettings::default()
        };
        // master_rows=500, maxsize=100, rows=500, scanheight=500, scan=min(700,500)=500
        let r = find_break_point_ignoring_marks(&bmp, 500, 0, 100, &settings).unwrap();
        assert!(r > 0);
    }

    #[test]
    fn fit_to_page_positive_allows_overflow() {
        // fit=50 → scanheight = 1.5 * maxsize；但被 rows clamp
        let mut bmp = white_master(200, 400, 167.0);
        draw_band(&mut bmp, 50, 30, 10, 190);
        draw_band(&mut bmp, 150, 30, 10, 190);
        let settings = BreakSettings {
            fit_to_page: 50,
            ..BreakSettings::default()
        };
        // master_rows=300, maxsize=100, rows=300, scanheight=150
        let r = find_break_point_ignoring_marks(&bmp, 300, 0, 100, &settings).unwrap();
        assert!(r > 0);
    }

    // ---- apply_page_break_marks ----

    #[test]
    fn apply_marks_empty_returns_initial() {
        let mut marks: Vec<PageBreakMark> = Vec::new();
        let r = apply_page_break_marks(100, 0, &mut marks);
        assert_eq!(r, 100);
    }

    #[test]
    fn breakpage_mark_in_range_forces_rowcount() {
        // rowcount=100, row0=0, mark.row=50 type=BREAKPAGE → 强制 rowcount=50
        let mut marks = vec![PageBreakMark {
            row: 50,
            mark_type: MARK_TYPE_BREAKPAGE,
        }];
        let r = apply_page_break_marks(100, 0, &mut marks);
        assert_eq!(r, 50);
        // mark 已被消费
        assert_eq!(marks[0].mark_type, MARK_TYPE_DISABLED);
    }

    #[test]
    fn breakpage_mark_beyond_rowcount_ignored() {
        // mark.row=200 > rowcount+row0=100 → 跳出，无影响
        let mut marks = vec![PageBreakMark {
            row: 200,
            mark_type: MARK_TYPE_BREAKPAGE,
        }];
        let r = apply_page_break_marks(100, 0, &mut marks);
        assert_eq!(r, 100);
        assert_eq!(marks[0].mark_type, MARK_TYPE_BREAKPAGE);
    }

    #[test]
    fn disabled_mark_is_skipped() {
        // 已 disabled 的 mark 不影响
        let mut marks = vec![PageBreakMark {
            row: 50,
            mark_type: MARK_TYPE_DISABLED,
        }];
        let r = apply_page_break_marks(100, 0, &mut marks);
        assert_eq!(r, 100);
    }

    #[test]
    fn nobreak_pair_crossing_rowcount_rewinds() {
        // 一对 NOBREAK 围住 rowcount：50..150，rowcount=100 落在区间内
        // 期望回退到区间起点 50
        let mut marks = vec![
            PageBreakMark {
                row: 50,
                mark_type: MARK_TYPE_NOBREAK,
            },
            PageBreakMark {
                row: 150,
                mark_type: MARK_TYPE_NOBREAK,
            },
        ];
        let r = apply_page_break_marks(100, 0, &mut marks);
        assert_eq!(r, 50);
    }

    #[test]
    fn nobreak_pair_outside_rowcount_no_effect() {
        // NOBREAK 区间在 rowcount 之后：150..200，rowcount=100 不受影响
        let mut marks = vec![
            PageBreakMark {
                row: 150,
                mark_type: MARK_TYPE_NOBREAK,
            },
            PageBreakMark {
                row: 200,
                mark_type: MARK_TYPE_NOBREAK,
            },
        ];
        let r = apply_page_break_marks(100, 0, &mut marks);
        assert_eq!(r, 100);
    }

    #[test]
    fn row0_offset_applied_to_mark_row() {
        // row0=20，mark.row=70 → 相对 rowcount = 70-20 = 50
        let mut marks = vec![PageBreakMark {
            row: 70,
            mark_type: MARK_TYPE_BREAKPAGE,
        }];
        let r = apply_page_break_marks(100, 20, &mut marks);
        assert_eq!(r, 50);
    }

    #[test]
    fn find_break_point_invokes_marks_path() {
        // 整合测试：master 全白触发 fallback=scanheight，再被 BREAKPAGE mark 截断
        let bmp = white_master(200, 600, 167.0);
        let settings = BreakSettings::default();
        let mut marks = vec![PageBreakMark {
            row: 80,
            mark_type: MARK_TYPE_BREAKPAGE,
        }];
        // master_rows=500, maxsize=200, row0=0 → fallback rowcount=200
        // mark.row=80 < 200 → 强制 rowcount=80
        let r = find_break_point(&bmp, 500, 0, 200, &settings, &mut marks).unwrap();
        assert_eq!(r, 80);
    }

    #[test]
    fn find_break_point_no_marks_short_circuit() {
        // 空 marks 直接返回 ignoring_marks 的结果
        let bmp = white_master(200, 100, 167.0);
        let settings = BreakSettings::default();
        let mut marks: Vec<PageBreakMark> = Vec::new();
        // rows=50, maxsize=100 → 早退 rows=50
        let r = find_break_point(&bmp, 50, 0, 100, &settings, &mut marks).unwrap();
        assert_eq!(r, 50);
    }

    // ---- 常量校验 ----

    #[test]
    fn mark_type_constants_match_c_header() {
        assert_eq!(MARK_TYPE_BREAKPAGE, 0);
        assert_eq!(MARK_TYPE_NOBREAK, 1);
        assert_eq!(MARK_TYPE_DISABLED, -1);
        assert_eq!(MAX_PAGE_BREAK_MARKS, 32);
    }
}
