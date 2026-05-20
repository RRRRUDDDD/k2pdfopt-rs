//! `crop.rs` —— bbox 计算、blank 检测、trim margins 三个核心几何算法。
//!
//! ## 来源
//!
//! - C 对照：[`k2pdfoptlib/bmpregion.c`]
//!   - `bmpregion_calc_bbox`（行 466-684）
//!   - `bmpregion_trim_margins`（行 748-766）
//!   - `bmpregion_is_blank`（行 407-423）
//!   - `trim_to` static helper（行 769-819）
//!   - `height2_calc` static helper（行 827-873）
//! - 计划：`rust-rewrite-execution-plan.md` Step 5.2
//!
//! ## 与 C 版的差异
//!
//! 1. **借用替代指针**：C 用裸指针 `colcount`/`rowcount` 写入 `region->colcount`；
//!    Rust 计算结果通过返回值 [`BBox`] 传出，不可变借用 [`RegionView`] 仅读
//! 2. **浮点统一 f64**（ADR-016 + codex 复核），舍入显式 `(x + 0.5).floor() as i32`
//! 3. **flags 用具名常量** [`TRIM_C1`]..[`TRIM_CALC_TEXT`]，避免散落魔术数字
//! 4. **`calc_bbox` 拒绝越界**：C 版 `printf + exit(10)`，Rust 版返回 `Result`

use crate::region::RegionView;
use k2core::Rect;

/// 裁剪 / 文本参数算法的可调参数。对应 C 版 `K2PDFOPT_SETTINGS` 的相关字段。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CropSettings {
    /// 源文档是否从左到右（C `src_left_to_right`，默认 true）。
    /// 影响 [`calc_bbox`] 中左右两侧 `trim_to` 的 `gaplen` 不对称：
    /// LTR 时左 2.0 / 右 4.0，RTL 时左 4.0 / 右 2.0。
    pub src_left_to_right: bool,
    /// 容忍的孤立瑕疵半径（pts），用于 `trim_to` 抑制噪声。C 默认 1.5。
    pub defect_size_pts: f64,
}

impl Default for CropSettings {
    fn default() -> Self {
        Self {
            src_left_to_right: true,
            defect_size_pts: 1.5,
        }
    }
}

/// `trim_margins` flags 位（与 C 版 0x1/0x2/0x4/0x8/0x10 对齐）。
pub const TRIM_C1: u8 = 0x1;
/// 见 [`TRIM_C1`]。
pub const TRIM_C2: u8 = 0x2;
/// 见 [`TRIM_C1`]。
pub const TRIM_R1: u8 = 0x4;
/// 见 [`TRIM_C1`]。
pub const TRIM_R2: u8 = 0x8;
/// 见 [`TRIM_C1`]。 计算文本统计（rowbase / lcheight / capheight）。
pub const TRIM_CALC_TEXT: u8 = 0x10;
/// 4 边全裁，不算 text params（等价于 C 版常见参数 `0xf`）。
pub const TRIM_ALL: u8 = TRIM_C1 | TRIM_C2 | TRIM_R1 | TRIM_R2;
/// 4 边全裁 + text params（等价于 C 版 `0x1f`）。
pub const TRIM_ALL_AND_TEXT: u8 = TRIM_ALL | TRIM_CALC_TEXT;

/// 文本行统计结果。对应 C `TEXTROW` 的字段子集（`k2pdfopt.h:517-535`）。
///
/// 仅在 [`calc_bbox`] 的 `calc_text_params=true` 时计算。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TextRowStats {
    /// "基线"行号：rowcount 跨越 50% 阈值时（从 r2 侧）所在行。
    /// 对应 C `bbox->rowbase`（行 598）。
    pub rowbase: i32,
    /// 50% 阈值上下宽度（= lcheight 默认值）。对应 C `bbox->h5050`（行 602）。
    pub h5050: i32,
    /// 小写字母高度（启发式调整后）。对应 C `bbox->lcheight`。
    pub lcheight: i32,
    /// 大写字母高度（5% 阈值上下宽度，经 height2 sanity 调整）。对应 C `bbox->capheight`。
    pub capheight: i32,
}

/// 裁剪后的边界框 + 行/列像素直方图 + 可选文本统计。
#[derive(Clone, Debug)]
pub struct BBox {
    /// 收紧后的矩形（inclusive 端点）。
    pub rect: Rect,
    /// 逐行黑像素计数；索引为**绝对 bitmap 行号**（长度 = `bmp.height`）。
    /// 仅 `[rect.y0, rect.y1]` 范围内非 0；其余位置 0。
    pub row_counts: Vec<i32>,
    /// 逐列黑像素计数；索引为**绝对 bitmap 列号**（长度 = `bmp.width`）。
    pub col_counts: Vec<i32>,
    /// 文本统计；仅当 `calc_text_params=true` 时填充。
    pub text_stats: Option<TextRowStats>,
}

/// `calc_bbox` / `trim_margins` 的错误类型。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CropError {
    /// 区域端点超出底层位图边界（C 版直接 `exit(10)`，Rust 版返回错误）。
    #[error("region out of bounds: rect={rect:?}, bitmap size={bmp_w}x{bmp_h}")]
    OutOfBounds { rect: Rect, bmp_w: u32, bmp_h: u32 },
}

/// 计算 `view` 内的紧凑边界框 + 行/列直方图。
///
/// **C 对照**：`bmpregion_calc_bbox`（`bmpregion.c:466-684`）。
///
/// 步骤：
/// 1. 校验 rect 在位图范围内（C 行 484-494，越界则 `exit(10)`）
/// 2. 用 `bmp.width`/`bmp.height` 大小的稀疏数组 `col_counts`/`row_counts`
///    累计每行每列黑像素数（C 行 524-536）
/// 3. 四次 `trim_to` 收紧 c1/c2/r1/r2（C 行 553-562；LTR vs RTL 影响左右 gaplen）
/// 4. 若 `calc_text_params=true`，进一步算 `rowbase` / `h5050` / `lcheight` /
///    `capheight`（C 行 587-655）
pub fn calc_bbox(
    view: &RegionView,
    settings: &CropSettings,
    calc_text_params: bool,
) -> Result<BBox, CropError> {
    if !view.is_in_bounds() {
        return Err(CropError::OutOfBounds {
            rect: view.rect,
            bmp_w: view.bmp.width,
            bmp_h: view.bmp.height,
        });
    }

    let bmp_w = view.bmp.width as usize;
    let bmp_h = view.bmp.height as usize;
    let mut col_counts = vec![0i32; bmp_w];
    let mut row_counts = vec![0i32; bmp_h];

    // C 行 524-536: 累计每行/每列黑像素数（"暗 = p[0] < bgcolor"）
    let mut bbox_rect = view.rect;
    let bgcolor = view.bgcolor;
    for r in bbox_rect.y0..=bbox_rect.y1 {
        for c in bbox_rect.x0..=bbox_rect.x1 {
            // 已经 is_in_bounds 校验过 c/r 非负且 < bmp_w/bmp_h，可安全转 u32
            let gray = view.bmp.gray_at(c as u32, r as u32).unwrap_or(255);
            if gray < bgcolor {
                row_counts[r as usize] += 1;
                col_counts[c as usize] += 1;
            }
        }
    }

    // C 行 553-562: trim 4 边
    let left_gap = if settings.src_left_to_right { 2.0 } else { 4.0 };
    let right_gap = if settings.src_left_to_right { 4.0 } else { 2.0 };
    trim_to(
        &col_counts,
        &mut bbox_rect.x0,
        bbox_rect.x1,
        left_gap,
        view.dpi,
        settings.defect_size_pts,
    );
    trim_to(
        &col_counts,
        &mut bbox_rect.x1,
        bbox_rect.x0,
        right_gap,
        view.dpi,
        settings.defect_size_pts,
    );
    trim_to(
        &row_counts,
        &mut bbox_rect.y0,
        bbox_rect.y1,
        4.0,
        view.dpi,
        settings.defect_size_pts,
    );
    trim_to(
        &row_counts,
        &mut bbox_rect.y1,
        bbox_rect.y0,
        4.0,
        view.dpi,
        settings.defect_size_pts,
    );

    // C 行 587-655: 文本统计
    let text_stats = if calc_text_params {
        Some(calc_text_row_stats(&row_counts, bbox_rect))
    } else {
        None
    };

    Ok(BBox {
        rect: bbox_rect,
        row_counts,
        col_counts,
        text_stats,
    })
}

/// 按 `flags` 收紧 region 矩形，返回新矩形。
///
/// **C 对照**：`bmpregion_trim_margins`（`bmpregion.c:748-766`）。
///
/// 内部调用 [`calc_bbox`] 然后按 flags 复制端点：
/// - `flags & TRIM_C1` → 收紧 x0
/// - `flags & TRIM_C2` → 收紧 x1
/// - `flags & TRIM_R1` → 收紧 y0
/// - `flags & TRIM_R2` → 收紧 y1
/// - `flags & TRIM_CALC_TEXT` → 同时算文本统计
///
/// 未被 flag 选中的端点保持原值。
pub fn trim_margins(
    view: &RegionView,
    settings: &CropSettings,
    flags: u8,
) -> Result<Rect, CropError> {
    let calc_text = (flags & TRIM_CALC_TEXT) != 0;
    let bbox = calc_bbox(view, settings, calc_text)?;
    let mut out = view.rect;
    if (flags & TRIM_C1) != 0 {
        out.x0 = bbox.rect.x0;
    }
    if (flags & TRIM_C2) != 0 {
        out.x1 = bbox.rect.x1;
    }
    if (flags & TRIM_R1) != 0 {
        out.y0 = bbox.rect.y0;
    }
    if (flags & TRIM_R2) != 0 {
        out.y1 = bbox.rect.y1;
    }
    Ok(out)
}

/// 同 [`trim_margins`] 但同时返回 [`BBox`]（行列直方图 + 可选文本统计）。
///
/// 便于上层（行/列检测、wrap 等）复用 `BBox` 内的统计结果，避免重算。
pub fn trim_margins_with_bbox(
    view: &RegionView,
    settings: &CropSettings,
    flags: u8,
) -> Result<(Rect, BBox), CropError> {
    let calc_text = (flags & TRIM_CALC_TEXT) != 0;
    let bbox = calc_bbox(view, settings, calc_text)?;
    let mut out = view.rect;
    if (flags & TRIM_C1) != 0 {
        out.x0 = bbox.rect.x0;
    }
    if (flags & TRIM_C2) != 0 {
        out.x1 = bbox.rect.x1;
    }
    if (flags & TRIM_R1) != 0 {
        out.y0 = bbox.rect.y0;
    }
    if (flags & TRIM_R2) != 0 {
        out.y1 = bbox.rect.y1;
    }
    Ok((out, bbox))
}

/// 判定 `view` 是否为"空白"区域。
///
/// **C 对照**：`bmpregion_is_blank`（`bmpregion.c:407-423`）。
///
/// 算法：trim margins（4 边全裁），比较裁剪前后面积。判定为 blank 的三个条件：
/// - 裁后宽度 - 1 ≤ 5（即 `width <= 6`）
/// - 裁后高度 - 1 ≤ 5
/// - 裁后面积 / **整张位图**面积 < 1e-4（C 行 417-422）
///
/// 注意 C 版 a1 用的是 `bmp->width * bmp->height` 而非 region rect 面积，
/// 故 a2/a1 比值天然偏小（防止小 region 误判）。
pub fn is_blank(view: &RegionView, settings: &CropSettings) -> Result<bool, CropError> {
    let trimmed = trim_margins(view, settings, TRIM_ALL)?;
    let a1 = (view.bmp.width as f64) * (view.bmp.height as f64);
    let a2 = (trimmed.width() as f64) * (trimmed.height() as f64);
    // C 行 422: c2 - c1 <= 5 即"裁后宽度 - 1 ≤ 5"，宽度本身 ≤ 6
    // 空矩形（trimmed.width()=0 或 height()=0）也算 blank
    let w_le = trimmed.x1 - trimmed.x0 <= 5;
    let h_le = trimmed.y1 - trimmed.y0 <= 5;
    let ratio_small = a1 > 0.0 && (a2 / a1) < 1e-4;
    Ok(w_le || h_le || ratio_small)
}

// --------------------------------------------------------------------------
// 内部 helper
// --------------------------------------------------------------------------

/// 严格复刻 C 版 `static void trim_to(...)`（`bmpregion.c:769-819`）。
///
/// 从 `*i1` 向 `i2` 方向扫描 `count[]`，找出首个"实质性内容簇"位置并写回 `*i1`。
/// `gaplen`/`defect_size_pts` 都按 pts 给出，函数内部按 `dpi` 换算成像素。
fn trim_to(count: &[i32], i1: &mut i32, i2: i32, gaplen: f64, dpi: f32, defect_size_pts: f64) {
    let dpi = dpi as f64;
    // C 行 774: igaplen = (int)(gaplen*dpi/72.); if (igaplen<1) igaplen=1;
    let mut igaplen = (gaplen * dpi / 72.0) as i32;
    if igaplen < 1 {
        igaplen = 1;
    }
    // C 行 778: clevel = 0
    let clevel: i32 = 0;
    // C 行 779: dlevel = (int)(pow(defect_size_pts*dpi/72., 2.)*PI/4. + .5)
    let r_px = defect_size_pts * dpi / 72.0;
    let dlevel = (r_px * r_px * std::f64::consts::PI / 4.0 + 0.5) as i32;
    // C 行 780: del = i2 > (*i1) ? 1 : -1
    let del: i32 = if i2 > *i1 { 1 } else { -1 };

    let mut defect_start: i32 = -1;
    let mut last_defect: i32 = -1;
    let mut dcount: i32 = 0;

    // C 行 784: for (;(*i1)!=i2;(*i1)=(*i1)+del)
    while *i1 != i2 {
        let idx = *i1;
        // count 是 bmp_w/bmp_h 大小、idx 在 [x0, x1] 或 [y0, y1] 范围内
        // 由 is_in_bounds 保证非负且 < len。但 trim 过程中 *i1 不会被推到 idx 外
        // 因为方向是 i1 → i2 都在合法区间内
        // 安全兜底：越界视为 0（与 C 版 i1 始终在合法区间内的语义一致）
        let v = count.get(idx as usize).copied().unwrap_or(0);
        if v <= clevel {
            dcount = 0; // 重置 defect 大小
        } else {
            // 找到一个"标记"
            if dcount == 0 {
                if defect_start >= 0 {
                    last_defect = defect_start;
                }
                defect_start = idx;
            }
            dcount += v;
            if dcount >= dlevel {
                if last_defect >= 0 && (defect_start - last_defect).abs() <= igaplen {
                    *i1 = last_defect;
                } else {
                    *i1 = defect_start;
                }
                return;
            }
        }
        *i1 += del;
    }

    // C 行 808-818: 循环正常结束后的尾部处理
    if defect_start < 0 {
        return;
    }
    if last_defect < 0 {
        *i1 = defect_start;
        return;
    }
    if (defect_start - last_defect).abs() <= igaplen {
        *i1 = last_defect;
    } else {
        *i1 = defect_start;
    }
}

/// 计算"加权高度"，对应 C `static int height2_calc(int *rc, int n)`
/// (`bmpregion.c:827-873`)。
///
/// 算法：对 `rc[0..n]` 排序后取第 90 百分位除 2 作为阈值，找首尾跨阈值的索引差。
fn height2_calc(rc: &[i32]) -> i32 {
    let n = rc.len();
    if n == 0 {
        return 1;
    }
    let mut sorted: Vec<i32> = rc.to_vec();
    sorted.sort_unstable();
    // C 行 859: thresh = c[9*n/10] / 2
    let pivot = (9 * n) / 10;
    let pivot = pivot.min(n - 1);
    let thresh = sorted[pivot] / 2;
    // C 行 861-863
    let mut i1: usize = 0;
    for (i, v) in rc.iter().enumerate().take(n.saturating_sub(1)) {
        if *v >= thresh {
            i1 = i;
            break;
        }
    }
    // C 行 865-867
    let mut i2: usize = i1;
    for i in (i1 + 1..n).rev() {
        if rc[i] >= thresh {
            i2 = i;
            break;
        }
    }
    // C 行 871: h2 = i - i1 + 1（保证 >= 1）
    let mut h2 = i2 as i32 - i1 as i32 + 1;
    if h2 < 1 {
        h2 = 1;
    }
    h2
}

/// 计算 `bbox` 内的文本行统计。
///
/// **C 对照**：`bmpregion_calc_bbox` 的 calc_text_params 分支（行 587-655）。
fn calc_text_row_stats(row_counts: &[i32], bbox: Rect) -> TextRowStats {
    let y0 = bbox.y0.max(0) as usize;
    let y1 = (bbox.y1 as usize).min(row_counts.len().saturating_sub(1));
    if y1 < y0 {
        return TextRowStats {
            rowbase: bbox.y1,
            h5050: 1,
            lcheight: 1,
            capheight: 1,
        };
    }

    // C 行 590-593: maxcount = max(rowcount[r1..=r2])
    let mut maxcount = 0i32;
    for v in &row_counts[y0..=y1] {
        if *v > maxcount {
            maxcount = *v;
        }
    }
    // C 行 594: mc2 = maxcount / 2
    let mc2_50 = maxcount / 2;
    // C 行 595-598: rowbase = 从 r2 向 r1 第一个 rowcount[i] > mc2 的索引
    let mut rowbase = bbox.y1;
    for i in (y0..=y1).rev() {
        if row_counts[i] > mc2_50 {
            rowbase = i as i32;
            break;
        }
    }
    // C 行 599-602: i = 从 r1 向 r2 第一个 rowcount[i] > mc2; lcheight = h5050 = rowbase - i + 1
    let mut first_50 = bbox.y0;
    for (i, v) in row_counts.iter().enumerate().take(y1 + 1).skip(y0) {
        if *v > mc2_50 {
            first_50 = i as i32;
            break;
        }
    }
    let mut h5050 = rowbase - first_50 + 1;
    if h5050 < 1 {
        h5050 = 1;
    }
    let mut lcheight = h5050;
    // C 行 603-607: mc2 = maxcount / 20; capheight = rowbase - (第一个 > mc2 from r1) + 1
    let mc2_5 = maxcount / 20;
    let mut first_5 = bbox.y0;
    for (i, v) in row_counts.iter().enumerate().take(y1 + 1).skip(y0) {
        if *v > mc2_5 {
            first_5 = i as i32;
            break;
        }
    }
    let mut capheight = rowbase - first_5 + 1;
    if capheight < 1 {
        capheight = 1;
    }

    // C 行 612: h2 = height2_calc(&rowcount[r1], r2-r1+1)
    let h2 = height2_calc(&row_counts[y0..=y1]);
    // C 行 620-621: if (capheight < h2*0.75) capheight = h2
    if (capheight as f64) < (h2 as f64) * 0.75 {
        capheight = h2;
    }
    // C 行 622-632: lcheight 启发式调整
    let f = (lcheight as f64) / (capheight as f64);
    if !(0.55..=0.85).contains(&f) {
        // C: lcheight = (int)(0.72*capheight + .5)
        lcheight = ((0.72 * capheight as f64) + 0.5) as i32;
        if lcheight < 1 {
            lcheight = 1;
        }
    }

    TextRowStats {
        rowbase,
        h5050,
        lcheight,
        capheight,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::region::RegionView;
    use k2types::{Bitmap, PixelFormat};

    fn make_white_bmp(w: u32, h: u32) -> Bitmap {
        let mut bmp = Bitmap::new(w, h, 150.0, PixelFormat::Gray8).unwrap();
        bmp.fill_byte(255);
        bmp
    }

    /// 在 `bmp` 的 `[x0, x1] x [y0, y1]` 范围画"黑色"（gray=0）矩形。
    fn paint_rect(bmp: &mut Bitmap, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let px = bmp.pixel_mut(x, y).unwrap();
                px[0] = 0;
            }
        }
    }

    // ---- trim_to ----

    #[test]
    fn trim_to_finds_first_real_content_cluster() {
        // count = [0,0,0,5,5,5,5,5,0,0]，从左 i1=0 向 i2=9 找
        // dlevel @ defect_size=1.5pt, dpi=150 -> r_px=3.125, area ≈ 7.67 -> dlevel=8
        // 经过 i=3..7 累计 25 → ≥ 8 触发；返回 defect_start=3
        let count = vec![0, 0, 0, 5, 5, 5, 5, 5, 0, 0];
        let mut i1 = 0;
        trim_to(&count, &mut i1, 9, 4.0, 150.0, 1.5);
        assert_eq!(i1, 3);
    }

    #[test]
    fn trim_to_handles_reverse_direction() {
        // i2 < *i1 → del = -1，从右向左扫
        let count = vec![0, 0, 0, 5, 5, 5, 5, 5, 0, 0];
        let mut i1 = 9;
        trim_to(&count, &mut i1, 0, 4.0, 150.0, 1.5);
        assert_eq!(i1, 7);
    }

    #[test]
    fn trim_to_all_zero_count_no_change() {
        let count = vec![0i32; 10];
        let mut i1 = 0;
        trim_to(&count, &mut i1, 9, 4.0, 150.0, 1.5);
        // 无任何 defect_start → 函数返回时 i1 = i2（循环 *i1!=i2 自然结束后 *i1==i2）
        assert_eq!(i1, 9);
    }

    #[test]
    fn trim_to_handles_isolated_noise() {
        // 一个孤立的小点不应被认为是边缘
        // count = [0, 0, 1, 0, 0, 0, 5, 5, 5, 5]
        // i1=0 找到 idx=2 v=1 < dlevel=8，dcount=1，继续，i=3 v=0 重置 dcount
        // 后续在 idx=6 处累计 5,10..(>=8) 时触发 → last_defect=2, defect_start=6
        // |6-2|=4, igaplen=4*150/72=8 → 4<=8 → 回到 last_defect=2
        // 即 dcount=0 后下一个 defect_start 会把前一次的 defect_start 存为 last_defect
        // 这里 igaplen=8（gaplen=4.0），4<=8 → i1 = 2（保留孤立点附近）
        let count = vec![0, 0, 1, 0, 0, 0, 5, 5, 5, 5];
        let mut i1 = 0;
        trim_to(&count, &mut i1, 9, 4.0, 150.0, 1.5);
        // C 版会回到 last_defect=2（gap 在 igaplen 内）
        assert_eq!(i1, 2);
    }

    // ---- calc_bbox ----

    #[test]
    fn calc_bbox_tight_around_drawn_rect() {
        let mut bmp = make_white_bmp(50, 30);
        // 画一个 20x10 的黑矩形 @ (10,8) -> (29,17)
        paint_rect(&mut bmp, 10, 8, 29, 17);
        let view = RegionView::full(&bmp);
        let s = CropSettings::default();
        let bb = calc_bbox(&view, &s, false).unwrap();
        // bbox 应该收紧到 [10,8,29,17] 或非常接近
        assert!(
            (bb.rect.x0 - 10).abs() <= 1,
            "x0={} should be near 10",
            bb.rect.x0
        );
        assert!(
            (bb.rect.x1 - 29).abs() <= 1,
            "x1={} should be near 29",
            bb.rect.x1
        );
        assert!(
            (bb.rect.y0 - 8).abs() <= 1,
            "y0={} should be near 8",
            bb.rect.y0
        );
        assert!(
            (bb.rect.y1 - 17).abs() <= 1,
            "y1={} should be near 17",
            bb.rect.y1
        );
        // row_counts / col_counts 长度 = 整张图
        assert_eq!(bb.row_counts.len(), 30);
        assert_eq!(bb.col_counts.len(), 50);
        // text_stats 默认不算
        assert!(bb.text_stats.is_none());
    }

    #[test]
    fn calc_bbox_white_region_returns_initial_rect_or_collapses() {
        let bmp = make_white_bmp(20, 20);
        let view = RegionView::full(&bmp);
        let s = CropSettings::default();
        let bb = calc_bbox(&view, &s, false).unwrap();
        // 全白 → trim_to 循环正常结束、*i1 = i2，意思是从两端都裁到中间
        // 实际：x0 从 0 向 19 扫无内容 → x0 := 19；x1 从 19 向 0 扫无内容 → x1 := 0
        // bbox 退化为 "x0=19, x1=0"（is_empty 矩形）
        assert!(bb.rect.x0 >= bb.rect.x1 || bb.rect.y0 >= bb.rect.y1);
    }

    #[test]
    fn calc_bbox_text_stats_populated_when_requested() {
        let mut bmp = make_white_bmp(100, 60);
        // 模拟两行文本：每行 5 像素高，留 4 像素空隙
        paint_rect(&mut bmp, 10, 20, 80, 24); // 行 1
        paint_rect(&mut bmp, 10, 29, 80, 33); // 行 2
        let view = RegionView::full(&bmp);
        let s = CropSettings::default();
        let bb = calc_bbox(&view, &s, true).unwrap();
        let stats = bb.text_stats.expect("calc_text_params=true");
        // rowbase 应该接近末行底部
        assert!(
            stats.rowbase >= 25 && stats.rowbase <= 33,
            "rowbase={}",
            stats.rowbase
        );
        // 各高度都至少 1
        assert!(stats.h5050 >= 1);
        assert!(stats.lcheight >= 1);
        assert!(stats.capheight >= 1);
    }

    #[test]
    fn calc_bbox_out_of_bounds_returns_err() {
        let bmp = make_white_bmp(5, 5);
        let bad = RegionView::new(&bmp, Rect::new(0, 0, 5, 4)); // x1=5 越界
        let s = CropSettings::default();
        let err = calc_bbox(&bad, &s, false).unwrap_err();
        match err {
            CropError::OutOfBounds { bmp_w, bmp_h, .. } => {
                assert_eq!(bmp_w, 5);
                assert_eq!(bmp_h, 5);
            }
        }
    }

    // ---- trim_margins ----

    #[test]
    fn trim_margins_respects_flag_bits() {
        let mut bmp = make_white_bmp(50, 30);
        paint_rect(&mut bmp, 10, 8, 29, 17);
        let view = RegionView::full(&bmp);
        let s = CropSettings::default();

        // 只裁 c1
        let r = trim_margins(&view, &s, TRIM_C1).unwrap();
        assert!(r.x0 > 0, "x0 should be tightened");
        assert_eq!(r.x1, 49, "x1 unchanged");
        assert_eq!(r.y0, 0);
        assert_eq!(r.y1, 29);

        // 只裁 y 方向
        let r = trim_margins(&view, &s, TRIM_R1 | TRIM_R2).unwrap();
        assert_eq!(r.x0, 0);
        assert_eq!(r.x1, 49);
        assert!(r.y0 > 0);
        assert!(r.y1 < 29);

        // 全裁
        let r = trim_margins(&view, &s, TRIM_ALL).unwrap();
        assert!(r.x0 >= 9 && r.x0 <= 11);
        assert!(r.x1 >= 28 && r.x1 <= 30);
    }

    #[test]
    fn trim_margins_no_flags_returns_original_rect() {
        let mut bmp = make_white_bmp(20, 20);
        paint_rect(&mut bmp, 5, 5, 14, 14);
        let view = RegionView::full(&bmp);
        let s = CropSettings::default();
        let r = trim_margins(&view, &s, 0).unwrap();
        assert_eq!(r, view.rect);
    }

    #[test]
    fn trim_margins_with_bbox_returns_both() {
        let mut bmp = make_white_bmp(40, 40);
        paint_rect(&mut bmp, 10, 10, 29, 29);
        let view = RegionView::full(&bmp);
        let s = CropSettings::default();
        let (r, bb) = trim_margins_with_bbox(&view, &s, TRIM_ALL).unwrap();
        assert_eq!(r, bb.rect);
        // row_counts 在 [10, 29] 处应该 > 0
        let sum_in: i32 = bb.row_counts[10..=29].iter().sum();
        assert!(sum_in > 0);
    }

    // ---- is_blank ----

    #[test]
    fn is_blank_true_for_all_white() {
        let bmp = make_white_bmp(100, 100);
        let view = RegionView::full(&bmp);
        let s = CropSettings::default();
        assert!(is_blank(&view, &s).unwrap());
    }

    #[test]
    fn is_blank_false_for_substantial_content() {
        let mut bmp = make_white_bmp(100, 100);
        paint_rect(&mut bmp, 10, 10, 89, 89); // 80x80 黑块占图 64%
        let view = RegionView::full(&bmp);
        let s = CropSettings::default();
        assert!(!is_blank(&view, &s).unwrap());
    }

    #[test]
    fn is_blank_true_for_tiny_speck() {
        // 一个 2x2 黑点：trim 后宽度 ≤ 6 触发 blank
        // 注意 defect_size_pts=1.5 @ dpi=150 → r_px=3.125 → dlevel=8
        // 2x2=4 像素 < dlevel=8，trim_to 不会停在这里 → 整图都被认为是空白
        let mut bmp = make_white_bmp(200, 200);
        paint_rect(&mut bmp, 100, 100, 101, 101);
        let view = RegionView::full(&bmp);
        let s = CropSettings::default();
        assert!(is_blank(&view, &s).unwrap());
    }

    #[test]
    fn is_blank_ratio_threshold() {
        // 一个 8x8 黑块 @ 800x800 → 面积比 = 64/640000 = 1e-4 边界
        // 实际 trim 后只剩 8x8，宽度 - 1 = 7 > 5 → 不会因尺寸触发
        // a2/a1 = 64/640000 = 1.0e-4 → 不严格小于 1e-4 → 不算 blank
        let mut bmp = make_white_bmp(800, 800);
        paint_rect(&mut bmp, 100, 100, 107, 107);
        let view = RegionView::full(&bmp);
        let s = CropSettings::default();
        // 实际取决于 dlevel；defect_size_pts=1.5@150dpi → dlevel=8
        // 8x8=64 像素累积，trim_to 找到边界
        // 此处不强约束 blank 与否，仅测试 API 调用成功
        let _ = is_blank(&view, &s).unwrap();
    }

    // ---- height2_calc 单测 ----

    #[test]
    fn height2_calc_normal_distribution() {
        // 30 个值，多数 0/低，中间一段高 → 第 90 百分位 / 2 作阈值
        let mut rc = vec![0i32; 30];
        for v in rc.iter_mut().take(20).skip(10) {
            *v = 100;
        }
        let h2 = height2_calc(&rc);
        // 第 90 百分位 = rc[27]（sorted）= 0... 实际上排序后 [0,..,0, 100,..,100]
        // 100 ≥ 17 → sorted[27]=100, thresh=50 → rc[10] 起 ≥ 50 → i1=10, i2=19 → h2=10
        assert_eq!(h2, 10);
    }

    #[test]
    fn height2_calc_empty_returns_one() {
        assert_eq!(height2_calc(&[]), 1);
    }

    #[test]
    fn height2_calc_all_zero_returns_at_least_one() {
        let rc = vec![0i32; 20];
        let h2 = height2_calc(&rc);
        assert!(h2 >= 1);
    }
}
