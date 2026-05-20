//! 列检测与页面区域分解 —— 对应 C 版 `k2proc.c::pageregions_find_columns`
//! 与 `pageregions.c`。
//!
//! ## 算法分级
//!
//! v2.55 C 版完整列检测依赖文本行（`textrows`）边界来限定垂直搜索范围（详见
//! `k2proc.c::bmpregion_find_multicolumn_divider`）。Step 6.2 之前 textrow
//! 还未实现，因此本 Step 6.1 落地"**简化版列检测**"：
//!
//! 1. 数据结构与 C 版 1:1 对照（[`PageRegion`] / [`PageRegions`]）
//! 2. 完整公开 helper（[`row_black_count`] / [`col_black_count`] /
//!    [`is_clear`] / [`column_height_and_gap_test`]），可在 Step 6.2 完成后
//!    被 textrow-aware 版本直接复用
//! 3. 主算法 [`find_multicolumn_divider_simple`] 用"垂直 shaft + 高度 gap
//!    测试"在 region.y0..=region.y1 全局搜索（不依赖 textrow.r1/r2 限定）
//!
//! ## 简化版与完整版的预期差异
//!
//! - 简化版无法处理"上有页眉 + 下有页脚 + 中间两列"，会因页眉/页脚跨度过长
//!   把 shaft 测试拒绝。fixture `two-column.pdf` / `three-column.pdf` 等
//!   synthetic 数据无此问题，可通过验收
//! - Step 6.2 完成后回头实现 textrow-aware 版本（Open Question 6.1.A）
//!
//! ## 关键 C 文件 / 行号对照
//!
//! | C 函数 | C 行号 | Rust 函数 |
//! |--------|--------|-----------|
//! | `pageregions_find_columns` | `k2proc.c:799-851` | [`find_columns`] |
//! | `pageregions_find_next_level` | `k2proc.c:859-1017` | [`find_columns_in_view`] |
//! | `bmpregion_find_multicolumn_divider` | `k2proc.c:1809-2224` | [`find_multicolumn_divider_simple`] |
//! | `bmpregion_is_clear` | `bmpregion.c:155-251` | [`is_clear`] |
//! | `bmpregion_column_height_and_gap_test` | `bmpregion.c:306-341` | [`column_height_and_gap_test`] |
//! | `bmpregion_row_black_count` | `bmpregion.c:42-54` | [`row_black_count`] |
//! | `bmpregion_col_black_count` | `bmpregion.c:57-70` | [`col_black_count`] |

use crate::crop::{trim_margins, CropError, CropSettings, TRIM_ALL};
use crate::region::RegionView;
use k2core::Rect;
use k2types::Bitmap;

/// 单个页面区域 —— 对应 C 版 `PAGEREGION`（`k2pdfopt.h:885-891`）。
///
/// 注意：C 版的 `bmpregion` 字段是完整的 `BMPREGION`（含 textrows / wrectmaps
/// 等），Rust 版只保留几何 [`Rect`] 与三个标志。后续 layout 阶段需要更多上下文
/// 时再扩展。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRegion {
    /// 区域几何（inclusive）。
    pub rect: Rect,
    /// 是否为整页跨度（`fullspan=1`）—— 1 列时整页 + 2 列时上/下贯穿条带都是 fullspan
    pub fullspan: bool,
    /// 列检测层级（C `level`）。1=最外层，2/3/... = 嵌套子列
    pub level: u8,
    /// 是否为"notes" 列（C `notes`，v2.20+）。简化版尚未支持 notes 检测，恒为 false
    pub notes: bool,
}

impl PageRegion {
    /// 构造一个"整页 fullspan"区域。
    #[must_use]
    pub fn fullspan(rect: Rect, level: u8) -> Self {
        Self {
            rect,
            fullspan: true,
            level,
            notes: false,
        }
    }

    /// 构造一个"单列子区"区域（非 fullspan）。
    #[must_use]
    pub fn column(rect: Rect, level: u8) -> Self {
        Self {
            rect,
            fullspan: false,
            level,
            notes: false,
        }
    }
}

/// 页面区域有序集合 —— 对应 C 版 `PAGEREGIONS`（`k2pdfopt.h:892-896`）。
///
/// 顺序代表显示顺序（左到右、上到下，遵循 [`ColumnSettings::src_left_to_right`]）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageRegions {
    /// 全部区域，按显示顺序排列。
    pub regions: Vec<PageRegion>,
}

impl PageRegions {
    /// 构造空集合。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前区域数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// 添加一个区域。
    pub fn push(&mut self, region: PageRegion) {
        self.regions.push(region);
    }
}

/// 列检测设置 —— 对应 C 版 `K2PDFOPT_SETTINGS` 中影响列检测的字段。
///
/// 独立 struct（不直接借用 `k2settings::Settings`），避免 `k2layout → k2settings`
/// 反向依赖。上层组装 [`crate::ConvertContext`] 时再做字段转换。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnSettings {
    /// 最大列数（C `max_columns`，默认 2）
    pub max_columns: u32,
    /// 最小列间隙，inches（C `min_column_gap_inches`，默认 0.1）
    pub min_column_gap_inches: f64,
    /// 最大列间隙，inches（C `max_column_gap_inches`，默认 1.5；负值=禁用）
    pub max_column_gap_inches: f64,
    /// 最小列高度，inches（C `min_column_height_inches`，默认 1.5）
    pub min_column_height_inches: f64,
    /// 列分隔位置允许的搜索范围（C `column_gap_range`，默认 0.33；
    /// 实际半宽 = width * column_gap_range / 2，即页面中线 ± 16.5%）
    pub column_gap_range: f64,
    /// 列分隔位置允许的跨页移动（C `column_offset_max`，默认 0.3；
    /// 当前未在简化版使用，预留给 6.2 完整版）
    pub column_offset_max: f64,
    /// "shaft 是否 clear"的暗像素允许密度（C `gtc_in`，默认 0.005 ≈ 0.5%）
    pub gtc_in: f64,
    /// 是否对裁出的 region 做 trim（C `src_trim`，默认 true）
    pub src_trim: bool,
    /// 列阅读顺序：true=左到右，false=右到左（C `src_left_to_right`，默认 true）
    pub src_left_to_right: bool,
    /// trim 用的 defect_size_pts（默认 0.75，与 [`CropSettings`] 一致）
    pub defect_size_pts: f64,
}

impl Default for ColumnSettings {
    fn default() -> Self {
        Self {
            max_columns: 2,
            min_column_gap_inches: 0.1,
            max_column_gap_inches: 1.5,
            min_column_height_inches: 1.5,
            column_gap_range: 0.33,
            column_offset_max: 0.3,
            gtc_in: 0.005,
            src_trim: true,
            src_left_to_right: true,
            defect_size_pts: 0.75,
        }
    }
}

impl ColumnSettings {
    fn to_crop_settings(self) -> CropSettings {
        CropSettings {
            src_left_to_right: self.src_left_to_right,
            defect_size_pts: self.defect_size_pts,
        }
    }
}

/// 单列高度 + gap 测试的位标志状态码（C 行 296-303）。
///
/// 位含义：
/// - `bit 0 (1)` → column 0（左）高度不足
/// - `bit 1 (2)` → column 1（右）高度不足
/// - `bit 2 (4)` → 两列之间的 gap 超过 `max_column_gap_inches`
///
/// `0` 表示通过。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnTestStatus(pub u8);

impl ColumnTestStatus {
    /// 测试通过。
    pub const OK: Self = Self(0);
    /// 左列高度不足。
    pub const COL0_TOO_SHORT: u8 = 1;
    /// 右列高度不足。
    pub const COL1_TOO_SHORT: u8 = 2;
    /// 列间隙过大。
    pub const GAP_TOO_WIDE: u8 = 4;

    /// 是否完全通过。
    #[must_use]
    pub fn is_ok(self) -> bool {
        self.0 == 0
    }

    /// 是否包含指定位。
    #[must_use]
    pub fn has(self, bit: u8) -> bool {
        (self.0 & bit) != 0
    }
}

/// [`is_clear`] 返回的状态。
///
/// **C 对照**：`bmpregion.c::bmpregion_is_clear` 行 250 返回 0 表示有暗像素，
/// 非零表示 clear（值范围 1..=11，越小越 "clean"）。本 Rust 版用 enum 表示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearStatus {
    /// 区域不 clear（有过多暗像素）。
    Dirty,
    /// 区域 clear；附带"洁净等级" 1..=11（小=干净，11=刚好达标）。
    Clear(u8),
}

impl ClearStatus {
    /// 是否 clear。
    #[must_use]
    pub fn is_clear(self) -> bool {
        matches!(self, ClearStatus::Clear(_))
    }

    /// 取洁净等级；Dirty 返回 0，Clear(k) 返回 k。
    #[must_use]
    pub fn level(self) -> u8 {
        match self {
            ClearStatus::Dirty => 0,
            ClearStatus::Clear(k) => k,
        }
    }
}

// ----------------------------------------------------------------------------
// 公开 helpers（与 C 版函数 1:1 对照，供本模块 + 后续 textrow / wrap 复用）
// ----------------------------------------------------------------------------

/// 统计指定行 `row` 在 [`RegionView`] 内的暗像素数。
///
/// **C 对照**：`bmpregion.c:42-54 bmpregion_row_black_count`。
///
/// `row` 越界返回 0（C 版无越界保护，调用方保证 `r1<=row<=r2`；Rust 版宽松）。
#[must_use]
pub fn row_black_count(view: &RegionView, row: i32) -> i32 {
    if row < view.rect.y0 || row > view.rect.y1 {
        return 0;
    }
    let mut count = 0;
    for col in view.rect.x0..=view.rect.x1 {
        if view.is_dark(col, row) {
            count += 1;
        }
    }
    count
}

/// 统计指定列 `col` 在 [`RegionView`] 内的暗像素数。
///
/// **C 对照**：`bmpregion.c:57-70 bmpregion_col_black_count`。
#[must_use]
pub fn col_black_count(view: &RegionView, col: i32) -> i32 {
    if col < view.rect.x0 || col > view.rect.x1 {
        return 0;
    }
    let mut count = 0;
    for row in view.rect.y0..=view.rect.y1 {
        if view.is_dark(col, row) {
            count += 1;
        }
    }
    count
}

/// 检测 [`RegionView`] 是否"clear"（暗像素密度低于阈值）。
///
/// **C 对照**：`bmpregion.c:155-251 bmpregion_is_clear`。
///
/// # 参数
///
/// - `view`：待测区域。
/// - `row_black`：可选预算行黑像素数组（索引 = 整张 bitmap 的行号，长度 ≥ `bmp.height`）。
///   若 `Some` 且对应行 `==0`，可跳过逐像素扫描；与 C 行 191-194 一致。
/// - `col_black`：可选预算列黑像素数组（索引 = 整张 bitmap 的列号，长度 ≥ `bmp.width`）。
/// - `gtc_in`：暗像素允许密度（每 inch 的列暗像素阈值，乘以 dpi×宽度 得绝对阈值）。
///
/// # 算法
///
/// 1. 阈值 `pt = (gtc_in × dpi × width + 0.5) as i32`（C 行 164）
/// 2. 若 width/height 任一 ≤ 5 → 直接逐行扫描（不进入 col 优化分支）
/// 3. 比较 row vs col 全零数量，选更高效的扫描方向
/// 4. 累加暗像素，若 > pt 返回 [`ClearStatus::Dirty`]
/// 5. 否则返回 [`ClearStatus::Clear`]（等级 = `1 + (10 * c / pt) as u8`，C 行 187）
///
/// Rust 简化：不实现 C 版的 `col_pix_count`（2D 前缀和缓存），实际使用场景中
/// shaft 宽度通常很小（≤ min_column_gap_pixels ≈ 10-30），暴力扫描足够快。
#[must_use]
pub fn is_clear(
    view: &RegionView,
    row_black: Option<&[i32]>,
    col_black: Option<&[i32]>,
    gtc_in: f64,
) -> ClearStatus {
    let width = view.rect.width() as i32;
    let height = view.rect.height() as i32;
    if width <= 0 || height <= 0 {
        return ClearStatus::Clear(1);
    }

    // C 行 164: pt = gtc_in * dpi * width + 0.5
    let pt = ((gtc_in * view.dpi as f64 * width as f64) + 0.5) as i32;
    let pt = pt.max(0);

    let mindim = width.min(height);

    // C 行 198-231: 大区域时比较 row / col 优先级，选高效方向
    if mindim > 5 {
        if let (Some(rb), Some(cb)) = (row_black, col_black) {
            // bcc = "区域内全零列数"，brc = "区域内全零行数"
            let mut bcc = 0i32;
            for c in view.rect.x0..=view.rect.x1 {
                if let Some(&v) = cb.get(c as usize) {
                    if v == 0 {
                        bcc += 1;
                    }
                }
            }
            let mut brc = 0i32;
            for r in view.rect.y0..=view.rect.y1 {
                if let Some(&v) = rb.get(r as usize) {
                    if v == 0 {
                        brc += 1;
                    }
                }
            }
            // C 行 215: 若按列扫描的有效列数 > 2 倍按行扫描的有效行数 → 按列
            if bcc * height > 2 * brc * width {
                let mut c = 0i32;
                for col in view.rect.x0..=view.rect.x1 {
                    if col < 0 || col >= view.bmp.width as i32 {
                        continue;
                    }
                    let col_zero = cb.get(col as usize).copied().unwrap_or(0) == 0;
                    if col_zero {
                        continue;
                    }
                    c += col_black_count(view, col);
                    if c > pt {
                        return ClearStatus::Dirty;
                    }
                }
                return ClearStatus::Clear(level_from_count(c, pt));
            }
        }
    }

    // 默认按行扫描（C 行 233-251）
    let mut c = 0i32;
    for row in view.rect.y0..=view.rect.y1 {
        if row < 0 || row >= view.bmp.height as i32 {
            continue;
        }
        if let Some(rb) = row_black {
            if rb.get(row as usize).copied().unwrap_or(0) == 0 {
                continue;
            }
        }
        c += row_black_count(view, row);
        if c > pt {
            return ClearStatus::Dirty;
        }
    }
    ClearStatus::Clear(level_from_count(c, pt))
}

fn level_from_count(c: i32, pt: i32) -> u8 {
    // C 行 187 / 250: return pt<=0 ? 1 : 1 + (int)(10 * c / pt);
    if pt <= 0 {
        return 1;
    }
    let lvl = 1 + (10 * c) / pt;
    lvl.clamp(1, 255) as u8
}

/// 列高度 + gap 测试 —— 对应 C 版 `bmpregion_column_height_and_gap_test`。
///
/// **C 对照**：`bmpregion.c:306-341`。
///
/// # 参数
///
/// - `region`：源区域。
/// - `r1`/`r2`：行边界（来自 `textrow[itop].r1` / `textrow[ibottom].r2`；简化版
///   可传 `region.rect.y0` / `region.rect.y1`）。
/// - `cmid`：分隔列号（C `divider_column`）。
/// - `settings`：列设置。
///
/// # 返回
///
/// `(left_trimmed, right_trimmed, status)`：trim 后的左右两列 [`Rect`] 与
/// [`ColumnTestStatus`]。即使状态非 0，trim 后的矩形也已返回（供调用方更新
/// `ileft` / `iright` 优化，对应 C 行 2111-2114）。
///
/// # 错误
///
/// 仅在 trim 过程中遇到 [`CropError::OutOfBounds`] 时返回；正常输入下不会触发。
pub fn column_height_and_gap_test(
    region: &RegionView,
    r1: i32,
    r2: i32,
    cmid: i32,
    settings: &ColumnSettings,
) -> Result<(Rect, Rect, ColumnTestStatus), CropError> {
    let crop = settings.to_crop_settings();
    let min_height_pixels = (settings.min_column_height_inches * region.dpi as f64) as i32;
    let mut status: u8 = 0;

    // 左列：region.c1..=cmid-1, r1..=r2
    let left_view = region.with_rect(Rect::new(region.rect.x0, r1, cmid - 1, r2));
    let left_trimmed = trim_margins(&left_view, &crop, TRIM_ALL)?;
    if (left_trimmed.y1 - left_trimmed.y0 + 1) < min_height_pixels {
        status |= ColumnTestStatus::COL0_TOO_SHORT;
    }

    // 右列：cmid..=region.c2, r1..=r2
    let right_view = region.with_rect(Rect::new(cmid, r1, region.rect.x1, r2));
    let right_trimmed = trim_margins(&right_view, &crop, TRIM_ALL)?;
    if (right_trimmed.y1 - right_trimmed.y0 + 1) < min_height_pixels {
        status |= ColumnTestStatus::COL1_TOO_SHORT;
    }

    // gap 测试（C 行 337-338）
    if settings.max_column_gap_inches >= 0.0 {
        let max_gap_pixels = (settings.max_column_gap_inches * region.dpi as f64).round() as i32;
        let gap_pixels = right_trimmed.x0 - left_trimmed.x1 - 1;
        if gap_pixels > max_gap_pixels {
            status |= ColumnTestStatus::GAP_TOO_WIDE;
        }
    }

    Ok((left_trimmed, right_trimmed, ColumnTestStatus(status)))
}

// ----------------------------------------------------------------------------
// 主算法（公开入口 + 简化版多列分隔搜索）
// ----------------------------------------------------------------------------

/// 列检测主入口 —— 对应 C 版 `pageregions_find_columns`。
///
/// 对整张 `bmp` 做最多 `settings.max_columns` 层列分解，返回按显示顺序排列的
/// [`PageRegions`]。
///
/// 算法：
///
/// 1. `max_columns == 1` → 直接返回单条 fullspan 区域（C 行 819-823）
/// 2. 否则迭代 `ilevel=1..max_columns`，对每一层用
///    [`find_multicolumn_divider_simple`] 尝试切分上一层未 fullspan 的区域
///
/// # 错误
///
/// trim 时遇到越界返回 [`CropError`]；正常 fixture 输入下不会触发。
pub fn find_columns(bmp: &Bitmap, settings: &ColumnSettings) -> Result<PageRegions, CropError> {
    let view = RegionView::full(bmp);

    let mut sorted = PageRegions::new();

    if settings.max_columns <= 1 {
        // C 行 819-823: maxlevels==1 直接 add fullspan
        sorted.push(PageRegion::fullspan(view.rect, 1));
        return Ok(sorted);
    }

    // C 行 824: 第一层
    find_columns_in_view(&mut sorted, &view, settings, 1)?;

    // C 行 825-850: 后续层迭代细分
    for ilevel in 2..=settings.max_columns {
        let level_u8 = ilevel as u8;
        let mut idx = 0usize;
        while idx < sorted.regions.len() {
            let cur = sorted.regions[idx];
            if cur.level == level_u8 - 1 && !cur.fullspan && !cur.notes {
                // 子分这块
                let sub_view = view.with_rect(cur.rect);
                let mut sub_regions = PageRegions::new();
                find_columns_in_view(&mut sub_regions, &sub_view, settings, level_u8)?;

                if sub_regions.regions.is_empty() {
                    idx += 1;
                    continue;
                }

                // C 行 845-846: 删 cur，插入 sub_regions（C 是 delete_one+insert）
                sorted.regions.remove(idx);
                for (i, sr) in sub_regions.regions.iter().enumerate() {
                    sorted.regions.insert(idx + i, *sr);
                }
                // 不递增 idx 让外层重新检查（C 行 847 `j--`）
            } else {
                idx += 1;
            }
        }
    }

    Ok(sorted)
}

/// 单层列检测 —— 对应 C 版 `pageregions_find_next_level`。
///
/// 用 [`find_multicolumn_divider_simple`] 在 `view` 中寻找列分隔符。
fn find_columns_in_view(
    sorted: &mut PageRegions,
    view: &RegionView,
    settings: &ColumnSettings,
    level: u8,
) -> Result<(), CropError> {
    // 预算 row_black_count / col_black_count（整张 bitmap 的索引）
    let bmp_w = view.bmp.width as usize;
    let bmp_h = view.bmp.height as usize;

    let mut row_black = vec![0i32; bmp_h];
    for r in view.rect.y0.max(0)..=view.rect.y1.min(bmp_h as i32 - 1) {
        row_black[r as usize] = row_black_count(view, r);
    }
    let mut col_black = vec![0i32; bmp_w];
    for c in view.rect.x0.max(0)..=view.rect.x1.min(bmp_w as i32 - 1) {
        col_black[c as usize] = col_black_count(view, c);
    }

    // 简化版主算法：单次尝试，不像 C 版那样按 textrow 切片迭代
    let split = find_multicolumn_divider_simple(view, settings, &row_black, &col_black)?;

    if let Some((left, right)) = split {
        // C 行 1995-2001: 添加左右两列（按阅读方向）
        let (first, second) = if settings.src_left_to_right {
            (left, right)
        } else {
            (right, left)
        };
        sorted.push(PageRegion::column(first, level));
        sorted.push(PageRegion::column(second, level));
    } else {
        // C 行 2206-2207: 无法切分 → fullspan
        sorted.push(PageRegion::fullspan(view.rect, level));
    }
    Ok(())
}

/// 简化版列分隔搜索 —— 对应 C 版 `bmpregion_find_multicolumn_divider`
/// 但不依赖 textrow 边界。
///
/// 算法：
///
/// 1. `min_col_gap_pixels = (min_column_gap_inches × dpi + 0.5)`
/// 2. `dm = 1 + (width × column_gap_range / 2)` —— 搜索半径
/// 3. `middle = width / 2`
/// 4. 在 `[middle - dm, middle + dm]` 范围内迭代候选 shaft 起点 `c0`
/// 5. 对每个 `c0`，测试 shaft `[c0..c0+min_col_gap_pixels-1] × [r0..r1]` 是否 clear
/// 6. 找到首个 clear shaft → 调用 [`column_height_and_gap_test`] 验证两侧
/// 7. 通过 → 返回 trim 后的左右两列 [`Rect`]；不通过 → 继续找
///
/// 返回 `None` 表示未找到合法分隔，调用方应将整个 view 视为 fullspan。
fn find_multicolumn_divider_simple(
    region: &RegionView,
    settings: &ColumnSettings,
    row_black: &[i32],
    col_black: &[i32],
) -> Result<Option<(Rect, Rect)>, CropError> {
    let width = region.rect.width() as i32;
    let height = region.rect.height() as i32;
    if width < 8 || height < 4 {
        return Ok(None);
    }

    // C 行 1879 — shaft 宽度由 try_shaft 内部计算并复用，此处仅校验有效性
    let min_col_gap_pixels = (settings.min_column_gap_inches * region.dpi as f64 + 0.5) as i32;
    if min_col_gap_pixels.max(1) < 1 {
        return Ok(None);
    }

    // C 行 1865
    let min_height_pixels = (settings.min_column_height_inches * region.dpi as f64) as i32;
    if height < min_height_pixels {
        return Ok(None);
    }

    // C 行 1876-1877
    let dm = (1.0 + width as f64 * settings.column_gap_range / 2.0) as i32;
    let dm = dm.max(1);
    let middle = width / 2;

    // 中心向两侧交替展开搜索（C 行 1991/2029: c1 = middle - i 然后 middle + i）
    // 简化版按 i = 0..dm 顺序扫两遍
    for i in 0..dm {
        // 尝试中点 - i
        let c0 = region.rect.x0 + middle - i;
        if let Some(result) = try_shaft(region, settings, row_black, col_black, c0)? {
            return Ok(Some(result));
        }
        if i == 0 {
            continue; // i=0 时左右是同一个 c0
        }
        // 尝试中点 + i
        let c0 = region.rect.x0 + middle + i;
        if let Some(result) = try_shaft(region, settings, row_black, col_black, c0)? {
            return Ok(Some(result));
        }
    }
    Ok(None)
}

/// 测试单个候选 shaft 起点 `c0`：shaft 是否 clear + 两侧高度是否达标。
///
/// 通过 → 返回 `(left_rect, right_rect)`；不通过 → `None`。
fn try_shaft(
    region: &RegionView,
    settings: &ColumnSettings,
    row_black: &[i32],
    col_black: &[i32],
    c0: i32,
) -> Result<Option<(Rect, Rect)>, CropError> {
    let min_col_gap_pixels = (settings.min_column_gap_inches * region.dpi as f64 + 0.5) as i32;
    let min_col_gap_pixels = min_col_gap_pixels.max(1);

    // shaft 越界检查
    if c0 < region.rect.x0 || c0 > region.rect.x1 {
        return Ok(None);
    }
    let c1 = (c0 + min_col_gap_pixels - 1).min(region.rect.x1);

    // 构造 shaft view
    let shaft_view = region.with_rect(Rect::new(c0, region.rect.y0, c1, region.rect.y1));
    let clear = is_clear(
        &shaft_view,
        Some(row_black),
        Some(col_black),
        settings.gtc_in,
    );
    if !clear.is_clear() {
        return Ok(None);
    }

    // 列高度 + gap 测试（C 行 2096-2114）
    let divider_column = c0 + min_col_gap_pixels / 2;
    let (left, right, status) = column_height_and_gap_test(
        region,
        region.rect.y0,
        region.rect.y1,
        divider_column,
        settings,
    )?;

    if !status.is_ok() {
        return Ok(None);
    }

    Ok(Some((left, right)))
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

    use super::*;
    use k2types::PixelFormat;

    fn make_white_gray(w: u32, h: u32, dpi: f32) -> Bitmap {
        let mut bmp = Bitmap::new(w, h, dpi, PixelFormat::Gray8).unwrap();
        bmp.fill_byte(255);
        bmp
    }

    /// 画一个填充矩形（黑色）到 Gray8 位图。
    fn paint_rect(bmp: &mut Bitmap, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..=y1.min(bmp.height - 1) {
            for x in x0..=x1.min(bmp.width - 1) {
                if let Some(px) = bmp.pixel_mut(x, y) {
                    px[0] = 0;
                }
            }
        }
    }

    // -------------------- PageRegion / PageRegions --------------------

    #[test]
    fn pageregion_fullspan_constructor() {
        let r = PageRegion::fullspan(Rect::new(0, 0, 99, 99), 1);
        assert!(r.fullspan);
        assert_eq!(r.level, 1);
        assert!(!r.notes);
    }

    #[test]
    fn pageregion_column_constructor() {
        let r = PageRegion::column(Rect::new(0, 0, 49, 99), 2);
        assert!(!r.fullspan);
        assert_eq!(r.level, 2);
    }

    #[test]
    fn pageregions_push_len_empty() {
        let mut p = PageRegions::new();
        assert!(p.is_empty());
        assert_eq!(p.len(), 0);
        p.push(PageRegion::fullspan(Rect::new(0, 0, 9, 9), 1));
        assert_eq!(p.len(), 1);
        assert!(!p.is_empty());
    }

    // -------------------- ColumnSettings --------------------

    #[test]
    fn column_settings_default_matches_c_init() {
        let s = ColumnSettings::default();
        assert_eq!(s.max_columns, 2);
        assert!((s.min_column_gap_inches - 0.1).abs() < 1e-9);
        assert!((s.max_column_gap_inches - 1.5).abs() < 1e-9);
        assert!((s.min_column_height_inches - 1.5).abs() < 1e-9);
        assert!((s.column_gap_range - 0.33).abs() < 1e-9);
        assert!((s.gtc_in - 0.005).abs() < 1e-9);
        assert!(s.src_left_to_right);
        assert!(s.src_trim);
    }

    // -------------------- row_black_count / col_black_count --------------------

    #[test]
    fn row_black_count_all_white_returns_zero() {
        let bmp = make_white_gray(20, 10, 150.0);
        let view = RegionView::full(&bmp);
        for r in 0..10 {
            assert_eq!(row_black_count(&view, r), 0, "row {} all white", r);
        }
    }

    #[test]
    fn row_black_count_counts_dark_pixels() {
        let mut bmp = make_white_gray(20, 10, 150.0);
        paint_rect(&mut bmp, 5, 3, 14, 3); // 10 black pixels on row 3
        let view = RegionView::full(&bmp);
        assert_eq!(row_black_count(&view, 3), 10);
        assert_eq!(row_black_count(&view, 2), 0);
        assert_eq!(row_black_count(&view, 4), 0);
    }

    #[test]
    fn col_black_count_counts_dark_pixels() {
        let mut bmp = make_white_gray(20, 10, 150.0);
        paint_rect(&mut bmp, 7, 0, 7, 9); // entire column 7 black
        let view = RegionView::full(&bmp);
        assert_eq!(col_black_count(&view, 7), 10);
        assert_eq!(col_black_count(&view, 6), 0);
        assert_eq!(col_black_count(&view, 8), 0);
    }

    #[test]
    fn col_black_count_out_of_bounds_returns_zero() {
        let bmp = make_white_gray(5, 5, 150.0);
        let view = RegionView::full(&bmp);
        assert_eq!(col_black_count(&view, -1), 0);
        assert_eq!(col_black_count(&view, 5), 0);
    }

    // -------------------- is_clear --------------------

    #[test]
    fn is_clear_all_white_returns_clear() {
        let bmp = make_white_gray(40, 40, 150.0);
        let view = RegionView::full(&bmp);
        let status = is_clear(&view, None, None, 0.005);
        assert!(status.is_clear());
    }

    #[test]
    fn is_clear_with_dense_black_returns_dirty() {
        let mut bmp = make_white_gray(40, 40, 150.0);
        paint_rect(&mut bmp, 0, 0, 39, 39); // 全黑
        let view = RegionView::full(&bmp);
        let status = is_clear(&view, None, None, 0.005);
        assert_eq!(status, ClearStatus::Dirty);
    }

    #[test]
    fn is_clear_threshold_pt_zero_means_no_dark_allowed() {
        let mut bmp = make_white_gray(40, 40, 150.0);
        paint_rect(&mut bmp, 20, 20, 20, 20); // 单个黑像素
        let view = RegionView::full(&bmp);
        // gtc_in=0 → pt=0 → 任何暗像素都不允许 dirty
        let status = is_clear(&view, None, None, 0.0);
        assert_eq!(status, ClearStatus::Dirty);
    }

    #[test]
    fn is_clear_uses_row_black_optimization() {
        let bmp = make_white_gray(30, 30, 150.0);
        let view = RegionView::full(&bmp);
        let row_black: Vec<i32> = (0..30).map(|r| row_black_count(&view, r)).collect();
        let col_black: Vec<i32> = (0..30).map(|c| col_black_count(&view, c)).collect();
        let status = is_clear(&view, Some(&row_black), Some(&col_black), 0.005);
        assert!(status.is_clear());
    }

    #[test]
    fn is_clear_empty_region_returns_clear() {
        let bmp = make_white_gray(10, 10, 150.0);
        let view = RegionView::new(&bmp, Rect::new(5, 5, 4, 4)); // 空
        let status = is_clear(&view, None, None, 0.005);
        assert!(status.is_clear());
    }

    // -------------------- column_height_and_gap_test --------------------

    #[test]
    fn column_height_test_two_columns_pass() {
        let mut bmp = make_white_gray(400, 600, 150.0);
        // 两列内容，列宽 150，列间隙 100
        // 左列 (50..199), 右列 (250..399), gap (200..249)
        // 各列高度 600 / 150 = 4 inches > 1.5 inches min
        for y in (50..550).step_by(20) {
            paint_rect(&mut bmp, 50, y, 199, y + 10);
            paint_rect(&mut bmp, 250, y, 399, y + 10);
        }
        let view = RegionView::full(&bmp);
        let settings = ColumnSettings::default();
        let (left, right, status) =
            column_height_and_gap_test(&view, 0, 599, 225, &settings).unwrap();
        assert!(status.is_ok(), "expected ok, got {:?}", status);
        assert!(left.x1 < 225);
        assert!(right.x0 >= 225);
    }

    #[test]
    fn column_height_test_short_column_fails() {
        let mut bmp = make_white_gray(400, 600, 150.0);
        // 左列高度只有 50 像素，远小于 min (1.5 inch * 150 = 225 像素)
        paint_rect(&mut bmp, 50, 100, 199, 149);
        // 右列高度 500 像素，足够
        for y in (50..550).step_by(20) {
            paint_rect(&mut bmp, 250, y, 399, y + 10);
        }
        let view = RegionView::full(&bmp);
        let settings = ColumnSettings::default();
        let (_left, _right, status) =
            column_height_and_gap_test(&view, 0, 599, 225, &settings).unwrap();
        assert!(
            status.has(ColumnTestStatus::COL0_TOO_SHORT),
            "expected COL0_TOO_SHORT, got {:?}",
            status
        );
    }

    // -------------------- find_columns 主入口 --------------------

    #[test]
    fn find_columns_max_one_returns_fullspan() {
        let bmp = make_white_gray(800, 1000, 150.0);
        let settings = ColumnSettings {
            max_columns: 1,
            ..ColumnSettings::default()
        };
        let regions = find_columns(&bmp, &settings).unwrap();
        assert_eq!(regions.len(), 1);
        assert!(regions.regions[0].fullspan);
        assert_eq!(regions.regions[0].level, 1);
    }

    #[test]
    fn find_columns_single_column_not_split() {
        let mut bmp = make_white_gray(800, 1000, 150.0);
        // 单列文本，从左到右占满
        for y in (50..950).step_by(30) {
            paint_rect(&mut bmp, 100, y, 700, y + 15);
        }
        let settings = ColumnSettings::default();
        let regions = find_columns(&bmp, &settings).unwrap();
        // 应该是 fullspan
        assert_eq!(regions.len(), 1, "single column should NOT be split");
        assert!(regions.regions[0].fullspan);
    }

    #[test]
    fn find_columns_two_columns_split() {
        let mut bmp = make_white_gray(800, 1200, 150.0);
        // 两列，列宽 300，间隙 100
        // 左列 100..399, 右列 500..799, gap 400..499
        for y in (50..1150).step_by(30) {
            paint_rect(&mut bmp, 100, y, 399, y + 15);
            paint_rect(&mut bmp, 500, y, 799, y + 15);
        }
        let settings = ColumnSettings::default();
        let regions = find_columns(&bmp, &settings).unwrap();
        assert_eq!(
            regions.len(),
            2,
            "expected 2 columns, got {}: {:?}",
            regions.len(),
            regions.regions
        );
        // 两个 region 不应水平重叠
        let r0 = regions.regions[0].rect;
        let r1 = regions.regions[1].rect;
        assert!(
            r0.x1 < r1.x0 || r1.x1 < r0.x0,
            "columns should not overlap: r0={r0:?} r1={r1:?}"
        );
        // 左到右顺序：r0 在左、r1 在右
        assert!(
            r0.x1 < r1.x0,
            "left-to-right order broken: {r0:?} -> {r1:?}"
        );
    }

    #[test]
    fn find_columns_right_to_left_swaps_order() {
        let mut bmp = make_white_gray(800, 1200, 150.0);
        for y in (50..1150).step_by(30) {
            paint_rect(&mut bmp, 100, y, 399, y + 15);
            paint_rect(&mut bmp, 500, y, 799, y + 15);
        }
        let settings = ColumnSettings {
            src_left_to_right: false,
            ..ColumnSettings::default()
        };
        let regions = find_columns(&bmp, &settings).unwrap();
        assert_eq!(regions.len(), 2);
        // 右到左顺序：第一个 region 应该是右列
        assert!(
            regions.regions[0].rect.x0 > regions.regions[1].rect.x1,
            "right-to-left: first region should be the right column"
        );
    }

    #[test]
    fn find_columns_blank_page_fullspan() {
        let bmp = make_white_gray(800, 1000, 150.0);
        let settings = ColumnSettings::default();
        let regions = find_columns(&bmp, &settings).unwrap();
        // 全白页应该 fullspan（没有列）
        assert_eq!(regions.len(), 1);
        assert!(regions.regions[0].fullspan);
    }

    #[test]
    fn find_columns_three_columns_split() {
        let mut bmp = make_white_gray(1200, 1500, 150.0);
        // 三列，列宽 320，间隙 120
        // col0: 100..419, col1: 540..859, col2: 980..1199; gaps 420..539 + 860..979
        for y in (50..1450).step_by(30) {
            paint_rect(&mut bmp, 100, y, 419, y + 15);
            paint_rect(&mut bmp, 540, y, 859, y + 15);
            paint_rect(&mut bmp, 980, y, 1199, y + 15);
        }
        let settings = ColumnSettings {
            max_columns: 3,
            ..ColumnSettings::default()
        };
        let regions = find_columns(&bmp, &settings).unwrap();
        // 期望切出 ≥3 个不重叠区域
        assert!(
            regions.len() >= 3,
            "expected at least 3 regions, got {}: {:?}",
            regions.len(),
            regions.regions
        );
        // 任意两个 region 按 x 排序后不应水平重叠
        let mut rects: Vec<_> = regions.regions.iter().map(|r| r.rect).collect();
        rects.sort_by_key(|r| r.x0);
        for i in 1..rects.len() {
            assert!(
                rects[i - 1].x1 < rects[i].x0,
                "regions overlap: {:?}",
                rects
            );
        }
    }

    // -------------------- ColumnTestStatus enum --------------------

    #[test]
    fn column_test_status_bit_flags() {
        let ok = ColumnTestStatus(0);
        assert!(ok.is_ok());
        assert!(!ok.has(ColumnTestStatus::COL0_TOO_SHORT));

        let mixed =
            ColumnTestStatus(ColumnTestStatus::COL0_TOO_SHORT | ColumnTestStatus::GAP_TOO_WIDE);
        assert!(!mixed.is_ok());
        assert!(mixed.has(ColumnTestStatus::COL0_TOO_SHORT));
        assert!(mixed.has(ColumnTestStatus::GAP_TOO_WIDE));
        assert!(!mixed.has(ColumnTestStatus::COL1_TOO_SHORT));
    }

    #[test]
    fn clear_status_level_works() {
        assert_eq!(ClearStatus::Dirty.level(), 0);
        assert_eq!(ClearStatus::Clear(5).level(), 5);
        assert!(ClearStatus::Clear(1).is_clear());
        assert!(!ClearStatus::Dirty.is_clear());
    }
}
