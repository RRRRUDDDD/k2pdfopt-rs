//! `k2layout::words` — 行内词分割（textwords.c + bmpregion_one_row_find_textwords → Rust）。
//!
//! **Step 6.3 (M4)**：1:1 移植 `k2pdfoptlib/textwords.c`（124 行）三个 helper +
//! `bmpregion.c::bmpregion_one_row_find_textwords`（行 1752-1954）+
//! `bmpregion_count_text_row_pixels`（行 1974-2080）+ `bmpregion_find_gaps`
//! （行 2083-2136）+ `get_word_gap_threshold`（行 2155-2379）。
//!
//! # 范围
//!
//! 已实现（Step 6.3）：
//! - [`TextWords`] 类型别名（C `TEXTWORDS` 实质 = `TEXTROWS`）+ [`WordSettings`] 独立 struct
//! - [`WordGapDatabase`]：替代 C 版 `add_word_gaps` 内部 `static gap[1024]`（线程安全 / 显式生命周期）
//! - [`compute_col_gaps`]：词间横向 gap / gapblank / rowheight 更新
//! - [`remove_small_col_gaps`]：合并过窄的 word 缝（gap < mingap）
//! - [`add_word_gaps`]：把当前行的 word gap 写入 database（自动扩展）
//! - [`compute_median_gap`]：从 database 取中位数 gap（默认 0.7）
//! - [`one_row_find_textwords`]：主入口；按 lcheight 计算"空格大小"窗口扫描 bp 数组，
//!   找 gap → bi-modal 阈值判定 → 按 multiplier 切 word
//!
//! 推迟（Open Question 6.3.A-D）：
//! - hyphen 检测、行尾连字符识别 → Step 8.2 (M6 WrapState)
//! - dropcap detection 与 partial row split → 与 `find_doubles` 一同推迟到 Step 6.x
//! - word_longer_than 在 get_word_gap_threshold 内部循环再次收紧 gt → C 版用 `#ifdef COMMENT`
//!   注释掉的代码段，Rust 版不实现
//!
//! # C 行号对照
//!
//! 注释中所有 `[textwords.c:NNN]` / `[bmpregion.c:NNN]` 引用均按 v2.55 源码。

#![allow(clippy::too_many_arguments)]

use crate::crop::{calc_bbox, CropSettings, TRIM_C1, TRIM_C2, TRIM_CALC_TEXT};
use crate::region::RegionView;
use crate::rows::{RowType, TextRow, TextRows};
use k2core::rect::Rect;

// =====================================================================
// 数据结构
// =====================================================================

/// 词集合。C 版 `TEXTWORDS` 实际是 `TEXTROWS` 的 typedef（`k2pdfopt.h:543`），
/// Rust 端同样复用 [`TextRows`] 表达"一行内的多个词"。
pub type TextWords = TextRows;

/// 词检测设置（独立 struct，避免 k2layout → k2settings 反向依赖）。
///
/// 字段一一对应 C 版 `K2PDFOPT_SETTINGS`，默认值来自 `k2settings_init`
/// （`k2settings.c:38-300`）。
#[derive(Clone, Debug)]
pub struct WordSettings {
    /// 词间距阈值（C `word_spacing`，默认 -0.20）。
    ///
    /// 负值 = 自动模式（按 bi-modal 分布找最优阈值），绝对值作为最小 gap 限制。
    /// 正值 = 强制阈值（按 lcheight 比例）。`k2settings.c:128`。
    pub word_spacing: f64,
    /// 词间黑像素阈值（inches，C `gtw_in`，默认 0.0015，`k2settings.c:50`）。
    pub gtw_in: f64,
    /// 单页最大可视宽度（inches，C `max_region_width_inches`，默认 3.6，
    /// `k2settings.c:122`）。
    pub max_region_width_inches: f64,
    /// 源 DPI（C `src_dpi`，默认 300）。
    pub src_dpi: i32,
    /// 容忍的孤立瑕疵半径（pts，C `defect_size_pts`，默认 1.5）。
    pub defect_size_pts: f64,
    /// 左→右阅读（C `src_left_to_right`，默认 1）。
    pub src_left_to_right: bool,
}

impl Default for WordSettings {
    fn default() -> Self {
        Self {
            word_spacing: -0.20,
            gtw_in: 0.0015,
            max_region_width_inches: 3.6,
            src_dpi: 300,
            defect_size_pts: 1.5,
            src_left_to_right: true,
        }
    }
}

impl WordSettings {
    /// 提取 [`CropSettings`] 视图（用于复用 [`calc_bbox`]）。
    pub fn crop_settings(&self) -> CropSettings {
        CropSettings {
            src_left_to_right: self.src_left_to_right,
            defect_size_pts: self.defect_size_pts,
        }
    }
}

/// 跨页 / 跨行的 word gap 历史库。
///
/// **C 对照**：`textwords.c:83-84` 的 `static int nn; static double gap[1024];`。
/// C 版用 process-local static，Rust 版改为显式拥有的结构体，让调用方决定生命周期
/// （ADR-008 错误模型 + 显式 state 偏好）。
///
/// 容量固定 1024（与 C 版 `gap[1024]` 一致），溢出时按环形 `next_index & 0x3ff`
/// 覆盖旧条目，`compute_median_gap` 始终用最新最多 1024 条数据。
#[derive(Clone, Debug)]
pub struct WordGapDatabase {
    /// 环形缓冲（C `gap[1024]`）。
    gaps: [f64; 1024],
    /// 累计追加条数（不取模；缓冲填充用 `next_index & 0x3ff`）。
    /// C 行 102: `gap[nn&0x3ff]= g; nn++;`
    next_index: usize,
}

impl Default for WordGapDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl WordGapDatabase {
    /// 空 database。
    pub fn new() -> Self {
        Self {
            gaps: [0.0; 1024],
            next_index: 0,
        }
    }

    /// 重置（对应 C `textwords_add_word_gaps(NULL,0,NULL,0)`，`k2settings.c:559` 调用）。
    pub fn reset(&mut self) {
        self.next_index = 0;
        self.gaps = [0.0; 1024];
    }

    /// 当前 database 内有效条目数（min(next_index, 1024)）。
    pub fn len(&self) -> usize {
        self.next_index.min(1024)
    }

    /// 是否空。
    pub fn is_empty(&self) -> bool {
        self.next_index == 0
    }
}

// =====================================================================
// 公开 API：compute_col_gaps（C `textwords_compute_col_gaps`，textwords.c:26-43）
// =====================================================================

/// 计算词间 gap / gapblank / rowheight（横向版本）。
///
/// **C 对照**：`textwords.c:26-43`。
///
/// 与 [`crate::rows::compute_row_gaps`] 对称：rows 沿垂直方向算 r2→r1 gap，
/// words 沿水平方向算 c2→c1 gap。
///
/// - `words[i].gap` = `words[i+1].c1 - words[i].c2 - 1`（与最后一个的 gap 用 `c2_row` 算）
/// - `words[i].gapblank` = `words[i].gap`（横向无 baseline 区分，gap == gapblank）
/// - `words[i].rowheight` = `words[i+1].c1 - words[i].c1`（与下一个 word c1 间距；最后一个用 `c2 - c1`）
///
/// `c2_row` 是父 row 的 c2（C 行 40-42：最后一个 word 用 `c2 - last.c2` 作 gap）。
pub fn compute_col_gaps(words: &mut TextWords, c2_row: i32) {
    let n = words.rows.len();
    if n == 0 {
        return;
    }
    // C 行 34-39
    for i in 0..n - 1 {
        let next_c1 = words.rows[i + 1].c1;
        let cur_c1 = words.rows[i].c1;
        let cur_c2 = words.rows[i].c2;
        words.rows[i].gap = next_c1 - cur_c2 - 1;
        words.rows[i].gapblank = words.rows[i].gap;
        words.rows[i].rowheight = next_c1 - cur_c1;
    }
    // C 行 40-42: 最后一个 word
    let last = n - 1;
    let last_c2 = words.rows[last].c2;
    let last_c1 = words.rows[last].c1;
    words.rows[last].gap = c2_row - last_c2;
    words.rows[last].gapblank = words.rows[last].gap;
    words.rows[last].rowheight = last_c2 - last_c1;
}

// =====================================================================
// 公开 API：remove_small_col_gaps（C `textwords_remove_small_col_gaps`，textwords.c:46-72）
// =====================================================================

/// 合并过窄的 word 缝。
///
/// **C 对照**：`textwords.c:46-72`。
///
/// 算法：mingap = max(mingap, word_spacing)；遍历 words，若 `gap[i] / lcheight < mingap`，
/// 把 word[i+1] 合并到 word[i]（吸收 c2/gap/r1/r2 极值），后续整体左移。
pub fn remove_small_col_gaps(
    words: &mut TextWords,
    lcheight: i32,
    mut mingap: f64,
    word_spacing: f64,
) {
    if lcheight <= 0 {
        return;
    }
    // C 行 52-53
    if mingap < word_spacing {
        mingap = word_spacing;
    }
    // C 行 54-71: 主循环
    let mut i = 0;
    while i + 1 < words.rows.len() {
        let cur_gap = words.rows[i].gap as f64 / lcheight as f64;
        if cur_gap >= mingap {
            i += 1;
            continue;
        }
        // 合并 i 与 i+1
        // C 行 61: cur.c2 = next.c2
        words.rows[i].c2 = words.rows[i + 1].c2;
        // C 行 62: cur.gap = next.gap
        words.rows[i].gap = words.rows[i + 1].gap;
        words.rows[i].gapblank = words.rows[i + 1].gapblank;
        // C 行 63-64: r1 取 min
        if words.rows[i + 1].r1 < words.rows[i].r1 {
            words.rows[i].r1 = words.rows[i + 1].r1;
        }
        // C 行 65-66: r2 取 max
        if words.rows[i + 1].r2 > words.rows[i].r2 {
            words.rows[i].r2 = words.rows[i + 1].r2;
        }
        // C 行 67-69: 删除 i+1 (Vec::remove 自动后移)
        words.rows.remove(i + 1);
        // C 行 70: i-- 然后 for ++ → 这次循环仍处理同一 i（再检查新合并行与下一行）
        // Rust 端不 ++（外层 while 已不增），保持 i 不变
    }
}

// =====================================================================
// 公开 API：add_word_gaps / compute_median_gap
// =====================================================================

/// 把当前行的所有 word gap 写入 database（用于跨行 / 跨页统计）。
///
/// **C 对照**：`textwords_add_word_gaps`（textwords.c:79-105，textwords!=NULL 分支）。
///
/// 仅记录 `gap/lcheight >= word_spacing` 的"够长"gap（避免字符内噪声污染）。
/// 当 database 满 1024 条时按 `nn & 0x3ff` 环形覆盖。
pub fn add_word_gaps(
    words: &TextWords,
    lcheight: i32,
    dbase: &mut WordGapDatabase,
    word_spacing: f64,
) {
    if lcheight <= 0 {
        return;
    }
    // C 行 92: textwords->n > 1
    if words.rows.len() < 2 {
        return;
    }
    // C 行 96-104: 遍历前 n-1 个（最后一个的 gap 不算）
    for i in 0..words.rows.len() - 1 {
        let g = words.rows[i].gap as f64 / lcheight as f64;
        // C 行 100: g>=word_spacing
        if g >= word_spacing {
            // C 行 102-103: gap[nn&0x3ff]=g; nn++
            dbase.gaps[dbase.next_index & 0x3ff] = g;
            dbase.next_index += 1;
        }
    }
}

/// 从 database 计算中位数 gap。
///
/// **C 对照**：`textwords_add_word_gaps`（textwords.c:107-122，median_gap!=NULL 分支）。
///
/// 空 database 返回 0.7（C 行 121-122 fallback 默认值）。
/// 按 C `sortd` 行为：升序排序后取 `gap_sorted[n/2]`（**注意**：C 用整数除法，
/// `n=4` 时取 `gap_sorted[2]` 即第 3 个元素，"上中位数"）。
pub fn compute_median_gap(dbase: &WordGapDatabase) -> f64 {
    let n = dbase.len();
    if n == 0 {
        // C 行 121-122
        return 0.7;
    }
    let mut sorted: Vec<f64> = dbase.gaps[..n].to_vec();
    // C 行 117: sortd ascending
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // C 行 118: gap_sorted[n/2]（整数除法 → 上中位数）
    sorted[n / 2]
}

// =====================================================================
// 公开 API：one_row_find_textwords（主入口）
// =====================================================================

/// 把一行 `row` 切成若干 word。
///
/// **C 对照**：`bmpregion_one_row_find_textwords`（`bmpregion.c:1752-1954`）。
///
/// # 输入
/// - `view`：覆盖整张位图的视图（DPI/bgcolor 在 view.dpi/view.bgcolor）
/// - `row_rect`：当前行的几何 rect（inclusive 端点）。通常来自 [`crate::rows::find_textrows`] 的输出
/// - `row_lcheight`：当前行的小写高度 lcheight（C `region->bbox.lcheight`）。**必须 > 0**，
///   否则函数直接返回空集（C 行 1816-1821 宽度<6 早退路径）
/// - `settings`：见 [`WordSettings`]
/// - `dbase`：可变 [`WordGapDatabase`]；`add_to_dbase=true` 时新 gap 会追加
/// - `add_to_dbase`：是否把本行 gap 累加进 database（与 C 第 3 参一致）
///
/// # 输出
///
/// 返回 [`TextWords`]，每个 [`TextRow`] 是一个 word：
/// - `c1/c2/r1/r2` 来自 [`calc_bbox`]（精确 word bbox）
/// - `region_type = RowType::Word`
/// - `rowbase/h5050/lcheight/capheight` 来自 word 内部文本统计
/// - `gap/gapblank/rowheight` 由后续 [`compute_col_gaps`] 填充
pub fn one_row_find_textwords(
    view: &RegionView,
    row_rect: Rect,
    row_lcheight: i32,
    settings: &WordSettings,
    dbase: &mut WordGapDatabase,
    add_to_dbase: bool,
) -> TextWords {
    let mut words = TextWords::new();
    // C 行 1810-1812: 默认就是 row 整个 bbox（如果切失败 / 宽度过小，回退此结果）
    let default_word = TextRow {
        c1: row_rect.x0,
        c2: row_rect.x1,
        r1: row_rect.y0,
        r2: row_rect.y1,
        rowbase: row_rect.y1,
        gap: -1,
        gapblank: -1,
        rowheight: -1,
        capheight: -1,
        h5050: -1,
        lcheight: row_lcheight,
        region_type: RowType::Word,
        rat: 0.0,
    };

    // C 行 1814-1815: 对 newregion 做 trim 0x13（C1|C2|CALC_TEXT）
    // 即左右收紧、不裁 r1/r2、同时算 textstats
    let row_view = RegionView::with(view.bmp, row_rect, view.dpi, view.bgcolor);
    let crop_settings = settings.crop_settings();
    let trim_flags = TRIM_C1 | TRIM_C2 | TRIM_CALC_TEXT;
    let bbox_result = crate::crop::trim_margins_with_bbox(&row_view, &crop_settings, trim_flags);
    let (trimmed_rect, bbox) = match bbox_result {
        Ok(t) => t,
        Err(_) => {
            // 视图越界（罕见）→ 直接返回 default_word
            words.push(default_word);
            return words;
        }
    };
    // 用新算出的 lcheight（如果统计成功）替代外部传入值
    let effective_lcheight = bbox
        .text_stats
        .map(|s| s.lcheight)
        .filter(|&v| v > 0)
        .unwrap_or(row_lcheight);

    // C 行 1816-1821: 宽度<6 → 早退
    let width = trimmed_rect.x1 - trimmed_rect.x0 + 1;
    if width < 6 {
        words.push(default_word);
        return words;
    }

    // C 行 1826-1828: dr = lcheight，至少 1
    let dr = effective_lcheight.max(1);

    // C 行 1845-1849: 算 bp 数组 + find_gaps
    let bp = count_text_row_pixels(trimmed_rect, &bbox.col_counts, dr, view.dpi, settings);
    let (mut gw, mut copt) = find_gaps(&bp, dr, trimmed_rect.x0, trimmed_rect.x1);
    let mut ngaps = gw.len();

    // C 行 1852-1854: sortxyi(gw, copt, ngaps) + flipi → 按 gw 降序排序
    sort_pair_desc(&mut gw, &mut copt);

    // C 行 1855-1856: gap_thresh = get_word_gap_threshold(...)
    let mut gap_thresh = get_word_gap_threshold(&gw, dr, width, settings);

    // C 行 1857-1860: 若 word_spacing<0 且 gap_thresh < mgt，提升到 mgt
    let mgt = (settings.word_spacing.abs() * dr as f64 + 0.5) as i32;
    if settings.word_spacing < 0.0 && gap_thresh < mgt {
        gap_thresh = mgt;
    }

    // C 行 1864-1866: 截断 ngaps 到第一个 gw[i] < gap_thresh 的位置（独占）
    let mut new_ngaps = ngaps;
    for (i, &val) in gw.iter().enumerate().take(ngaps) {
        if val < gap_thresh {
            new_ngaps = i;
            break;
        }
    }
    ngaps = new_ngaps;
    gw.truncate(ngaps);
    copt.truncate(ngaps);

    // C 行 1875: sortxyi(copt, gw, ngaps) → 按 copt（位置）升序
    sort_pair_by_first_asc(&mut copt, &mut gw);

    // C 行 1881: display_width = max_region_width_inches * src_dpi
    let display_width = (settings.max_region_width_inches * settings.src_dpi as f64).round() as i32;

    // C 行 1882-1926: 主切分循环
    let mut i0: i32 = -1;
    let mut i: i32 = 0;
    let mut multiplier: f64 = 1.0;
    while i <= ngaps as i32 {
        // C 行 1887: i<ngaps && gw[i]<gap_thresh*multiplier → continue
        if (i as usize) < ngaps {
            let cur_gw = gw[i as usize] as f64;
            if cur_gw < gap_thresh as f64 * multiplier {
                i += 1;
                continue;
            }
        }
        // C 行 1889-1890: c1 / c2 边界
        let c1 = if i0 < 0 {
            trimmed_rect.x0
        } else {
            copt[i0 as usize] + 1
        };
        let c2 = if (i as usize) == ngaps {
            trimmed_rect.x1
        } else {
            copt[i as usize]
        };
        // C 行 1891-1892: 宽度<2 跳过
        if c2 - c1 < 2 {
            i += 1;
            continue;
        }
        // C 行 1893-1909: word 太长检测（仅自动模式）
        if settings.word_spacing < 0.0 && (c2 - c1 + 1) > display_width && i - i0 > 1 {
            multiplier *= 0.9;
            if multiplier > 0.05 && (gap_thresh as f64 * multiplier) >= mgt as f64 {
                // 回退到 i0 重扫
                i = i0;
                // 但 i 当前-1 的话下一轮 +1 后就回到 0+1，C 版逻辑是 i=i0 → 然后 for 增量 i++
                // 在 Rust while 里我们处理为：i=i0 + 1 后继续；注意 i0 可能 -1 → i=0
                i += 1;
                continue;
            }
        }
        // C 行 1914-1921: 切出 word region，算 bbox
        let word_rect = Rect::new(c1, trimmed_rect.y0, c2, trimmed_rect.y1);
        let word_view = RegionView::with(view.bmp, word_rect, view.dpi, view.bgcolor);
        if let Ok(word_bbox) = calc_bbox(&word_view, &crop_settings, true) {
            let word_row = TextRow {
                c1: word_bbox.rect.x0,
                c2: word_bbox.rect.x1,
                r1: word_bbox.rect.y0,
                r2: word_bbox.rect.y1,
                rowbase: word_bbox
                    .text_stats
                    .map(|s| s.rowbase)
                    .unwrap_or(word_bbox.rect.y1),
                gap: -1,
                gapblank: -1,
                rowheight: -1,
                capheight: word_bbox.text_stats.map(|s| s.capheight).unwrap_or(-1),
                h5050: word_bbox.text_stats.map(|s| s.h5050).unwrap_or(-1),
                lcheight: word_bbox.text_stats.map(|s| s.lcheight).unwrap_or(-1),
                region_type: RowType::Word,
                rat: 0.0,
            };
            words.push(word_row);
        }
        // C 行 1923-1925: 重置
        i0 = i;
        multiplier = 1.0;
        i += 1;
    }

    // C 行 1930: compute_col_gaps
    compute_col_gaps(&mut words, trimmed_rect.x1);

    // C 行 1934-1941: word_spacing>=0 时去小 gap
    if settings.word_spacing >= 0.0 {
        if add_to_dbase {
            add_word_gaps(
                &words,
                effective_lcheight,
                dbase,
                gap_thresh as f64 / dr as f64,
            );
        }
        let median = compute_median_gap(dbase);
        remove_small_col_gaps(
            &mut words,
            effective_lcheight,
            median / 1.9,
            gap_thresh as f64 / dr as f64,
        );
    } else if add_to_dbase {
        // 自动模式 C 版也调用了 add_word_gaps（注意：但只在 word_spacing>=0 分支内）
        // 实际 C 行 1935 是 `if (word_spacing>=0.)` 包住 add+remove。Rust 一致
    }

    // C 行 1944-1949: 若没切出 word 则退回 default
    if words.rows.is_empty() {
        words.push(default_word);
    }

    words
}

// =====================================================================
// 内部 helper: count_text_row_pixels（C bmpregion.c:1974-2080）
// =====================================================================

/// 计算 bp 数组（沿 row 横向的"暗度归一化"）。
///
/// **C 对照**：`bmpregion_count_text_row_pixels`（`bmpregion.c:1974-2080`）。
///
/// 输出 bp 长度 = `row.x1 - row.x0 + 1`，索引 0 对应 `col_counts[row.x0]`。
///
/// 公式：
/// - mingap = max(2, dr*0.02)
/// - 对每列 i ∈ [x0, x1]，取 \[i - mingap/2, i + mingap - 1] 窗口
/// - pt = (window_size * gtw_in * dpi + 0.5) clamp 到 ≥1
/// - bp[i-x0] = 10 * sum(col_counts[ii]) / pt（10 倍归一化）
fn count_text_row_pixels(
    rect: Rect,
    col_counts: &[i32],
    dr: i32,
    dpi: f32,
    settings: &WordSettings,
) -> Vec<i32> {
    let nc = (rect.x1 - rect.x0 + 1) as usize;
    let mut bp = vec![0i32; nc];

    // C 行 2002-2004: mingap = max(2, dr*0.02)
    let mut mingap = (dr as f64 * 0.02) as i32;
    if mingap < 2 {
        mingap = 2;
    }

    // C 行 2017-2035 / 2046-2061: LTR 与 RTL 算同样的 bp（顺序差异不影响结果）
    for i in rect.x0..=rect.x1 {
        // C 行 2021-2026: 窗口端点
        let mut i1 = i - mingap / 2;
        let mut i2 = i1 + mingap - 1;
        if i1 < rect.x0 {
            i1 = rect.x0;
        }
        if i2 > rect.x1 {
            i2 = rect.x1;
        }
        // C 行 2027-2029: pt
        let window_size = (i2 - i1 + 1) as f64;
        let mut pt = (window_size * settings.gtw_in * dpi as f64 + 0.5) as i32;
        if pt < 1 {
            pt = 1;
        }
        // C 行 2030: sum over col_counts[ii]
        let mut sum: i64 = 0;
        for ii in i1..=i2 {
            // col_counts 用绝对索引；越界（理论不应发生）按 0 处理
            if let Some(&v) = col_counts.get(ii as usize) {
                sum += v as i64;
            }
        }
        // C 行 2031: bp[i-c1] = 10*sum/pt
        bp[(i - rect.x0) as usize] = (10 * sum / pt as i64) as i32;
    }
    bp
}

// =====================================================================
// 内部 helper: find_gaps（C bmpregion.c:2083-2136）
// =====================================================================

/// 在 bp 数组中找 gap（双阈值状态机）。
///
/// **C 对照**：`bmpregion_find_gaps`（`bmpregion.c:2083-2136`）。
///
/// 阈值 thlow=10 / thhigh=20（C 行 2088-2089）。算法：
/// 1. 跳到 bp >= thhigh 的位置（进入"文字带"）
/// 2. 推进直到 bp < thlow（离开文字带）
/// 3. 在后续 2*dr 范围内找最低 bp 作为 copt（gap 中心）
/// 4. 一旦碰到 bp > thhigh 提前停（新文字带开始）
/// 5. 记录 gw=col0-c0（gap 宽度），copt（中心位置）
///
/// 返回 (gw, copt)，长度相同 = ngaps。
fn find_gaps(bp: &[i32], dr: i32, c1: i32, c2: i32) -> (Vec<i32>, Vec<i32>) {
    let mut gw = Vec::new();
    let mut copt = Vec::new();
    let thlow = 10;
    let thhigh = 20;
    let dr = dr.max(1);

    // C 行 2103: 外层 col0 推进循环
    let mut col0 = c1;
    while col0 <= c2 {
        // C 行 2107-2109: 跳到 bp >= thhigh
        while col0 <= c2 {
            if bp_at(bp, c1, col0) >= thhigh {
                break;
            }
            col0 += 1;
        }
        if col0 > c2 {
            break;
        }
        // C 行 2112-2114: 推进直到 bp < thlow（注意 C `for (col0++;...)` 起手 +1）
        col0 += 1;
        while col0 <= c2 {
            if bp_at(bp, c1, col0) < thlow {
                break;
            }
            col0 += 1;
        }
        if col0 >= c2 {
            break;
        }
        // C 行 2118-2124: 在 2*dr 范围内找最低 bp（copt0），bp>thhigh 提前停
        let mut copt0 = col0;
        let c0 = col0;
        while col0 <= c2 && (col0 - c0) <= 2 * dr {
            if bp_at(bp, c1, col0) < bp_at(bp, c1, copt0) {
                copt0 = col0;
            }
            if bp_at(bp, c1, col0) > thhigh {
                break;
            }
            col0 += 1;
        }
        if col0 > c2 {
            break;
        }
        if copt0 > c2 {
            copt0 = c2;
        }
        // C 行 2129-2131: 记录 gw / copt
        gw.push(col0 - c0);
        copt.push(copt0);
        // C 行 2132: col0 = copt0（继续推进）
        col0 = copt0;
        // C 行 2133-2134: 若 copt0==c2 直接退出
        if copt0 == c2 {
            break;
        }
        // 外层 for col0++ 由 Rust 显式 +1 推进
        col0 += 1;
    }
    (gw, copt)
}

/// 读 bp 数组（按绝对列号映射回 bp 索引 [col - c1]）。越界返回 0。
#[inline]
fn bp_at(bp: &[i32], c1: i32, col: i32) -> i32 {
    let idx = (col - c1) as usize;
    if idx < bp.len() {
        bp[idx]
    } else {
        0
    }
}

// =====================================================================
// 内部 helper: get_word_gap_threshold（C bmpregion.c:2155-2379）
// =====================================================================

/// 决定 word gap 阈值（pixels）。
///
/// **C 对照**：`get_word_gap_threshold`（`bmpregion.c:2155-2379`）。
///
/// 输入：
/// - `gw`：**已按降序排序**的 gap 宽度数组（C 调用前 `sortxyi + flipi`）
/// - `dr`：lcheight（dr ≥ 1）
/// - `row_width`：行宽（pixels）
///
/// 返回阈值（≥ gap_thresh 的 gap 才被视作 word 边界）。
///
/// 注意：C 版 signature 还接受 `copt[]`，但仅在 `#ifdef COMMENT` 注释代码块
/// 内（`bmpregion.c:2355-2373` 的 `word_longer_than` 内层收紧）被引用。Rust
/// 版不实现该路径，因此省略 `copt` 参数。
fn get_word_gap_threshold(gw: &[i32], dr: i32, row_width: i32, settings: &WordSettings) -> i32 {
    let ngaps = gw.len();
    // C 行 2168-2169: ngaps<=0 直接用 |word_spacing|*dr
    if ngaps == 0 {
        return (settings.word_spacing.abs() * dr as f64 + 0.5) as i32;
    }
    let dr_f = dr as f64;
    // C 行 2176: expected = row_width/(6*dr) - 1
    let mut expected = row_width as f64 / (6.0 * dr_f) - 1.0;
    // C 行 2183: display_width
    let display_width = settings.max_region_width_inches * settings.src_dpi as f64;

    // C 行 2200-2201: expected<=0 或 (expected<1.5 && gw[0]/dr<0.2) → no gaps
    if expected <= 0.0 || (expected < 1.5 && gw[0] as f64 / dr_f < 0.2) {
        return gw[ngaps - 1] + 1; // C: gw[ngaps-1] + 0.1 → 取整 → +1（更保守）
    }
    // C 行 2203-2204: word_spacing>=0 或 ngaps<2 → fixed
    if settings.word_spacing >= 0.0 || ngaps < 2 {
        return (settings.word_spacing.abs() * dr_f + 0.5) as i32;
    }
    if expected < 0.1 {
        expected = 0.1;
    }

    // C 行 2210-2214: dgap[i] = gw[i] - gw[i+1]; gapcount[i] = i+1
    let mut dgap: Vec<i32> = Vec::with_capacity(ngaps - 1);
    let mut gapcount: Vec<i32> = Vec::with_capacity(ngaps - 1);
    for i in 0..ngaps - 1 {
        dgap.push(gw[i] - gw[i + 1]);
        gapcount.push((i + 1) as i32);
    }
    // C 行 2215-2217: sort by dgap desc → 按 dgap 升序 + flip
    sort_pair_desc(&mut dgap, &mut gapcount);

    let mut gt: i32 = -1;

    // C 行 2231-2262: 找 best-centered 大 dgap
    let mut ibest: i32 = -1;
    let mut bestpos: f64 = -1.0;
    for i in 0..ngaps - 1 {
        let dgap_i = dgap[i] as f64;
        // C 行 2242: dgap[i]/dr < 0.1 → break
        if dgap_i / dr_f < 0.1 {
            break;
        }
        // C 行 2249-2250: v2.53: prev dgap 突减则 break
        if i > 0 {
            let prev_dgap = dgap[i - 1] as f64;
            if (prev_dgap - dgap_i) / dr_f > 0.35 {
                break;
            }
        }
        // C 行 2251-2252: gw 比 prev 小很多则 continue（同位置过滤）
        if i > 0 {
            let gc_i = gapcount[i] as usize;
            let gc_prev = gapcount[i - 1] as usize;
            if (gw[gc_i] as f64) / (gw[gc_prev] as f64) > 0.6 {
                continue;
            }
        }
        let pos = gapcount[i] as f64 / expected;
        // C 行 2257: 更接近 1.0 的 pos 胜出
        if bestpos < 0.0 || (pos - 1.0).abs() < (bestpos - 1.0).abs() {
            bestpos = pos;
            ibest = i as i32;
        }
    }
    if ibest >= 0 {
        // C 行 2268: gt = (gw[gc[ibest]] + gw[gc[ibest]-1]) / 2
        let gc_best = gapcount[ibest as usize] as usize;
        if gc_best >= 1 && gc_best < ngaps {
            gt = (gw[gc_best] + gw[gc_best - 1]) / 2;
        }
    } else {
        // C 行 2276-2308: fallback - 找"够大的 dgap"
        for i in 0..ngaps - 1 {
            let dgap_i = dgap[i] as f64;
            // C 行 2286: dgap[i]/dr < 0.07 → break
            if dgap_i / dr_f < 0.07 {
                break;
            }
            // C 行 2288-2292: i==0 && ngaps<=2 → 直接平均
            if i == 0 && ngaps <= 2 {
                gt = (gw[0] + gw[1]) / 2;
                break;
            }
            // C 行 2299-2300: 复杂判定
            let cond_a = i == 0 && (dgap[i + 1] as f64 / dgap_i) < 0.6;
            let cond_b = {
                let r = gapcount[i] as f64 / expected;
                (0.3..3.5).contains(&r)
            };
            if cond_a || cond_b {
                let gc_i = gapcount[i] as usize;
                if gc_i >= 1 && gc_i < ngaps {
                    gt = (gw[gc_i] + gw[gc_i - 1]) / 2;
                }
                break;
            }
        }
    }

    // C 行 2316-2351: gt 仍 < 0 → 短行 / 长行 fallback
    if gt < 0 {
        if expected < 3.5 && row_width <= display_width as i32 {
            // C 行 2321-2329: 短行
            if gw[0] as f64 / dr_f < 0.15 {
                gt = gw[0] + 1; // C: gw[0] + 0.1 → 取整
            } else {
                let mut i = (2.0 * expected + 0.5) as usize;
                if i > ngaps - 1 {
                    i = ngaps - 1;
                }
                gt = gw[i] + (0.1 * dr_f) as i32;
            }
        } else {
            // C 行 2336-2350: 长行
            let mut i = (0.35 * expected + 0.5) as usize;
            if i > ngaps - 1 {
                i = ngaps - 1;
            }
            gt = gw[i];
            if gt > (0.4 * dr_f) as i32 {
                gt /= 2;
            } else {
                gt -= (0.1 * dr_f) as i32;
            }
            if gt < 0 {
                gt = 0;
            }
        }
    }
    gt
}

// =====================================================================
// 内部 helper: paired sort helpers
// =====================================================================

/// 按第一个 vec 降序排序，第二个 vec 跟随。等价于 C `sortxyi + array_flipi`。
fn sort_pair_desc(keys: &mut Vec<i32>, vals: &mut Vec<i32>) {
    debug_assert_eq!(keys.len(), vals.len());
    let n = keys.len();
    let mut indices: Vec<usize> = (0..n).collect();
    // 升序 by key，然后 reverse → 降序
    indices.sort_by_key(|&i| keys[i]);
    indices.reverse();
    let new_keys: Vec<i32> = indices.iter().map(|&i| keys[i]).collect();
    let new_vals: Vec<i32> = indices.iter().map(|&i| vals[i]).collect();
    *keys = new_keys;
    *vals = new_vals;
}

/// 按第一个 vec 升序排序，第二个 vec 跟随。等价于 C `sortxyi`。
fn sort_pair_by_first_asc(keys: &mut Vec<i32>, vals: &mut Vec<i32>) {
    debug_assert_eq!(keys.len(), vals.len());
    let n = keys.len();
    let mut indices: Vec<usize> = (0..n).collect();
    indices.sort_by_key(|&i| keys[i]);
    let new_keys: Vec<i32> = indices.iter().map(|&i| keys[i]).collect();
    let new_vals: Vec<i32> = indices.iter().map(|&i| vals[i]).collect();
    *keys = new_keys;
    *vals = new_vals;
}

// =====================================================================
// 单元测试
// =====================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use k2core::Rect;
    use k2types::{Bitmap, PixelFormat};

    fn build_bitmap_words_row(words: &[(i32, i32)], height: u32) -> Bitmap {
        build_bitmap_words_row_with_width(words, height, 200)
    }

    fn build_bitmap_words_row_with_width(words: &[(i32, i32)], height: u32, width: u32) -> Bitmap {
        // 黑白图：每个 (start, end) 表示一个 word 的列范围，区间内填 0（暗）。
        let mut bmp = Bitmap::new(width, height, 150.0, PixelFormat::Gray8).unwrap();
        // 先全填 255
        bmp.fill_byte(255);
        // 在每个 word 范围 + 中间行（中间 60% 高度）填 0
        let y0 = height as i32 / 5;
        let y1 = height as i32 - height as i32 / 5;
        for &(c1, c2) in words {
            for r in y0..=y1 {
                for c in c1..=c2 {
                    if (c as u32) < bmp.width && (r as u32) < bmp.height {
                        if let Some(px) = bmp.pixel_mut(c as u32, r as u32) {
                            px[0] = 0;
                        }
                    }
                }
            }
        }
        bmp
    }

    #[test]
    fn word_settings_default_is_auto_mode() {
        let s = WordSettings::default();
        assert!(s.word_spacing < 0.0);
        assert_eq!(s.gtw_in, 0.0015);
        assert_eq!(s.max_region_width_inches, 3.6);
        assert_eq!(s.src_dpi, 300);
        assert_eq!(s.defect_size_pts, 1.5);
        assert!(s.src_left_to_right);
    }

    #[test]
    fn word_settings_crop_settings_propagates() {
        let s = WordSettings {
            src_left_to_right: false,
            defect_size_pts: 2.5,
            ..WordSettings::default()
        };
        let cs = s.crop_settings();
        assert!(!cs.src_left_to_right);
        assert_eq!(cs.defect_size_pts, 2.5);
    }

    #[test]
    fn database_default_empty() {
        let db = WordGapDatabase::default();
        assert!(db.is_empty());
        assert_eq!(db.len(), 0);
    }

    #[test]
    fn database_reset_clears() {
        let mut db = WordGapDatabase::new();
        db.gaps[0] = 1.5;
        db.next_index = 5;
        db.reset();
        assert!(db.is_empty());
        assert_eq!(db.len(), 0);
        assert_eq!(db.gaps[0], 0.0);
    }

    #[test]
    fn database_len_caps_at_1024() {
        let mut db = WordGapDatabase::new();
        db.next_index = 2000;
        assert_eq!(db.len(), 1024);
    }

    #[test]
    fn compute_col_gaps_empty_no_op() {
        let mut words = TextWords::new();
        compute_col_gaps(&mut words, 100);
        assert!(words.rows.is_empty());
    }

    #[test]
    fn compute_col_gaps_single_word_uses_c2_row() {
        let mut words = TextWords::new();
        words.push(TextRow {
            c1: 10,
            c2: 30,
            r1: 0,
            r2: 50,
            region_type: RowType::Word,
            ..TextRow::default()
        });
        compute_col_gaps(&mut words, 100);
        let w = &words.rows[0];
        // C 行 40-42: gap = c2_row - c2 = 100 - 30 = 70
        assert_eq!(w.gap, 70);
        assert_eq!(w.gapblank, 70);
        // rowheight = c2 - c1 = 30 - 10 = 20
        assert_eq!(w.rowheight, 20);
    }

    #[test]
    fn compute_col_gaps_multi_words_pairwise() {
        let mut words = TextWords::new();
        for &(c1, c2) in &[(10, 30), (40, 60), (70, 90)] {
            words.push(TextRow {
                c1,
                c2,
                ..TextRow::default()
            });
        }
        compute_col_gaps(&mut words, 100);
        // word 0: gap = 40-30-1 = 9
        assert_eq!(words.rows[0].gap, 9);
        // word 1: gap = 70-60-1 = 9
        assert_eq!(words.rows[1].gap, 9);
        // word 2 (last): gap = 100-90 = 10
        assert_eq!(words.rows[2].gap, 10);
        // rowheights：next.c1 - cur.c1
        assert_eq!(words.rows[0].rowheight, 40 - 10);
        assert_eq!(words.rows[1].rowheight, 70 - 40);
        assert_eq!(words.rows[2].rowheight, 90 - 70);
    }

    #[test]
    fn remove_small_col_gaps_merges_narrow_gap() {
        let mut words = TextWords::new();
        // 3 个 word：gap[0]=4 像素 narrow（合并），gap[1]=29 像素 wide（保留）
        for &(c1, c2) in &[(10, 30), (35, 60), (90, 110)] {
            words.push(TextRow {
                c1,
                c2,
                r1: 0,
                r2: 50,
                ..TextRow::default()
            });
        }
        compute_col_gaps(&mut words, 120);
        // gap[0] = 35-30-1 = 4; lcheight=20 → 4/20=0.2 → < mingap=0.5 → 合并
        // gap[1] = 90-60-1 = 29; 29/20=1.45 → >= 0.5 → 保留
        remove_small_col_gaps(&mut words, 20, 0.5, 0.1);
        assert_eq!(words.rows.len(), 2);
        // 合并后 word[0]: c1=10, c2=60
        assert_eq!(words.rows[0].c1, 10);
        assert_eq!(words.rows[0].c2, 60);
        // word[1] 保持原样
        assert_eq!(words.rows[1].c1, 90);
        assert_eq!(words.rows[1].c2, 110);
    }

    #[test]
    fn remove_small_col_gaps_keeps_when_above_threshold() {
        let mut words = TextWords::new();
        for &(c1, c2) in &[(10, 30), (50, 70), (90, 100)] {
            words.push(TextRow {
                c1,
                c2,
                ..TextRow::default()
            });
        }
        compute_col_gaps(&mut words, 110);
        // gap[0]=19/lc → 19/10=1.9, gap[1]=19/10=1.9，mingap=0.5 → 全保留
        remove_small_col_gaps(&mut words, 10, 0.5, 0.1);
        assert_eq!(words.rows.len(), 3);
    }

    #[test]
    fn remove_small_col_gaps_mingap_clamped_by_word_spacing() {
        let mut words = TextWords::new();
        for &(c1, c2) in &[(10, 30), (32, 50)] {
            words.push(TextRow {
                c1,
                c2,
                r1: 0,
                r2: 10,
                ..TextRow::default()
            });
        }
        compute_col_gaps(&mut words, 100);
        // gap = 32-30-1 = 1; lcheight 10; 1/10 = 0.1
        // mingap=0.05 应放过；但 word_spacing=0.5 强制 mingap=0.5 → 触发合并
        remove_small_col_gaps(&mut words, 10, 0.05, 0.5);
        assert_eq!(words.rows.len(), 1);
    }

    #[test]
    fn remove_small_col_gaps_zero_lcheight_no_op() {
        let mut words = TextWords::new();
        for &(c1, c2) in &[(10, 30), (40, 60)] {
            words.push(TextRow {
                c1,
                c2,
                ..TextRow::default()
            });
        }
        compute_col_gaps(&mut words, 100);
        remove_small_col_gaps(&mut words, 0, 0.5, 0.1);
        // lcheight=0 → 直接 return
        assert_eq!(words.rows.len(), 2);
    }

    #[test]
    fn add_word_gaps_records_above_threshold() {
        let mut db = WordGapDatabase::new();
        let mut words = TextWords::new();
        for &(c1, c2) in &[(10, 30), (50, 70), (90, 100)] {
            words.push(TextRow {
                c1,
                c2,
                ..TextRow::default()
            });
        }
        compute_col_gaps(&mut words, 110);
        // gap[0] = 19, gap[1] = 19 → 都 /10 = 1.9 > 0.5
        add_word_gaps(&words, 10, &mut db, 0.5);
        // 仅记录前 n-1 个，故 2 条
        assert_eq!(db.len(), 2);
    }

    #[test]
    fn add_word_gaps_skips_below_threshold() {
        let mut db = WordGapDatabase::new();
        let mut words = TextWords::new();
        for &(c1, c2) in &[(10, 30), (32, 50), (52, 70)] {
            words.push(TextRow {
                c1,
                c2,
                ..TextRow::default()
            });
        }
        compute_col_gaps(&mut words, 100);
        // gap = 1/10 = 0.1 → < 0.5 → 全跳过
        add_word_gaps(&words, 10, &mut db, 0.5);
        assert_eq!(db.len(), 0);
    }

    #[test]
    fn add_word_gaps_single_word_no_op() {
        let mut db = WordGapDatabase::new();
        let mut words = TextWords::new();
        words.push(TextRow {
            c1: 10,
            c2: 30,
            ..TextRow::default()
        });
        add_word_gaps(&words, 10, &mut db, 0.5);
        assert_eq!(db.len(), 0);
    }

    #[test]
    fn add_word_gaps_zero_lcheight_no_op() {
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
        add_word_gaps(&words, 0, &mut db, 0.5);
        assert_eq!(db.len(), 0);
    }

    #[test]
    fn compute_median_gap_empty_returns_default() {
        let db = WordGapDatabase::new();
        assert_eq!(compute_median_gap(&db), 0.7);
    }

    #[test]
    fn compute_median_gap_picks_upper_median() {
        let mut db = WordGapDatabase::new();
        for &v in &[1.0, 2.0, 3.0, 4.0] {
            db.gaps[db.next_index & 0x3ff] = v;
            db.next_index += 1;
        }
        // n=4, n/2=2, sorted=[1,2,3,4], sorted[2]=3
        assert_eq!(compute_median_gap(&db), 3.0);
    }

    #[test]
    fn compute_median_gap_unsorted_input_sorts() {
        let mut db = WordGapDatabase::new();
        for &v in &[5.0, 1.0, 4.0, 2.0, 3.0] {
            db.gaps[db.next_index & 0x3ff] = v;
            db.next_index += 1;
        }
        // n=5, n/2=2, sorted=[1,2,3,4,5], sorted[2]=3
        assert_eq!(compute_median_gap(&db), 3.0);
    }

    #[test]
    fn sort_pair_desc_basic() {
        let mut keys = vec![3, 1, 4, 1, 5];
        let mut vals = vec![10, 20, 30, 40, 50];
        sort_pair_desc(&mut keys, &mut vals);
        assert_eq!(keys, vec![5, 4, 3, 1, 1]);
        // 5对应50, 4对应30, 3对应10
        assert_eq!(vals[0], 50);
        assert_eq!(vals[1], 30);
        assert_eq!(vals[2], 10);
    }

    #[test]
    fn sort_pair_asc_basic() {
        let mut keys = vec![3, 1, 4, 1, 5];
        let mut vals = vec![10, 20, 30, 40, 50];
        sort_pair_by_first_asc(&mut keys, &mut vals);
        assert_eq!(keys, vec![1, 1, 3, 4, 5]);
        assert_eq!(vals[4], 50);
        assert_eq!(vals[2], 10);
    }

    #[test]
    fn count_text_row_pixels_uniform_band() {
        // 200 列 col_counts，行号 50 有暗带（每个列 col_count=50）
        let rect = Rect::new(10, 0, 80, 100);
        let mut col_counts = vec![0i32; 200];
        // 每列暗 50 像素（如行 30 一行都暗，但用 col_counts 直接构造）
        for v in col_counts.iter_mut().take(81).skip(10) {
            *v = 50;
        }
        let settings = WordSettings {
            gtw_in: 0.0015,
            ..WordSettings::default()
        };
        let bp = count_text_row_pixels(rect, &col_counts, 10, 150.0, &settings);
        // 每个 bp 应接近 10 * (mingap * 50) / pt
        // mingap = max(2, 10*0.02) = 2; window_size = 2; pt = (2*0.0015*150+0.5)=0.95→0
        // 但 pt clamp >=1，bp = 10*sum/1 = 10*2*50 = 1000 (在 mingap=2 时)
        assert!(bp[0] > 100);
        assert!(bp[bp.len() / 2] > 100);
    }

    #[test]
    fn count_text_row_pixels_zero_dark() {
        let rect = Rect::new(0, 0, 50, 100);
        let col_counts = vec![0i32; 200];
        let settings = WordSettings::default();
        let bp = count_text_row_pixels(rect, &col_counts, 10, 150.0, &settings);
        assert!(bp.iter().all(|&v| v == 0));
    }

    #[test]
    fn find_gaps_single_band_no_gap() {
        // bp 全部 >= thhigh（无 gap） → 找不到任何 gap
        let bp = vec![30i32; 50];
        let (gw, copt) = find_gaps(&bp, 10, 0, 49);
        assert!(gw.is_empty());
        assert!(copt.is_empty());
    }

    #[test]
    fn find_gaps_two_bands_one_gap() {
        // bp: [30 x 10] [5 x 10] [30 x 10] [5 x 10] [30 x 10]
        // 应找到两个 gap（band 之间）
        let mut bp = vec![0i32; 50];
        for v in bp.iter_mut().take(10) {
            *v = 30;
        }
        for v in bp.iter_mut().take(20).skip(10) {
            *v = 5;
        }
        for v in bp.iter_mut().take(30).skip(20) {
            *v = 30;
        }
        for v in bp.iter_mut().take(40).skip(30) {
            *v = 5;
        }
        for v in bp.iter_mut().take(50).skip(40) {
            *v = 30;
        }
        let (gw, _copt) = find_gaps(&bp, 10, 0, 49);
        assert!(!gw.is_empty(), "expected at least 1 gap, got {gw:?}");
    }

    #[test]
    fn get_word_gap_threshold_empty_falls_back_to_word_spacing() {
        let s = WordSettings {
            word_spacing: -0.2,
            ..WordSettings::default()
        };
        // ngaps=0 → 0.2 * 10 + 0.5 = 2.5 → as i32 → 2
        let gt = get_word_gap_threshold(&[], 10, 100, &s);
        assert_eq!(gt, 2);
    }

    #[test]
    fn get_word_gap_threshold_positive_word_spacing_uses_fixed() {
        let s = WordSettings {
            word_spacing: 0.3,
            ..WordSettings::default()
        };
        let gw = vec![10, 5];
        let gt = get_word_gap_threshold(&gw, 10, 100, &s);
        // word_spacing>=0 → 0.3*10+0.5 = 3.5 → as i32 → 3
        assert_eq!(gt, 3);
    }

    #[test]
    fn get_word_gap_threshold_finds_bimodal_break() {
        // expected = 200/(6*10) - 1 = 2.33
        // 大 gap 8, 8；小 gap 2, 2；显著 bi-modal
        // 已按 gw 降序：gw=[8,8,2,2]
        let s = WordSettings {
            word_spacing: -0.2,
            ..WordSettings::default()
        };
        let gw = vec![8, 8, 2, 2];
        let gt = get_word_gap_threshold(&gw, 10, 200, &s);
        // dgap = [0, 6, 0]; sort desc → [6,0,0], gapcount=[2,1,3]
        // gapcount[0]/expected = 2/2.33 = 0.86 → pos
        // ibest=0 → gt = (gw[2]+gw[1])/2 = (2+8)/2 = 5
        assert!(gt > 0, "expected positive gt, got {gt}");
    }

    #[test]
    fn one_row_find_textwords_too_narrow_returns_default() {
        // 5 列宽，width < 6 → 早退
        let bmp = build_bitmap_words_row(&[(0, 4)], 50);
        let view = RegionView::full(&bmp);
        let mut db = WordGapDatabase::new();
        let words = one_row_find_textwords(
            &view,
            Rect::new(0, 10, 4, 40),
            10,
            &WordSettings::default(),
            &mut db,
            false,
        );
        // 早退路径返回 1 个 default word
        assert_eq!(words.rows.len(), 1);
        assert_eq!(words.rows[0].region_type, RowType::Word);
    }

    #[test]
    fn one_row_find_textwords_three_words_separates() {
        // 三个明显间隔的 word：[40..80] [180..220] [320..360]
        // 用 width=400 让 row_width > 6*lcheight 触发 expected>0 路径
        let bmp = build_bitmap_words_row_with_width(&[(40, 80), (180, 220), (320, 360)], 50, 400);
        let view = RegionView::full(&bmp);
        let mut db = WordGapDatabase::new();
        // 用 word_spacing=0.5 走"固定阈值"模式（gap_thresh = 0.5*lcheight + 0.5）
        // 避免 synthetic fixture 在自动 bi-modal 模式下因 dgap=0 退化为单 word
        let settings = WordSettings {
            word_spacing: 0.5,
            ..WordSettings::default()
        };
        let words = one_row_find_textwords(
            &view,
            Rect::new(0, 5, 399, 45),
            10,
            &settings,
            &mut db,
            false,
        );
        // 应至少切出 2 个 word（理想 3 个）
        assert!(words.rows.len() >= 2, "got {} words", words.rows.len());
        // 每个 word 都是 Word 类型
        assert!(words.rows.iter().all(|w| w.region_type == RowType::Word));
        // c1 应该是单调递增（按位置切分）
        for i in 0..words.rows.len() - 1 {
            assert!(words.rows[i].c1 <= words.rows[i + 1].c1, "c1 not monotonic");
        }
    }

    #[test]
    fn one_row_find_textwords_single_word_no_split() {
        // 单个连续 word 覆盖大部分宽度
        let bmp = build_bitmap_words_row(&[(20, 180)], 50);
        let view = RegionView::full(&bmp);
        let mut db = WordGapDatabase::new();
        let words = one_row_find_textwords(
            &view,
            Rect::new(0, 5, 199, 45),
            10,
            &WordSettings::default(),
            &mut db,
            false,
        );
        // 整行就一个连续 word，不应过度拆分
        assert!(
            words.rows.len() <= 2,
            "single word over-split: {}",
            words.rows.len()
        );
    }

    #[test]
    fn one_row_find_textwords_default_word_includes_full_row() {
        // 单 word 早退路径：bbox 退回整行
        let bmp = build_bitmap_words_row(&[(0, 3)], 30);
        let view = RegionView::full(&bmp);
        let mut db = WordGapDatabase::new();
        let words = one_row_find_textwords(
            &view,
            Rect::new(0, 5, 3, 25),
            5,
            &WordSettings::default(),
            &mut db,
            false,
        );
        assert_eq!(words.rows.len(), 1);
        let w = &words.rows[0];
        // default word 端点对齐传入 rect
        assert_eq!(w.c1, 0);
        assert_eq!(w.c2, 3);
    }

    #[test]
    fn one_row_find_textwords_add_to_dbase_in_positive_mode() {
        // word_spacing>=0 + add_to_dbase=true：应记录 gap 进 db
        let bmp = build_bitmap_words_row(&[(10, 30), (60, 80), (110, 130)], 50);
        let view = RegionView::full(&bmp);
        let mut db = WordGapDatabase::new();
        let settings = WordSettings {
            word_spacing: 0.2,
            ..WordSettings::default()
        };
        let _words = one_row_find_textwords(
            &view,
            Rect::new(0, 5, 199, 45),
            10,
            &settings,
            &mut db,
            true,
        );
        // 至少有部分 gap 进入 db（取决于切分结果）
        // 不强制断言条数，只要不 panic
        let _len = db.len();
    }

    #[test]
    fn one_row_find_textwords_empty_input_returns_default() {
        // 空 bitmap row：col_counts 全 0 → 找不到任何 gap → 返回 default word
        let mut bmp = Bitmap::new(100, 30, 150.0, PixelFormat::Gray8).unwrap();
        bmp.fill_byte(255);
        let view = RegionView::full(&bmp);
        let mut db = WordGapDatabase::new();
        let words = one_row_find_textwords(
            &view,
            Rect::new(0, 5, 99, 25),
            10,
            &WordSettings::default(),
            &mut db,
            false,
        );
        // 全白 → trim 后宽度可能为 0 或 <6，应返回 default word
        assert_eq!(words.rows.len(), 1);
    }

    #[test]
    fn text_words_type_alias() {
        // TextWords = TextRows 别名验证
        let mut words: TextWords = TextWords::new();
        words.push(TextRow::default());
        assert_eq!(words.rows.len(), 1);
    }

    #[test]
    fn bp_at_in_bounds() {
        let bp = vec![10, 20, 30, 40, 50];
        assert_eq!(bp_at(&bp, 100, 100), 10);
        assert_eq!(bp_at(&bp, 100, 102), 30);
        assert_eq!(bp_at(&bp, 100, 104), 50);
    }

    #[test]
    fn bp_at_out_of_bounds_returns_zero() {
        let bp = vec![10, 20, 30];
        // col=200, c1=100 → idx=100 > len → 0
        assert_eq!(bp_at(&bp, 100, 200), 0);
    }

    #[test]
    fn database_wraparound_overwrites() {
        let mut db = WordGapDatabase::new();
        // 填到 1025 条 → 第 1025 条覆盖 index 0（1024 & 0x3ff = 0）
        for i in 0..1025 {
            db.gaps[db.next_index & 0x3ff] = i as f64;
            db.next_index += 1;
        }
        assert_eq!(db.len(), 1024);
        // i=1024 时 idx = 1024 & 1023 = 0，覆盖 gaps[0] = 1024.0
        assert_eq!(db.gaps[0], 1024.0);
        // gaps[1] 仍是 i=1 时的写入值 1.0
        assert_eq!(db.gaps[1], 1.0);
    }
}
