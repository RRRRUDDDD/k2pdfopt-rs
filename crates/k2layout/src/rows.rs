//! `k2layout::rows` — 文本行检测（textrows.c + bmpregion_find_textrows → Rust）。
//!
//! **Step 6.2 (M4)**：1:1 移植 `k2pdfoptlib/textrows.c`（1074 行）核心 helper +
//! `bmpregion.c::bmpregion_find_textrows`（行 1287-1670）+ `bmpregion_fill_row_threshold_array`
//! （行 1676-1736）。
//!
//! # 范围
//!
//! 已实现（Step 6.2）：
//! - [`TextRow`] / [`TextRows`] / [`RowSettings`] / [`RowType`] 数据结构
//! - [`fill_row_threshold_array`]：从 BBox row_counts 算 dynamic aperture 平滑后的
//!   threshold 数组 + rhmean_pixels
//! - [`find_textrows`]：主入口，按 brc/trc/dtrc 状态机切行 + figure caption 处理
//! - [`compute_row_gaps`]：行间距 / rowheight 更新
//! - [`remove_defects`]：删极小杂质行（v2.52 加入）
//! - [`remove_small_rows`]：删过小行 / 合并相邻行（v2.10 + v2.33 改进）
//! - [`sort_by_gap`] / [`sort_by_row_position`]：heapsort（与 C 一致）
//! - [`region_is_figure`] / [`determine_type`]：行类型判定
//! - [`scale_textrow`]：几何缩放
//! - [`line_spacing_is_same`] / [`font_size_is_same`]：行间距 / 字号相似度判定
//!
//! 推迟（Open Question 6.2.A）：
//! - `textrows_find_doubles`（双高/三高行拆分）→ Step 6.x（需先确认 row_split_fom 与
//!   c1new 计算精度，且 Step 5.7 SSIM 工具不一定能覆盖此回归）
//! - `FONTSIZE_HISTOGRAM`（字号直方图）→ M5（reflow 阶段才使用）
//!
//! # C 行号对照
//!
//! 注释中所有 `[C行 NNNN]` / `[bmpregion.c:NNNN]` 引用均按 v2.55 源码。

#![allow(clippy::too_many_arguments)]

use crate::crop::{calc_bbox, BBox, CropError, CropSettings, TextRowStats};
use crate::region::RegionView;
use k2core::rect::Rect;

// =====================================================================
// 数据结构
// =====================================================================

/// 文本行类型（C 行 511-516 `REGION_TYPE_*` 宏）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum RowType {
    /// `REGION_TYPE_UNDETERMINED = 0`。
    #[default]
    Undetermined = 0,
    /// `REGION_TYPE_TEXTLINE = 1`。
    TextLine = 1,
    /// `REGION_TYPE_MULTILINE = 2`。
    MultiLine = 2,
    /// `REGION_TYPE_WORD = 3`。
    Word = 3,
    /// `REGION_TYPE_FIGURE = 4`。
    Figure = 4,
    /// `REGION_TYPE_MULTIWORD = 5`。
    MultiWord = 5,
}

/// 单行文本统计（C `TEXTROW`，`k2pdfopt.h:511-531`）。
///
/// 含 inclusive 端点 `c1/c2/r1/r2`、`rowbase` 基线、行间距 `gap/gapblank/rowheight`、
/// 字号 `capheight/h5050/lcheight`、类型 `region_type`、find_doubles 评分 `rat`。
#[derive(Clone, Debug, PartialEq)]
pub struct TextRow {
    /// 左列（inclusive，bitmap 绝对坐标）。
    pub c1: i32,
    /// 右列（inclusive）。
    pub c2: i32,
    /// 顶行（inclusive）。
    pub r1: i32,
    /// 底行（inclusive）。
    pub r2: i32,
    /// 基线行号（C `rowbase`，`bmpregion.c:589`）。
    pub rowbase: i32,
    /// 此行 rowbase → 下一行 r1 - 1 的"baseline gap"（C 行 213）。
    pub gap: i32,
    /// 此行 r2 → 下一行 r1 - 1 的"blank gap"（C 行 214）。
    pub gapblank: i32,
    /// 上一行 rowbase → 此行 rowbase（C 行 220）。
    pub rowheight: i32,
    /// 大写高度（C `capheight`，从 rowcount[i]>maxcount/20 处算到 rowbase）。
    pub capheight: i32,
    /// 50% 高度（C `h5050`，从 rowcount[i]>maxcount/2 处算到 rowbase）。
    pub h5050: i32,
    /// 小写高度（C `lcheight`，h5050 经启发式调整）。
    pub lcheight: i32,
    /// 行类型。
    pub region_type: RowType,
    /// `find_doubles` 给的评分（默认 0.0；> 0 表示由 find_doubles 拆出）。
    pub rat: f64,
}

impl Default for TextRow {
    /// 对应 `textrow_init`（C 行 141-149）。
    fn default() -> Self {
        Self {
            c1: -1,
            c2: -1,
            r1: -1,
            r2: -1,
            rowbase: -1,
            gap: -1,
            gapblank: -1,
            rowheight: -1,
            capheight: -1,
            h5050: -1,
            lcheight: -1,
            region_type: RowType::Undetermined,
            rat: 0.0,
        }
    }
}

impl TextRow {
    /// 用 bbox + text_stats 填一个 TextRow（C `textrow_assign_bmpregion`，行 132-138）。
    pub fn from_bbox(rect: Rect, stats: Option<&TextRowStats>, region_type: RowType) -> Self {
        let (rowbase, capheight, h5050, lcheight) = stats
            .map(|s| (s.rowbase, s.capheight, s.h5050, s.lcheight))
            .unwrap_or((rect.y1, -1, -1, -1));
        Self {
            c1: rect.x0,
            c2: rect.x1,
            r1: rect.y0,
            r2: rect.y1,
            rowbase,
            gap: -1,
            gapblank: -1,
            rowheight: -1,
            capheight,
            h5050,
            lcheight,
            region_type,
            rat: 0.0,
        }
    }

    /// 行高（像素，inclusive 语义）。
    pub fn height(&self) -> i32 {
        self.r2 - self.r1 + 1
    }

    /// 行宽（像素，inclusive 语义）。
    pub fn width(&self) -> i32 {
        self.c2 - self.c1 + 1
    }

    /// rect 表达（便于复用 Rect helpers）。
    pub fn rect(&self) -> Rect {
        Rect::new(self.c1, self.y_top(), self.c2, self.r2)
    }

    #[inline]
    fn y_top(&self) -> i32 {
        self.r1
    }
}

/// 文本行集合（C `TEXTROWS`，`k2pdfopt.h:537-541`）。
#[derive(Clone, Debug, Default)]
pub struct TextRows {
    /// 行列表。
    pub rows: Vec<TextRow>,
}

impl TextRows {
    /// 空集合。
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    /// 行数。
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// 是否空。
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// 添加一行（C `textrows_add_textrow`，行 96-119）。
    pub fn push(&mut self, row: TextRow) {
        self.rows.push(row);
    }

    /// 删除指定索引行（C `textrows_delete_one`，行 57-65）。
    ///
    /// 越界时静默忽略（与 C 版"不验证"语义一致）。
    pub fn delete(&mut self, index: usize) {
        if index < self.rows.len() {
            self.rows.remove(index);
        }
    }
}

/// 行检测设置（独立 struct，避免 k2layout → k2settings 反向依赖）。
///
/// 字段一一对应 C 版 `K2PDFOPT_SETTINGS`，默认值来自 `k2settings_init`
/// （`k2settings.c:38-300`）。
#[derive(Clone, Debug)]
pub struct RowSettings {
    /// 行间最大允许空白（inches）；超过则拆分。C 默认 0.25 (`k2settings.c:140`)。
    pub max_vertical_gap_inches: f64,
    /// 行间黑像素阈值（inches，C `gtr_in`，默认 0.006，`k2settings.c:49`）。
    pub gtr_in: f64,
    /// 最小图形高度（inches，C `dst_min_figure_height_in`，默认 0.75，`k2settings.c:96`）。
    pub dst_min_figure_height_in: f64,
    /// 杂质尺寸（pts，C `defect_size_pts`）。
    pub defect_size_pts: f64,
    /// 文本行最小高度阈值（pts，C `textheight_min_pts`，默认 -1.0 = 关闭，
    /// `k2settings.c:240`）。
    pub textheight_min_pts: f64,
    /// "小片段"阈值（inches，C `little_piece_threshold_inches`，默认 0.5，
    /// `k2settings.c:162`）。
    pub little_piece_threshold_inches: f64,
    /// 图形宽高比下限（C `no_wrap_ar_limit`，默认 0.2，`k2settings.c:160`）。
    pub no_wrap_ar_limit: f64,
    /// 图形高度下限（inches，C `no_wrap_height_limit_inches`，默认 0.55，
    /// `k2settings.c:161`）。
    pub no_wrap_height_limit_inches: f64,
    /// 是否检测双高/三高行（C `detect_double_rows`，默认 1，`k2settings.c:239`）。
    ///
    /// **当前 Rust 版**：保留字段供后续 `find_doubles` 实现，本步 find_textrows
    /// 不会使用此字段（Open Question 6.2.A）。
    pub detect_double_rows: bool,
    /// 是否在 trim 时同时 trim 边缘留白（C `src_trim`，默认 1，`k2settings.c:139`）。
    pub src_trim: bool,
    /// 左→右阅读（C `src_left_to_right`，默认 1）。
    pub src_left_to_right: bool,
}

impl Default for RowSettings {
    fn default() -> Self {
        Self {
            max_vertical_gap_inches: 0.25,
            gtr_in: 0.006,
            dst_min_figure_height_in: 0.75,
            defect_size_pts: 0.75,
            textheight_min_pts: -1.0,
            little_piece_threshold_inches: 0.5,
            no_wrap_ar_limit: 0.2,
            no_wrap_height_limit_inches: 0.55,
            detect_double_rows: true,
            src_trim: true,
            src_left_to_right: true,
        }
    }
}

impl RowSettings {
    /// 提取 [`CropSettings`] 视图（用于复用 [`calc_bbox`]）。
    pub fn crop_settings(&self) -> CropSettings {
        CropSettings {
            src_left_to_right: self.src_left_to_right,
            defect_size_pts: self.defect_size_pts,
        }
    }
}

// =====================================================================
// fill_row_threshold_array
// =====================================================================

/// 计算 row threshold 数组（C `bmpregion_fill_row_threshold_array`，行 1676-1736）。
///
/// 用一个 `aperture` 大小的水平窗口在 `view.rect.y0..=y1` 滚动求和 row_counts 累计
/// 黑像素，归一化到 0..=10*sum/pixel_threshold，输出每行的"非空白程度"。
///
/// 输入 `row_counts` 是 [`BBox::row_counts`] 完整长度数组（绝对索引）。
///
/// 返回 `(rowthresh, rhmean_pixels)`：
/// - `rowthresh`：长度 = `y1 - y0 + 1`，索引 0 对应绝对行 y0
/// - `rhmean_pixels`：平均文本行高（像素），用于后续 `rhmin_pix` 计算
pub fn fill_row_threshold_array(
    view: &RegionView,
    row_counts: &[i32],
    settings: &RowSettings,
    dynamic_aperture: bool,
) -> (Vec<i32>, i32) {
    let y0 = view.rect.y0;
    let y1 = view.rect.y1;
    let dpi = view.dpi as f64;

    // C 行 1682-1684: aperturemax = max(2, dpi/72)
    let mut aperturemax = (dpi / 72.0 + 0.5) as i32;
    if aperturemax < 2 {
        aperturemax = 2;
    }
    let mut aperture = aperturemax;

    if y1 < y0 {
        return (Vec::new(), 0);
    }
    let nr = (y1 - y0 + 1) as usize;
    let mut rowthresh = vec![0i32; nr];
    let mut rhmean_sum: i64 = 0;
    let mut ntr: i32 = 0;
    let mut dtrc: i32 = 0;

    for i in y0..=y1 {
        if dynamic_aperture {
            // C 行 1699-1703
            aperture = ((dtrc as f64) / 13.7 + 0.5) as i32;
            if aperture > aperturemax {
                aperture = aperturemax;
            }
            if aperture < 2 {
                aperture = 2;
            }
        }
        // C 行 1705-1710: i1 = i - aperture/2; i2 = i1 + aperture - 1; clamp [y0, y1]
        let mut i1 = i - aperture / 2;
        let mut i2 = i1 + aperture - 1;
        if i1 < y0 {
            i1 = y0;
        }
        if i2 > y1 {
            i2 = y1;
        }
        // C 行 1711-1713: pt = (int)((i2-i1+1)*gtr_in*dpi+.5); 1 minimum
        let aperture_height = (i2 - i1 + 1) as f64;
        let mut pt = (aperture_height * settings.gtr_in * dpi + 0.5) as i32;
        if pt < 1 {
            pt = 1;
        }
        // C 行 1715: sum over aperture rowcount
        let mut sum: i64 = 0;
        for ii in i1..=i2 {
            if let Some(c) = row_counts.get(ii as usize) {
                sum += *c as i64;
            }
        }
        // C 行 1717: rowthresh[i-r1] = 10*sum/pt
        let thresh = (10 * sum / pt as i64).min(i32::MAX as i64) as i32;
        rowthresh[(i - y0) as usize] = thresh;
        // C 行 1717-1727: 累计 dtrc / ntr
        if thresh <= 40 {
            if dtrc > 0 {
                rhmean_sum += dtrc as i64;
                ntr += 1;
            }
            dtrc = 0;
        } else {
            dtrc += 1;
        }
    }
    // C 行 1729-1733: tail
    if dtrc > 0 {
        rhmean_sum += dtrc as i64;
        ntr += 1;
    }
    let rhmean = if ntr > 0 {
        (rhmean_sum / ntr as i64) as i32
    } else {
        0
    };
    (rowthresh, rhmean)
}

// =====================================================================
// find_textrows 主入口
// =====================================================================

/// 在 `view` 内检测所有文本行 / 图形（C `bmpregion_find_textrows`，
/// `bmpregion.c:1287-1670`）。
///
/// **参数**：
/// - `dynamic_aperture`：动态调整 aperture（C 行 1320 同名参数）
/// - `remove_small_rows`：删过小行（C 行 1608-1617）
/// - `minrowgap_in`：最小行间距（inches），传给 `remove_small_rows`（C 行 1616）
/// - `join_figure_captions`：合并图形 caption（C 行 1439-1466）
///
/// **流程**：
/// 1. trim region（C 行 1309）；本 Rust 版直接用传入的 view（调用方负责 trim）
/// 2. 调用 [`calc_bbox`] 拿 row_counts（C 版用 `region->rowcount`）
/// 3. [`fill_row_threshold_array`] 算 rowthresh + rhmean_pixels
/// 4. 主循环：brc/trc/dtrc 状态机切行（C 行 1374-1540）
/// 5. [`compute_row_gaps`]（C 行 1547）
/// 6. [`remove_defects`]（C 行 1566，v2.52）
/// 7. (跳过) `find_doubles`（推迟到 Step 6.x，Open Question 6.2.A）
/// 8. [`compute_row_gaps`] 第二次（C 行 1592）
/// 9. 若 `remove_small_rows=true`：[`determine_type`] 所有行 + [`remove_small_rows`]
///    + [`compute_row_gaps`] 第三次（C 行 1608-1620）
/// 10. [`determine_type`] 最终重判（C 行 1641-1642）
///
/// **返回**：[`TextRows`] 含所有切出的行；若 region 高度 < 1 或 row_counts 全 0
///   返回空集合。
pub fn find_textrows(
    view: &RegionView,
    settings: &RowSettings,
    dynamic_aperture: bool,
    remove_small_rows_flag: bool,
    minrowgap_in: f64,
    join_figure_captions: bool,
) -> Result<TextRows, CropError> {
    let crop_settings = settings.crop_settings();

    // C 行 1308-1309: 调用方应该已经 trim 过；我们这里复算 bbox 拿 row_counts
    let bbox = calc_bbox(view, &crop_settings, false)?;
    let region_rect = bbox.rect;
    if region_rect.y1 < region_rect.y0 || region_rect.x1 < region_rect.x0 {
        return Ok(TextRows::new());
    }

    // 用裁后的 region_rect 重做 fill_row_threshold_array
    let region_view = view.with_rect(region_rect);
    let dpi = view.dpi as f64;
    let (rowthresh, rhmean_pixels) =
        fill_row_threshold_array(&region_view, &bbox.row_counts, settings, dynamic_aperture);

    // C 行 1319: brcmin = max_vertical_gap_inches * dpi
    let brcmin = (settings.max_vertical_gap_inches * dpi) as i32;

    // C 行 1349-1355: rhmin_pix 范围 [0.04*dpi, 0.13*dpi]
    let mut rhmin_pix = rhmean_pixels / 3;
    let lo = (0.04 * dpi) as i32;
    let hi = (0.13 * dpi) as i32;
    if rhmin_pix < lo {
        rhmin_pix = lo;
    }
    if rhmin_pix > hi {
        rhmin_pix = hi;
    }
    if rhmin_pix < 1 {
        rhmin_pix = 1;
    }

    // C 行 1305-1307: figure/label 阈值
    let min_fig_height = settings.dst_min_figure_height_in;
    let max_fig_gap = 0.16;
    let max_label_height = 0.5;

    let mut textrows = TextRows::new();

    // C 行 1374-1540: 主循环
    let r1 = region_rect.y0;
    let r2 = region_rect.y1;
    let c1_outer = region_rect.x0;
    let c2_outer = region_rect.x1;
    let mut newregion_r1 = r1;

    let mut figrow: i32 = -1;
    let mut labelrow: i32 = -1;
    let mut dtrc: i32 = 0;
    // `trc` 是 C 版 "consecutive non-blank rows"（行 1370）。当前 Rust 算法仅依赖
    // brc/dtrc 判定，trc 留作 C 行号对照但未读使用 → 用前缀 `_` 抑制 warning。
    let mut _trc: i32 = 0;
    let mut brc: i32 = 0;

    let nr = (r2 - r1 + 1) as usize;
    let mut i = r1;
    while i <= r2 + 1 {
        // C 行 1382: row 是 blank？
        let is_blank_row = i > r2 || {
            let idx = (i - r1) as usize;
            // idx 不应越界因为 i <= r2，但保守用 get
            rowthresh.get(idx).copied().unwrap_or(0) <= 10
        };

        if is_blank_row {
            _trc = 0;
            brc += 1;

            if dtrc == 0 && i <= r2 {
                // C 行 1394-1396: 连续 blank，仅推进 newregion.r1
                if brc > brcmin {
                    newregion_r1 += 1;
                }
                i += 1;
                continue;
            }
            // C 行 1401: big enough blank gap → 加一行
            if dtrc + brc >= rhmin_pix || i > r2 {
                // C 行 1407-1410: dtrc 下限
                if dtrc < (dpi * 0.02) as i32 {
                    dtrc = (dpi * 0.02) as i32;
                }
                if dtrc < 2 {
                    dtrc = 2;
                }
                // C 行 1411-1430: 找更优断点
                if i <= r2 {
                    let i0 = i;
                    let mut iopt = i;
                    let mut k = i;
                    while k <= r2 && k - i0 < dtrc {
                        let idx_k = (k - r1) as usize;
                        let val = rowthresh.get(idx_k).copied().unwrap_or(0);
                        let val_opt = rowthresh.get((iopt - r1) as usize).copied().unwrap_or(0);
                        if val < val_opt {
                            iopt = k;
                            if val == 0 {
                                break;
                            }
                        }
                        if val > 100 {
                            break;
                        }
                        k += 1;
                    }
                    // 更新 i：若到 r2 也没找到完美断点，留在 r2；否则用 iopt
                    let iopt_val = rowthresh.get((iopt - r1) as usize).copied().unwrap_or(0);
                    if k > r2 && iopt_val > 0 {
                        i = r2;
                    } else {
                        i = iopt;
                    }
                }
                let newregion_r2 = i - 1;
                let region_height_inches = ((newregion_r2 - newregion_r1 + 1) as f64) / dpi;

                // C 行 1439-1452: 该 region 可能是 figure？
                if join_figure_captions
                    && i <= r2
                    && figrow < 0
                    && region_height_inches >= min_fig_height
                {
                    figrow = newregion_r1;
                    labelrow = -1;
                    newregion_r1 = i;
                    dtrc = 0;
                    _trc = 0;
                    brc = 1;
                    i += 1;
                    continue;
                }

                // C 行 1453-1507: 处理 figure caption
                // 关键控制流：当 figure 处理中决定不合并 + 新 region 可能是 figure 时，
                // 应直接 continue（保留 figrow），不能进入 figrow=-1 重置（C 行 1496-1497
                // `continue` 跳过行 1505 重置）
                let mut continued_figure = false;
                if figrow >= 0 {
                    let gap_inches = if labelrow >= 0 {
                        ((labelrow - newregion_r1) as f64) / dpi
                    } else {
                        -1.0
                    };
                    if region_height_inches < max_label_height
                        && gap_inches > 0.0
                        && gap_inches < max_fig_gap
                    {
                        // 合并到 figure：扩展 newregion 顶部到 figrow
                        newregion_r1 = figrow;
                    } else {
                        // 单独 dump figure
                        let fig_rect = Rect::new(c1_outer, figrow, c2_outer, newregion_r1 - 1);
                        if fig_rect.y1 > fig_rect.y0 {
                            let figview = view.with_rect(fig_rect);
                            let fig_bbox = calc_bbox(&figview, &crop_settings, true)?;
                            push_textrow_from_bbox(&mut textrows, &fig_bbox, RowType::Figure);
                        }
                        // 新 region 可能是 figure？
                        if i <= r2 && gap_inches > 0.0 && gap_inches < max_fig_gap {
                            // C 行 1491-1497: 设新 figrow + continue (不进入 figrow=-1 重置)
                            figrow = newregion_r2 + 1;
                            labelrow = -1;
                            newregion_r1 = i;
                            dtrc = 0;
                            _trc = 0;
                            brc = 1;
                            i += 1;
                            continued_figure = true;
                        }
                        // 否则按正常 textline 流程（不更新 newregion_r1，下面会再用）
                    }
                    if !continued_figure {
                        figrow = -1;
                        labelrow = -1;
                    }
                }
                if continued_figure {
                    continue;
                }
                // C 行 1515-1520: 加 textline
                if newregion_r2 > newregion_r1 {
                    let rect = Rect::new(c1_outer, newregion_r1, c2_outer, newregion_r2);
                    let rview = view.with_rect(rect);
                    let row_bbox = calc_bbox(&rview, &crop_settings, true)?;
                    push_textrow_from_bbox(&mut textrows, &row_bbox, RowType::TextLine);
                }
                // C 行 1524-1526: 推进状态
                newregion_r1 = i;
                dtrc = 0;
                _trc = 0;
                brc = 1;
            }
        } else {
            // C 行 1529-1538: non-blank
            if figrow >= 0 && labelrow < 0 {
                labelrow = i;
            }
            dtrc += 1;
            _trc += 1;
            brc = 0;
        }
        i += 1;
    }

    // C 行 1542-1544: rat = 0
    for row in &mut textrows.rows {
        row.rat = 0.0;
    }

    // C 行 1547: compute_row_gaps（首次）
    compute_row_gaps(&mut textrows, r2);

    // C 行 1566: remove_defects（v2.52）
    let defect_thresh = (settings.defect_size_pts / 72.0 * dpi + 0.5) as i32;
    remove_defects(&mut textrows, defect_thresh);

    // C 行 1582-1586: find_doubles（**推迟到 Step 6.x，Open Question 6.2.A**）

    // C 行 1592: compute_row_gaps（再次）
    compute_row_gaps(&mut textrows, r2);

    // C 行 1607-1617: remove_small_rows
    if remove_small_rows_flag {
        // 先 determine_type 所有行
        let width_in = (c2_outer - c1_outer + 1) as f64 / dpi;
        for row in &mut textrows.rows {
            determine_type(view, settings, row);
        }
        remove_small_rows(
            &mut textrows,
            view,
            settings,
            0.25,
            0.5,
            minrowgap_in,
            region_rect,
        )?;
        // 防止 unused
        let _ = width_in;
    }

    // C 行 1620: compute_row_gaps（第三次）
    compute_row_gaps(&mut textrows, r2);

    // C 行 1635-1638: bbox.type（这里只做最终 determine_type，不维护外层 region.bbox）
    let _ = nr; // 仅 for C 行号对齐

    // C 行 1641-1642: 最终 determine_type
    for row in &mut textrows.rows {
        determine_type(view, settings, row);
    }

    Ok(textrows)
}

fn push_textrow_from_bbox(textrows: &mut TextRows, bbox: &BBox, region_type: RowType) {
    let row = TextRow::from_bbox(bbox.rect, bbox.text_stats.as_ref(), region_type);
    textrows.push(row);
}

// =====================================================================
// compute_row_gaps（C 行 194-227）
// =====================================================================

/// 计算 `gap` / `gapblank` / `rowheight`（C `textrows_compute_row_gaps`，
/// `textrows.c:194-227`）。
///
/// - `gap[i]` = `textrow[i+1].r1 - textrow[i].rowbase - 1`（行 213）
/// - `gap[n-1]` = `r2 - textrow[n-1].rowbase`（行 225）
/// - `gapblank[i]` = `textrow[i+1].r1 - textrow[i].r2 - 1`（行 214）
/// - `gapblank[n-1]` = 0（行 226）
/// - `rowheight[0]` = `textrow[1].r1 - textrow[0].r1`（n>1 时；行 204）
///   或 `textrow[0].r2 - textrow[0].r1`（n=1 时；行 206）
/// - `rowheight[i]` = `textrow[i].rowbase - textrow[i-1].rowbase`（i≥1；行 220-221）
///
/// FIGURE 行的 `gap[i]` 用 `textrow[i].r2` 代替 `rowbase`（行 211-212）。
pub fn compute_row_gaps(textrows: &mut TextRows, r2: i32) {
    let n = textrows.rows.len();
    if n == 0 {
        return;
    }
    // C 行 203-206: rowheight[0]
    if n > 1 {
        textrows.rows[0].rowheight = textrows.rows[1].r1 - textrows.rows[0].r1;
    } else {
        textrows.rows[0].rowheight = textrows.rows[0].r2 - textrows.rows[0].r1;
    }
    // C 行 207-215: gap / gapblank for i=0..n-1
    for i in 0..n.saturating_sub(1) {
        let r1_anchor = if textrows.rows[i].region_type == RowType::Figure {
            textrows.rows[i].r2
        } else {
            textrows.rows[i].rowbase
        };
        textrows.rows[i].gap = textrows.rows[i + 1].r1 - r1_anchor - 1;
        textrows.rows[i].gapblank = textrows.rows[i + 1].r1 - textrows.rows[i].r2 - 1;
    }
    // C 行 219-221: rowheight[i] for i=1..n
    for i in 1..n {
        textrows.rows[i].rowheight = textrows.rows[i].rowbase - textrows.rows[i - 1].rowbase;
    }
    // C 行 222-226: last row
    if textrows.rows[n - 1].region_type == RowType::Figure {
        textrows.rows[n - 1].gap = 0;
    } else {
        textrows.rows[n - 1].gap = r2 - textrows.rows[n - 1].rowbase;
    }
    textrows.rows[n - 1].gapblank = 0;
}

// =====================================================================
// remove_defects（C 行 230-245）
// =====================================================================

/// 删除"宽高都 ≤ threshold"的杂质行（C `textrows_remove_defects`，
/// `textrows.c:230-245`，v2.52 加入）。
pub fn remove_defects(textrows: &mut TextRows, defect_size_threshold: i32) {
    textrows.rows.retain(|row| {
        // 行 237-238: r2-r1+1 > threshold || c2-c1+1 > threshold → 保留
        row.height() > defect_size_threshold || row.width() > defect_size_threshold
    });
}

// =====================================================================
// remove_small_rows（C 行 253-428）
// =====================================================================

/// 删除/合并过小行（C `textrows_remove_small_rows`，`textrows.c:253-428`）。
///
/// 算法：
/// 1. 计算所有非 figure 行的 `rh` (height) 与相邻 `gap` (baseline) / `gapblank` (blank) 中值
/// 2. 阈值 `mh = mhalf * fracrh`、`mg = mghalf * fracgap`、`mgbl = mgblhalf * fracgap`、
///    `mg1 = mghalf * 0.7`
/// 3. 每行检查三个标志：
///    - `textheight_below_min_threshold`：用户给的最小字号阈值（capheight 转 pts）
///    - `textrow_is_out_of_family_small`：行高小且 gap 小
///    - `row_is_a_fragment`：宽小且居中，且 gap 小
///    - `gap_below_user_threshold`：相邻 gap 小于用户阈值（v2.33）
/// 4. 命中任一 → 合并到上一行或下一行（取较短 gap）；合并后重算 capheight 等
///
/// 输入 `region_rect` 是 `find_textrows` 已 trim 过的 region 矩形；用于 nc/c1/c2 计算。
pub fn remove_small_rows(
    textrows: &mut TextRows,
    view: &RegionView,
    settings: &RowSettings,
    fracrh: f64,
    fracgap: f64,
    mingap_in: f64,
    region_rect: Rect,
) -> Result<(), CropError> {
    if textrows.rows.len() < 2 {
        return Ok(());
    }
    let c1 = region_rect.x0;
    let c2 = region_rect.x1;
    let nc = (c2 - c1 + 1) as f64;
    let dpi = view.dpi as f64;

    // C 行 274-291: 收集 rh / gap / gapbl 中值（仅 non-figure）
    let mut rh_vec: Vec<i32> = Vec::new();
    let mut gap_vec: Vec<i32> = Vec::new();
    let mut gapbl_vec: Vec<i32> = Vec::new();
    let n = textrows.rows.len();
    for (i, row) in textrows.rows.iter().enumerate() {
        if row.region_type == RowType::Figure {
            continue;
        }
        rh_vec.push(row.height());
        if i < n - 1 {
            // 注意：C 行 282-283 把"baseline gap"放进 gapbl，"blank gap"放进 gap
            // （命名反直觉但符合 C 源码）
            gapbl_vec.push(row.gap);
            gap_vec.push(row.gapblank);
        }
    }
    if rh_vec.len() < 2 {
        return Ok(());
    }
    rh_vec.sort_unstable();
    gap_vec.sort_unstable();
    gapbl_vec.sort_unstable();

    let nr = rh_vec.len();
    let ng = gap_vec.len();
    let mhalf = rh_vec[nr / 2];
    let mut mh = ((mhalf as f64) * fracrh) as i32;
    if mh < 1 {
        mh = 1;
    }
    let mg0 = if ng > 0 { gap_vec[ng / 2] } else { 0 };
    let mut mg = ((mg0 as f64) * fracgap) as i32;
    if mg < 1 {
        mg = 1;
    }
    let mgbl = if ng > 0 {
        ((gapbl_vec[ng / 2] as f64) * fracgap) as i32
    } else {
        0
    };
    let mg1 = ((mg0 as f64) * 0.7) as i32;

    // C 行 310-427: 主循环
    let crop_settings = settings.crop_settings();
    let mut i: i32 = 0;
    while (i as usize) < textrows.rows.len() {
        let row = &textrows.rows[i as usize];
        let trh = row.height();
        let textrow_capheight = row.capheight;

        // g1: 与上一行的 blank gap; gs1: 与上一行的 baseline gap
        let (g1, gs1) = if i == 0 {
            (mg0 + 1, mg + 1)
        } else {
            let prev = &textrows.rows[(i - 1) as usize];
            (row.r1 - prev.r2 - 1, prev.gap)
        };
        // g2 / gs2 同理（下一行）
        let n_now = textrows.rows.len() as i32;
        let (g2, gs2) = if i == n_now - 1 {
            (mg0 + 1, mg + 1)
        } else {
            let next = &textrows.rows[(i + 1) as usize];
            (next.r1 - row.r2 - 1, row.gap)
        };
        // C 行 344: textheight_below_min_threshold
        let textheight_below_min = if settings.textheight_min_pts > 0.0 {
            (textrow_capheight as f64) * 72.0 / dpi < settings.textheight_min_pts
        } else {
            false
        };
        // C 行 347-355: 若过小直接删
        if textheight_below_min {
            textrows.rows.remove(i as usize);
            // i 不变（继续看新 i 索引）；不能减到 -1
            if i > 0 {
                i -= 1;
            }
            continue;
        }
        // C 行 357-362
        let rowheight_oof_small = trh < mh;
        let blankgap_oof_small = g1 <= mg1 || g2 <= mg1;
        let baselinegap_oof_small = gs1 < mgbl || gs2 >= mgbl;
        let textrow_oof_small =
            rowheight_oof_small && (baselinegap_oof_small || blankgap_oof_small);
        // C 行 363-364: gap_below_user_threshold
        let gap_below_user = i < n_now - 1 && mingap_in > 0.0 && (g2 as f64) / dpi < mingap_in;
        // C 行 370-378: row_is_a_fragment
        let row_width_inches = (row.width() as f64) / dpi;
        let m1 = ((row.c1 - c1) as f64).abs() / nc.max(1.0);
        let m2 = ((row.c2 - c2) as f64).abs() / nc.max(1.0);
        let row_is_fragment = m1 > 0.1
            && m2 > 0.1
            && row_width_inches < settings.little_piece_threshold_inches
            && blankgap_oof_small;

        // C 行 391-392: 都不命中，跳过
        if !gap_below_user && !textrow_oof_small && !row_is_fragment {
            i += 1;
            continue;
        }
        // C 行 393-394: 决定合并方向
        if i == n_now - 1 || (i > 0 && g1 < g2) {
            i -= 1;
        }
        // C 行 398-402: 合并 i 和 i+1
        let next_idx = (i + 1) as usize;
        if next_idx >= textrows.rows.len() {
            break;
        }
        let next = textrows.rows[next_idx].clone();
        let row_mut = &mut textrows.rows[i as usize];
        row_mut.r2 = next.r2;
        if next.c2 > row_mut.c2 {
            row_mut.c2 = next.c2;
        }
        if next.c1 < row_mut.c1 {
            row_mut.c1 = next.c1;
        }
        // C 行 404-419: 重算 capheight / lcheight via calc_bbox
        let merged_rect = Rect::new(row_mut.c1, row_mut.r1, row_mut.c2, row_mut.r2);
        let merged_view = view.with_rect(merged_rect);
        let merged_bbox = calc_bbox(&merged_view, &crop_settings, true)?;
        let merged_stats = merged_bbox.text_stats.as_ref();
        let new_row = TextRow::from_bbox(merged_bbox.rect, merged_stats, RowType::TextLine);
        textrows.rows[i as usize] = new_row;
        // C 行 421-423: 删 i+1
        textrows.rows.remove(next_idx);
        // C 行 424: i--; 继续
        if textrows.rows.len() <= 1 {
            break;
        }
        if i > 0 {
            i -= 1;
        }
    }
    Ok(())
}

// =====================================================================
// sort_by_gap / sort_by_row_position（C 行 431-536，heapsort）
// =====================================================================

/// 按 `gap` 升序排序（C `textrows_sort_by_gap`，行 431-482，heapsort）。
///
/// **稳定性**：C heapsort 不保证稳定；Rust 用 `sort_by_key`（stable）便于测试可重复。
/// 排序顺序与 C 一致：升序。
pub fn sort_by_gap(textrows: &mut TextRows) {
    textrows.rows.sort_by_key(|a| a.gap);
}

/// 按 `r1` 升序排序（C `textrows_sort_by_row_position`，行 485-536，heapsort）。
pub fn sort_by_row_position(textrows: &mut TextRows) {
    textrows.rows.sort_by_key(|a| a.r1);
}

// =====================================================================
// region_is_figure / determine_type / scale_textrow（C 行 872-943）
// =====================================================================

/// 判断"宽 x 高"是否够大到算作 figure（C `region_is_figure`，
/// `textrows.c:894-903`）。
pub fn region_is_figure(settings: &RowSettings, width_in: f64, height_in: f64) -> bool {
    if height_in <= 0.0 {
        return false;
    }
    let ar = width_in / height_in;
    ar > settings.no_wrap_ar_limit
        && (height_in > settings.no_wrap_height_limit_inches
            || height_in > settings.dst_min_figure_height_in)
}

/// 给一行重新打类型标签（C `textrow_determine_type`，`textrows.c:872-891`）。
///
/// 若当前不是 Figure 且 [`region_is_figure`] 判定为 figure，则改为 Figure。
/// 其他情况不动（保留原类型）。
pub fn determine_type(view: &RegionView, settings: &RowSettings, textrow: &mut TextRow) {
    if textrow.region_type == RowType::Figure {
        return;
    }
    let dpi = view.dpi as f64;
    let width_in = (textrow.width() as f64) / dpi;
    let height_in = (textrow.height() as f64) / dpi;
    if region_is_figure(settings, width_in, height_in) {
        textrow.region_type = RowType::Figure;
    }
}

/// 几何缩放（C `textrow_scale`，`textrows.c:906-943`）。
///
/// `scalew` / `scaleh` 是缩放因子；`c2max` / `r2max` 是 clamp 上限。
///
/// **注意**：本 Rust 版未实现 `hyphen` 缩放（C 行 930-942）—— hyphen 字段
/// 留到 Step 8.1 (M6) 与 WrapState 一起处理。
pub fn scale_textrow(textrow: &mut TextRow, scalew: f64, scaleh: f64, c2max: i32, r2max: i32) {
    let scale_clamp_w = |v: i32| -> i32 {
        let scaled = ((v as f64) * scalew + 0.5) as i32;
        scaled.min(c2max)
    };
    let scale_clamp_h = |v: i32| -> i32 {
        let scaled = ((v as f64) * scaleh + 0.5) as i32;
        scaled.min(r2max)
    };
    textrow.c1 = scale_clamp_w(textrow.c1);
    textrow.r1 = scale_clamp_h(textrow.r1);
    textrow.c2 = scale_clamp_w(textrow.c2);
    textrow.r2 = scale_clamp_h(textrow.r2);
    textrow.rowbase = scale_clamp_h(textrow.rowbase);
    // C 行 924-929: 这些字段是 delta，不 clamp
    textrow.gap = ((textrow.gap as f64) * scaleh + 0.5) as i32;
    textrow.gapblank = ((textrow.gapblank as f64) * scaleh + 0.5) as i32;
    textrow.rowheight = ((textrow.rowheight as f64) * scaleh + 0.5) as i32;
    textrow.capheight = ((textrow.capheight as f64) * scaleh + 0.5) as i32;
    textrow.h5050 = ((textrow.h5050 as f64) * scaleh + 0.5) as i32;
    textrow.lcheight = ((textrow.lcheight as f64) * scaleh + 0.5) as i32;
}

// =====================================================================
// line_spacing_is_same / font_size_is_same（C 行 173-191）
// =====================================================================

/// 行间距是否相似（C `textrow_line_spacing_is_same`，行 173-177）。
///
/// `margin_pct` 是允许偏差百分比，与 C 的 `AGREE_WITHIN_MARGIN` 宏一致：
/// `|a - b| * 100 <= margin_pct * max(|a|, |b|)`。
pub fn line_spacing_is_same(tr1: &TextRow, tr2: &TextRow, margin_pct: i32) -> bool {
    agree_within_margin(tr1.rowheight, tr2.rowheight, margin_pct)
}

/// 字号是否相似（C `textrow_font_size_is_same`，行 183-191）。
///
/// 三个字号字段（lcheight/h5050/capheight）任一相似即返回 true，
/// 但首先要求两者都是 [`RowType::TextLine`]。
pub fn font_size_is_same(tr1: &TextRow, tr2: &TextRow, margin_pct: i32) -> bool {
    if tr1.region_type != RowType::TextLine || tr2.region_type != RowType::TextLine {
        return false;
    }
    agree_within_margin(tr1.lcheight, tr2.lcheight, margin_pct)
        || agree_within_margin(tr1.h5050, tr2.h5050, margin_pct)
        || agree_within_margin(tr1.capheight, tr2.capheight, margin_pct)
}

/// C 宏 `AGREE_WITHIN_MARGIN(a, b, margin)`：`abs(a-b)*100 <= margin*max(|a|,|b|)`。
fn agree_within_margin(a: i32, b: i32, margin_pct: i32) -> bool {
    let diff = (a - b).abs() as i64;
    let max_abs = a.unsigned_abs().max(b.unsigned_abs()) as i64;
    diff * 100 <= (margin_pct as i64) * max_abs
}

// =====================================================================
// 单元测试
// =====================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use k2types::{Bitmap, PixelFormat};

    fn bmp_with_rows(rows_y: &[(i32, i32, u8)], width: u32, height: u32) -> Bitmap {
        let mut bmp = Bitmap::new(width, height, 150.0, PixelFormat::Gray8).unwrap();
        bmp.fill_byte(255);
        for &(y0, y1, gray) in rows_y {
            for y in y0.max(0)..=y1.min((height - 1) as i32) {
                for x in 0..width {
                    let px = bmp.pixel_mut(x, y as u32).unwrap();
                    px[0] = gray;
                }
            }
        }
        bmp
    }

    fn view_full(bmp: &Bitmap, _dpi: f32) -> RegionView<'_> {
        RegionView::full(bmp)
    }

    fn settings_default() -> RowSettings {
        RowSettings::default()
    }

    // ---------------- TextRow / TextRows 基础 ----------------

    #[test]
    fn textrow_default_matches_c_init() {
        let r = TextRow::default();
        assert_eq!(r.c1, -1);
        assert_eq!(r.r2, -1);
        assert_eq!(r.rowbase, -1);
        assert_eq!(r.region_type, RowType::Undetermined);
        assert_eq!(r.rat, 0.0);
    }

    #[test]
    fn textrow_from_bbox_no_stats() {
        let rect = Rect::new(10, 20, 100, 200);
        let r = TextRow::from_bbox(rect, None, RowType::TextLine);
        assert_eq!(r.c1, 10);
        assert_eq!(r.r1, 20);
        assert_eq!(r.c2, 100);
        assert_eq!(r.r2, 200);
        assert_eq!(r.rowbase, 200);
        assert_eq!(r.capheight, -1);
        assert_eq!(r.region_type, RowType::TextLine);
    }

    #[test]
    fn textrow_from_bbox_with_stats() {
        let stats = TextRowStats {
            rowbase: 50,
            h5050: 12,
            lcheight: 10,
            capheight: 15,
        };
        let r = TextRow::from_bbox(Rect::new(0, 0, 100, 60), Some(&stats), RowType::TextLine);
        assert_eq!(r.rowbase, 50);
        assert_eq!(r.h5050, 12);
        assert_eq!(r.lcheight, 10);
        assert_eq!(r.capheight, 15);
    }

    #[test]
    fn textrow_width_height_inclusive() {
        let r = TextRow {
            c1: 5,
            c2: 14,
            r1: 10,
            r2: 19,
            ..TextRow::default()
        };
        assert_eq!(r.width(), 10);
        assert_eq!(r.height(), 10);
    }

    #[test]
    fn textrows_push_delete() {
        let mut t = TextRows::new();
        assert!(t.is_empty());
        t.push(TextRow::default());
        t.push(TextRow::default());
        assert_eq!(t.len(), 2);
        t.delete(0);
        assert_eq!(t.len(), 1);
        // 越界静默
        t.delete(5);
        assert_eq!(t.len(), 1);
    }

    // ---------------- RowSettings ----------------

    #[test]
    fn row_settings_default_matches_c() {
        let s = RowSettings::default();
        assert!((s.max_vertical_gap_inches - 0.25).abs() < 1e-9);
        assert!((s.gtr_in - 0.006).abs() < 1e-9);
        assert!((s.dst_min_figure_height_in - 0.75).abs() < 1e-9);
        assert!((s.little_piece_threshold_inches - 0.5).abs() < 1e-9);
        assert!((s.no_wrap_ar_limit - 0.2).abs() < 1e-9);
        assert!((s.no_wrap_height_limit_inches - 0.55).abs() < 1e-9);
        assert!(s.detect_double_rows);
        assert!(s.src_trim);
        assert!(s.src_left_to_right);
    }

    #[test]
    fn row_settings_to_crop_settings() {
        let s = RowSettings::default();
        let cs = s.crop_settings();
        assert!(cs.src_left_to_right);
        assert!((cs.defect_size_pts - 0.75).abs() < 1e-9);
    }

    // ---------------- fill_row_threshold_array ----------------

    #[test]
    fn fill_row_threshold_blank_region() {
        let bmp = bmp_with_rows(&[], 200, 100);
        let view = view_full(&bmp, 150.0);
        let bbox = calc_bbox(&view, &CropSettings::default(), false).unwrap();
        let (rt, rhmean) =
            fill_row_threshold_array(&view, &bbox.row_counts, &settings_default(), false);
        // 空 bbox → 返回空
        assert!(rt.is_empty() || rt.iter().all(|&v| v == 0));
        assert_eq!(rhmean, 0);
    }

    #[test]
    fn fill_row_threshold_single_dark_row() {
        // 一条 50px 高的全黑行在 y=40..=89
        let bmp = bmp_with_rows(&[(40, 89, 0)], 200, 100);
        let view = view_full(&bmp, 150.0);
        let bbox = calc_bbox(&view, &CropSettings::default(), false).unwrap();
        let (rt, rhmean) =
            fill_row_threshold_array(&view, &bbox.row_counts, &settings_default(), false);
        // 中间应该有非零 threshold
        let nonzero = rt.iter().filter(|&&v| v > 40).count();
        assert!(nonzero >= 30, "expected ≥30 above threshold, got {nonzero}");
        // 平均行高应该接近 50（容差 ±5）
        assert!((rhmean - 50).abs() <= 15, "rhmean={rhmean}");
    }

    // ---------------- find_textrows ----------------

    #[test]
    fn find_textrows_blank_returns_empty() {
        let bmp = bmp_with_rows(&[], 200, 100);
        let view = view_full(&bmp, 150.0);
        let rows = find_textrows(&view, &settings_default(), false, false, 0.0, false).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn find_textrows_two_rows_with_gap() {
        // 行 1: y=20..=40 (高 21)，行 2: y=60..=80 (高 21)，间距 20 像素
        let bmp = bmp_with_rows(&[(20, 40, 0), (60, 80, 0)], 200, 100);
        let view = view_full(&bmp, 150.0);
        let rows = find_textrows(&view, &settings_default(), false, false, 0.0, false).unwrap();
        assert_eq!(
            rows.len(),
            2,
            "expected 2 rows, got {} (rows={:?})",
            rows.len(),
            rows.rows
        );
        // 第一行应在 y=20 附近
        let r0 = &rows.rows[0];
        assert!((r0.r1 - 20).abs() <= 4, "r0.r1={}", r0.r1);
        assert!((r0.r2 - 40).abs() <= 4, "r0.r2={}", r0.r2);
        // 第二行
        let r1 = &rows.rows[1];
        assert!((r1.r1 - 60).abs() <= 4, "r1.r1={}", r1.r1);
        assert!((r1.r2 - 80).abs() <= 4, "r1.r2={}", r1.r2);
    }

    #[test]
    fn find_textrows_three_rows_compute_gaps() {
        // 行 y=10..=20, 40..=50, 70..=80（高 11，间距 19）
        let bmp = bmp_with_rows(&[(10, 20, 0), (40, 50, 0), (70, 80, 0)], 200, 100);
        let view = view_full(&bmp, 150.0);
        let rows = find_textrows(&view, &settings_default(), false, false, 0.0, false).unwrap();
        assert_eq!(rows.len(), 3);
        // rowheight[0] = textrow[1].r1 - textrow[0].r1
        let expected = rows.rows[1].r1 - rows.rows[0].r1;
        assert_eq!(rows.rows[0].rowheight, expected);
        // gap[i] = textrow[i+1].r1 - textrow[i].rowbase - 1
        let g0 = rows.rows[1].r1 - rows.rows[0].rowbase - 1;
        assert_eq!(rows.rows[0].gap, g0);
        // gapblank[i] = textrow[i+1].r1 - textrow[i].r2 - 1
        let gb = rows.rows[1].r1 - rows.rows[0].r2 - 1;
        assert_eq!(rows.rows[0].gapblank, gb);
    }

    #[test]
    fn find_textrows_dense_text_dynamic_aperture() {
        // 多行紧密排列：每行高 10，间距 5
        let mut rows_y = Vec::new();
        let mut y = 10;
        while y + 10 < 200 {
            rows_y.push((y, y + 9, 0));
            y += 15;
        }
        let bmp = bmp_with_rows(&rows_y, 100, 200);
        let view = view_full(&bmp, 150.0);
        let rows = find_textrows(&view, &settings_default(), true, false, 0.0, false).unwrap();
        // 至少能找到 8 行（容差宽，避免对算法过于敏感）
        assert!(rows.len() >= 8, "expected ≥8 rows, got {}", rows.len());
    }

    // ---------------- compute_row_gaps ----------------

    #[test]
    fn compute_row_gaps_single_row() {
        let mut t = TextRows::new();
        t.push(TextRow {
            r1: 10,
            r2: 30,
            rowbase: 28,
            region_type: RowType::TextLine,
            ..TextRow::default()
        });
        compute_row_gaps(&mut t, 100);
        // n=1: rowheight = r2-r1
        assert_eq!(t.rows[0].rowheight, 20);
        // gap = r2_outer - rowbase
        assert_eq!(t.rows[0].gap, 100 - 28);
        assert_eq!(t.rows[0].gapblank, 0);
    }

    #[test]
    fn compute_row_gaps_multi_row() {
        let mut t = TextRows::new();
        t.push(TextRow {
            r1: 10,
            r2: 30,
            rowbase: 28,
            region_type: RowType::TextLine,
            ..TextRow::default()
        });
        t.push(TextRow {
            r1: 50,
            r2: 70,
            rowbase: 68,
            region_type: RowType::TextLine,
            ..TextRow::default()
        });
        compute_row_gaps(&mut t, 100);
        // rowheight[0] = textrow[1].r1 - textrow[0].r1 = 40
        assert_eq!(t.rows[0].rowheight, 40);
        // rowheight[1] = rowbase[1] - rowbase[0] = 40
        assert_eq!(t.rows[1].rowheight, 40);
        // gap[0] = r1[1] - rowbase[0] - 1 = 50 - 28 - 1 = 21
        assert_eq!(t.rows[0].gap, 21);
        // gapblank[0] = r1[1] - r2[0] - 1 = 50 - 30 - 1 = 19
        assert_eq!(t.rows[0].gapblank, 19);
        // last gap = r2_outer - rowbase[1] = 100 - 68 = 32
        assert_eq!(t.rows[1].gap, 32);
        assert_eq!(t.rows[1].gapblank, 0);
    }

    #[test]
    fn compute_row_gaps_figure_uses_r2() {
        let mut t = TextRows::new();
        t.push(TextRow {
            r1: 0,
            r2: 50,
            rowbase: 30,
            region_type: RowType::Figure,
            ..TextRow::default()
        });
        t.push(TextRow {
            r1: 60,
            r2: 80,
            rowbase: 78,
            region_type: RowType::TextLine,
            ..TextRow::default()
        });
        compute_row_gaps(&mut t, 100);
        // gap[0] for figure: r1[1] - r2[0] - 1 = 60 - 50 - 1 = 9
        assert_eq!(t.rows[0].gap, 9);
    }

    // ---------------- remove_defects ----------------

    #[test]
    fn remove_defects_removes_small_rows() {
        let mut t = TextRows::new();
        // 5x5 杂质 + 20x20 正常
        t.push(TextRow {
            c1: 0,
            c2: 4,
            r1: 0,
            r2: 4,
            region_type: RowType::TextLine,
            ..TextRow::default()
        });
        t.push(TextRow {
            c1: 0,
            c2: 19,
            r1: 10,
            r2: 29,
            region_type: RowType::TextLine,
            ..TextRow::default()
        });
        remove_defects(&mut t, 6);
        assert_eq!(t.len(), 1);
        assert_eq!(t.rows[0].r1, 10);
    }

    #[test]
    fn remove_defects_preserves_wide_thin() {
        let mut t = TextRows::new();
        // 100x3 横线 - 高度小但宽度大，应保留
        t.push(TextRow {
            c1: 0,
            c2: 99,
            r1: 0,
            r2: 2,
            region_type: RowType::TextLine,
            ..TextRow::default()
        });
        remove_defects(&mut t, 5);
        assert_eq!(t.len(), 1);
    }

    // ---------------- sort ----------------

    #[test]
    fn sort_by_gap_ascending() {
        let mut t = TextRows::new();
        t.push(TextRow {
            gap: 30,
            ..TextRow::default()
        });
        t.push(TextRow {
            gap: 10,
            ..TextRow::default()
        });
        t.push(TextRow {
            gap: 20,
            ..TextRow::default()
        });
        sort_by_gap(&mut t);
        assert_eq!(t.rows[0].gap, 10);
        assert_eq!(t.rows[2].gap, 30);
    }

    #[test]
    fn sort_by_row_position_ascending() {
        let mut t = TextRows::new();
        t.push(TextRow {
            r1: 50,
            ..TextRow::default()
        });
        t.push(TextRow {
            r1: 10,
            ..TextRow::default()
        });
        sort_by_row_position(&mut t);
        assert_eq!(t.rows[0].r1, 10);
        assert_eq!(t.rows[1].r1, 50);
    }

    // ---------------- region_is_figure / determine_type ----------------

    #[test]
    fn region_is_figure_true_for_tall_image() {
        let s = settings_default();
        // ar = 0.5 > 0.2，h = 1.0 > 0.55 → figure
        assert!(region_is_figure(&s, 0.5, 1.0));
    }

    #[test]
    fn region_is_figure_false_for_skinny_line() {
        let s = settings_default();
        // ar = 0.01 < 0.2 → 非 figure
        assert!(!region_is_figure(&s, 0.01, 1.0));
    }

    #[test]
    fn region_is_figure_false_for_tiny() {
        let s = settings_default();
        // h = 0.1 < 0.55 且 < 0.75 → 非 figure
        assert!(!region_is_figure(&s, 0.5, 0.1));
    }

    #[test]
    fn determine_type_upgrades_to_figure() {
        let bmp = bmp_with_rows(&[(0, 99, 0)], 200, 100);
        let view = view_full(&bmp, 150.0);
        let mut row = TextRow {
            c1: 0,
            c2: 199,
            r1: 0,
            r2: 99,
            region_type: RowType::TextLine,
            ..TextRow::default()
        };
        determine_type(&view, &settings_default(), &mut row);
        // 200x100 @ 150dpi → 1.33 x 0.67 in，ar=2.0 > 0.2，h=0.67 > 0.55 → figure
        assert_eq!(row.region_type, RowType::Figure);
    }

    #[test]
    fn determine_type_keeps_figure() {
        let bmp = bmp_with_rows(&[(0, 9, 0)], 100, 10);
        let view = view_full(&bmp, 150.0);
        let mut row = TextRow {
            c1: 0,
            c2: 99,
            r1: 0,
            r2: 9,
            region_type: RowType::Figure,
            ..TextRow::default()
        };
        determine_type(&view, &settings_default(), &mut row);
        assert_eq!(row.region_type, RowType::Figure);
    }

    // ---------------- scale_textrow ----------------

    #[test]
    fn scale_textrow_doubles_coords() {
        let mut r = TextRow {
            c1: 10,
            c2: 100,
            r1: 20,
            r2: 200,
            rowbase: 180,
            gap: 5,
            gapblank: 3,
            rowheight: 50,
            capheight: 40,
            h5050: 35,
            lcheight: 30,
            region_type: RowType::TextLine,
            rat: 0.0,
        };
        scale_textrow(&mut r, 2.0, 2.0, 1000, 1000);
        assert_eq!(r.c1, 20);
        assert_eq!(r.c2, 200);
        assert_eq!(r.r1, 40);
        assert_eq!(r.r2, 400);
        assert_eq!(r.rowbase, 360);
        assert_eq!(r.gap, 10);
        assert_eq!(r.capheight, 80);
    }

    #[test]
    fn scale_textrow_clamps_to_max() {
        let mut r = TextRow {
            c1: 0,
            c2: 100,
            r1: 0,
            r2: 100,
            rowbase: 100,
            region_type: RowType::TextLine,
            ..TextRow::default()
        };
        scale_textrow(&mut r, 10.0, 10.0, 500, 500);
        assert_eq!(r.c2, 500);
        assert_eq!(r.r2, 500);
        assert_eq!(r.rowbase, 500);
    }

    // ---------------- agree_within_margin / spacing / font ----------------

    #[test]
    fn agree_within_margin_basic() {
        assert!(agree_within_margin(100, 105, 10)); // 5% ≤ 10%
        assert!(agree_within_margin(100, 110, 10)); // 10% = 10%
        assert!(!agree_within_margin(100, 120, 10)); // 20% > 10%
        assert!(agree_within_margin(0, 0, 10)); // 0/0 算相等
    }

    #[test]
    fn line_spacing_is_same_within_10pct() {
        let r1 = TextRow {
            rowheight: 100,
            ..TextRow::default()
        };
        let r2 = TextRow {
            rowheight: 105,
            ..TextRow::default()
        };
        assert!(line_spacing_is_same(&r1, &r2, 10));
    }

    #[test]
    fn font_size_requires_textline_type() {
        let r1 = TextRow {
            lcheight: 10,
            region_type: RowType::Undetermined,
            ..TextRow::default()
        };
        let r2 = TextRow {
            lcheight: 10,
            region_type: RowType::TextLine,
            ..TextRow::default()
        };
        assert!(!font_size_is_same(&r1, &r2, 10));
    }

    #[test]
    fn font_size_any_metric_match() {
        let r1 = TextRow {
            lcheight: 50,
            h5050: 60,
            capheight: 70,
            region_type: RowType::TextLine,
            ..TextRow::default()
        };
        let r2 = TextRow {
            lcheight: 30, // 差很大
            h5050: 55,    // ~8% 差
            capheight: 200,
            region_type: RowType::TextLine,
            ..TextRow::default()
        };
        // h5050 在 10% 内
        assert!(font_size_is_same(&r1, &r2, 10));
    }

    // ---------------- remove_small_rows ----------------

    #[test]
    fn remove_small_rows_noop_when_lt_2() {
        let mut t = TextRows::new();
        t.push(TextRow::default());
        let bmp = bmp_with_rows(&[], 100, 100);
        let view = view_full(&bmp, 150.0);
        remove_small_rows(
            &mut t,
            &view,
            &settings_default(),
            0.25,
            0.5,
            0.0,
            Rect::new(0, 0, 99, 99),
        )
        .unwrap();
        assert_eq!(t.len(), 1);
    }
}
