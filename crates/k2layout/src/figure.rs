//! `figure` - Figure bypass 算法（Step 8.3 / M6）
//!
//! 1:1 复刻 C 版 `k2proc.c::bmpregion_add` 中 figure 处理分支（行 1287-1668），
//! 把 figure 检测、tall region 判定、text_only 跳过、negative 取反、figure 旋转、
//! figure justification 选择五条决策提炼为独立纯函数。
//!
//! # 设计目标
//!
//! - **不接 master canvas / wrap pipeline**：纯决策函数，调用方拿到结果自行编排
//!   像素操作（bmp_rotate_right_angle / bmp_invert / masterinfo_flush）
//! - **不反向依赖 k2settings**：[`FigureSettings`] 独立 struct（与 Step 6.1 /
//!   Step 6.2 / Step 8.1 同源约定）
//! - **i32 字段保留 C 同源 sentinel**：dst_break_pages / dst_figure_justify /
//!   dst_negative 等都保留 i32 以匹配 C 多枚举值（即使 k2settings/output.rs
//!   暂时简化为 bool，调用端做映射；Open Question 8.3.X 推迟到 release 前对齐）
//!
//! # C 对照
//!
//! - `k2pdfoptlib/k2proc.c:1287-1305`：tall_region / is_figure 判定（[`is_tall_region`] / [`classify_figure`]）
//! - `k2pdfoptlib/k2proc.c:1307-1315`：text_only 跳过（[`evaluate_text_only_skip`]）
//! - `k2pdfoptlib/k2proc.c:1448-1449`：dst_negative=1 figure 预反转（[`should_invert_for_negative`]）
//! - `k2pdfoptlib/k2proc.c:1454-1496`：figure 旋转决策（[`compute_figure_rotation_deg`] / [`FigureRotation`]）
//! - `k2pdfoptlib/k2proc.c:1599-1603`：tall region 用 dst_figure_justify 覆盖
//!   ([`choose_justification_flags`])
//! - `k2pdfoptlib/textrows.c:894-903`：[`region_is_figure_by_aspect_ratio`]
//! - `k2pdfoptlib/k2pdfopt.h:511-516`：REGION_TYPE_* 常量

use crate::master::RegionType;

/// Figure 跳过后是否触发分页的 break_pages 模式哨兵。
///
/// 来源：`k2pdfoptlib/k2proc.c:1312`（`if (k2settings->dst_break_pages==4) masterinfo_flush(...)`）
pub const BREAK_PAGES_AFTER_FIGURE_SKIP: i32 = 4;

/// `dst_negative=1` 哨兵：仅 text 反转，figure 保留正向（需在 master canvas
/// 反转前先做一次预反转）。
///
/// 来源：`k2pdfoptlib/k2proc.c:1448`（`if (is_figure && k2settings->dst_negative==1) bmp_invert(bmp);`）
pub const DST_NEGATIVE_TEXT_ONLY: i32 = 1;

/// `dst_figure_justify` 的 sentinel：`-1` = 与 dst_justify 一致（C 版注释：
/// "Figure justification (dst_figure_justify). -1 = same as dst_justify."）。
///
/// 来源：`k2pdfoptlib/k2pdfopt.h:320`
pub const FIGURE_JUSTIFY_USE_REGION: i32 = -1;

/// Figure 处理相关 settings（与 C `K2PDFOPT_SETTINGS` 中 figure 相关字段子集）。
///
/// # C 对照
///
/// - `dst_min_figure_height_in`：`k2pdfopt.h:322`（默认 0.75 in）
/// - `no_wrap_ar_limit`：`k2pdfopt.h`（默认 0.2，由 k2settings.c:160 设）
/// - `no_wrap_height_limit_inches`：`k2pdfopt.h`（默认 0.55 in）
/// - `text_only`：`k2pdfopt.h:267`（默认 false）
/// - `dst_break_pages`：`k2pdfopt.h:300`（i32 0/1/2/3/4，默认 1）
/// - `dst_figure_justify`：`k2pdfopt.h:320`（i32 -1/0/1/2，默认 -1）
/// - `dst_figure_rotate`：`k2pdfopt.h:321`（默认 false）
/// - `dst_negative`：`k2pdfopt.h`（i32 0/1/2，默认 0）
#[derive(Debug, Clone, Copy)]
pub struct FigureSettings {
    /// Figure 高度下限（inches）。
    pub dst_min_figure_height_in: f64,
    /// Figure aspect ratio 下限（width/height）。
    pub no_wrap_ar_limit: f64,
    /// Figure 高度第二条下限（inches，与 `dst_min_figure_height_in` 取或）。
    pub no_wrap_height_limit_inches: f64,
    /// `text_only=true` 时跳过所有 figure。
    pub text_only: bool,
    /// `dst_break_pages` 模式（4 = 跳过 figure 后强制分页）。
    pub dst_break_pages: i32,
    /// Figure 对齐策略（`-1` = 与 region 一致，0/1/2 = 左/中/右）。
    pub dst_figure_justify: i32,
    /// 是否允许把 figure 自动旋转到 landscape。
    pub dst_figure_rotate: bool,
    /// Negative 输出模式（`1` = 仅 text 反转，figure 预反转抵消；其他不影响 figure）。
    pub dst_negative: i32,
}

impl Default for FigureSettings {
    /// 与 C `k2settings_init` 同源默认值。
    fn default() -> Self {
        Self {
            dst_min_figure_height_in: 0.75,
            no_wrap_ar_limit: 0.2,
            no_wrap_height_limit_inches: 0.55,
            text_only: false,
            dst_break_pages: 1,
            dst_figure_justify: FIGURE_JUSTIFY_USE_REGION,
            dst_figure_rotate: false,
            dst_negative: 0,
        }
    }
}

/// Figure 旋转角度（仅允许 0 / +90 / -90 三态）。
///
/// 来源：`k2pdfoptlib/k2proc.c:1483`（`bmp_rotation_deg=masterinfo->landscape?-90:90`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FigureRotation {
    /// 不旋转。
    #[default]
    None,
    /// 顺时针 90°（C `bmp_rotate_right_angle(bmp, 90)`，portrait viewport）。
    Cw90,
    /// 逆时针 90°（C `bmp_rotate_right_angle(bmp, -90)`，landscape viewport）。
    Ccw90,
}

impl FigureRotation {
    /// 转为 C 同源的 `bmp_rotation_deg` 整数值。
    #[must_use]
    pub fn to_deg(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Cw90 => 90,
            Self::Ccw90 => -90,
        }
    }

    /// 是否实际触发了旋转。
    #[must_use]
    pub fn is_rotated(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// `evaluate_text_only_skip` 的返回值。
///
/// 来源：`k2pdfoptlib/k2proc.c:1307-1315`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SkipDecision {
    /// 是否跳过该 region。
    pub skip: bool,
    /// 跳过后是否额外触发一次 `masterinfo_flush`。
    pub flush_page_after_skip: bool,
}

// =====================================================================
// region_is_figure_by_aspect_ratio - C textrows.c:894-903
// =====================================================================

/// 仅按 aspect-ratio + height 判定 region 是否 figure（C `region_is_figure`，
/// `textrows.c:894-903`）。
///
/// 等价 C：
/// ```c
/// aspect_ratio = width_in / height_in;
/// return aspect_ratio > no_wrap_ar_limit
///        && (height_in > no_wrap_height_limit_inches
///            || height_in > dst_min_figure_height_in);
/// ```
///
/// `height_in <= 0` 时直接返 false（防 div-by-zero）。
#[must_use]
pub fn region_is_figure_by_aspect_ratio(
    width_in: f64,
    height_in: f64,
    settings: &FigureSettings,
) -> bool {
    if height_in <= 0.0 {
        return false;
    }
    let ar = width_in / height_in;
    ar > settings.no_wrap_ar_limit
        && (height_in > settings.no_wrap_height_limit_inches
            || height_in > settings.dst_min_figure_height_in)
}

// =====================================================================
// is_tall_region - C k2proc.c:1288
// =====================================================================

/// Tall region 判定：`region_height_inches >= dst_min_figure_height_in`（C
/// `k2proc.c:1288`）。
///
/// 注意：tall region 不一定是 figure。是否 figure 还要看 textrows 数量
/// （见 [`classify_figure`]）。
#[must_use]
pub fn is_tall_region(height_in: f64, settings: &FigureSettings) -> bool {
    height_in >= settings.dst_min_figure_height_in
}

// =====================================================================
// classify_figure - C k2proc.c:1297-1305
// =====================================================================

/// `classify_figure` 的输入：根据 textrows 数量与首行类型对 region 做 figure
/// 判定。
///
/// # C 对照（`k2proc.c:1297-1305`）
///
/// ```c
/// if (newregion->textrows.n<=0)
///     is_figure = region_is_figure(k2settings, region_width_inches, region_height_inches);
/// else if (region->textrows.n==1) {
///     textrow_determine_type(newregion, k2settings, 0);
///     is_figure = (region->textrows.textrow[0].type == REGION_TYPE_FIGURE);
/// }
/// else
///     is_figure = 0;
/// ```
///
/// # 参数
///
/// - `textrows_count`：region 内的 textrow 数量。
/// - `first_row_type`：当 `textrows_count == 1` 时，首行的已分类类型（调用方
///   先调 `crate::rows::determine_type` 后再传入）。`textrows_count != 1`
///   时此参数被忽略。
/// - `region_width_inches` / `region_height_inches`：用于 textrows_count==0
///   分支调 [`region_is_figure_by_aspect_ratio`]。
#[must_use]
pub fn classify_figure(
    textrows_count: usize,
    first_row_type: RegionType,
    region_width_inches: f64,
    region_height_inches: f64,
    settings: &FigureSettings,
) -> bool {
    if textrows_count == 0 {
        region_is_figure_by_aspect_ratio(region_width_inches, region_height_inches, settings)
    } else if textrows_count == 1 {
        // C 调用方负责先 textrow_determine_type，本 fn 只读 first_row_type
        first_row_type == RegionType::Figure
    } else {
        false
    }
}

// =====================================================================
// evaluate_text_only_skip - C k2proc.c:1307-1315
// =====================================================================

/// 评估 `text_only` 模式下是否跳过 figure，以及跳过后是否需要分页。
///
/// # C 对照（`k2proc.c:1307-1315`）
///
/// ```c
/// if (k2settings->text_only && is_figure) {
///     bmpregion_free(newregion);
///     if (k2settings->dst_break_pages == 4) /* Break page if we're skipping a figure */
///         masterinfo_flush(masterinfo, k2settings, 0);
///     return;
/// }
/// ```
///
/// # 返回
///
/// - `skip = true` 表示调用方应丢弃该 region 并立即 return（与 C 一致）。
/// - `flush_page_after_skip = true` 表示丢弃后还要调 `masterinfo_flush`。
#[must_use]
pub fn evaluate_text_only_skip(is_figure: bool, settings: &FigureSettings) -> SkipDecision {
    if settings.text_only && is_figure {
        SkipDecision {
            skip: true,
            flush_page_after_skip: settings.dst_break_pages == BREAK_PAGES_AFTER_FIGURE_SKIP,
        }
    } else {
        SkipDecision::default()
    }
}

// =====================================================================
// should_invert_for_negative - C k2proc.c:1448-1449
// =====================================================================

/// 判断在 `dst_negative=1`（仅 text 反转模式）下，是否要对 figure 做预反转
/// 以抵消下游的全图反转。
///
/// # C 对照（`k2proc.c:1448-1449`）
///
/// ```c
/// if (is_figure && k2settings->dst_negative == 1)
///     bmp_invert(bmp);
/// ```
#[must_use]
pub fn should_invert_for_negative(is_figure: bool, settings: &FigureSettings) -> bool {
    is_figure && settings.dst_negative == DST_NEGATIVE_TEXT_ONLY
}

// =====================================================================
// compute_figure_rotation_deg - C k2proc.c:1454-1496
// =====================================================================

/// 判定 figure 是否需要旋转 90° 以更好适配 viewport（C `k2proc.c:1454-1496`）。
///
/// # C 对照
///
/// ```c
/// if (is_figure && k2settings->dst_figure_rotate) {
///     double dst_vwidth_in, dst_vheight_in;
///     k2pdfopt_settings_dst_viewable(k2settings, masterinfo, &dst_vwidth_in, &dst_vheight_in);
///     if ((dst_vheight_in > dst_vwidth_in
///          && region_width_inches > region_height_inches
///          && region_width_inches > dst_vwidth_in)
///      || (dst_vheight_in < dst_vwidth_in
///          && region_width_inches < region_height_inches
///          && region_height_inches > dst_vheight_in)) {
///         bmp_rotation_deg = masterinfo->landscape ? -90 : 90;
///         bmp_rotate_right_angle(bmp, bmp_rotation_deg);
///     }
/// }
/// ```
///
/// # 决策树
///
/// 1. 非 figure 或 `dst_figure_rotate=false` → [`FigureRotation::None`]
/// 2. **portrait viewport** (vh > vw) 且 **landscape figure** (w > h) 且
///    **figure 过宽** (w > vw) → 旋转
/// 3. **landscape viewport** (vh < vw) 且 **portrait figure** (w < h) 且
///    **figure 过高** (h > vh) → 旋转
/// 4. 旋转方向：`landscape=true` → [`FigureRotation::Ccw90`]（-90°），
///    否则 [`FigureRotation::Cw90`]（+90°）
///
/// # 参数
///
/// - `is_figure`：region 是否被判为 figure。
/// - `region_width_inches` / `region_height_inches`：region 自身尺寸（inches）。
/// - `dst_vwidth_in` / `dst_vheight_in`：viewport 可视区域尺寸（C
///   `k2pdfopt_settings_dst_viewable` 输出，已含 landscape 调整）。
/// - `landscape`：MASTERINFO 的 `landscape` flag。
/// - `settings`：figure 配置。
#[must_use]
pub fn compute_figure_rotation_deg(
    is_figure: bool,
    region_width_inches: f64,
    region_height_inches: f64,
    dst_vwidth_in: f64,
    dst_vheight_in: f64,
    landscape: bool,
    settings: &FigureSettings,
) -> FigureRotation {
    if !is_figure || !settings.dst_figure_rotate {
        return FigureRotation::None;
    }
    let portrait_viewport = dst_vheight_in > dst_vwidth_in;
    let landscape_viewport = dst_vheight_in < dst_vwidth_in;
    let landscape_figure = region_width_inches > region_height_inches;
    let portrait_figure = region_width_inches < region_height_inches;
    let figure_too_wide = region_width_inches > dst_vwidth_in;
    let figure_too_tall = region_height_inches > dst_vheight_in;

    let need_rotate = (portrait_viewport && landscape_figure && figure_too_wide)
        || (landscape_viewport && portrait_figure && figure_too_tall);

    if !need_rotate {
        return FigureRotation::None;
    }
    if landscape {
        FigureRotation::Ccw90
    } else {
        FigureRotation::Cw90
    }
}

// =====================================================================
// choose_justification_flags - C k2proc.c:1599-1603
// =====================================================================

/// 为 tall region / figure 选择最终的 justification flags。
///
/// # C 对照（`k2proc.c:1599-1603`）
///
/// ```c
/// if (tall_region && k2settings->dst_figure_justify >= 0)
///     justification_flags_ex = k2settings->dst_figure_justify;
/// else
///     justification_flags_ex = added_region->justification_flags;
/// ```
///
/// 当 region 是 tall 且 `dst_figure_justify >= 0` 时，覆盖为 figure justify；
/// 否则保留 region 自带的 justification_flags。
#[must_use]
pub fn choose_justification_flags(
    tall_region: bool,
    region_justification_flags: i32,
    settings: &FigureSettings,
) -> i32 {
    if tall_region && settings.dst_figure_justify >= 0 {
        settings.dst_figure_justify
    } else {
        region_justification_flags
    }
}

// =====================================================================
// Unit tests
// =====================================================================

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn defaults() -> FigureSettings {
        FigureSettings::default()
    }

    // ---------------- FigureSettings ----------------

    #[test]
    fn settings_defaults_match_c_init() {
        let s = defaults();
        assert!((s.dst_min_figure_height_in - 0.75).abs() < 1e-9);
        assert!((s.no_wrap_ar_limit - 0.2).abs() < 1e-9);
        assert!((s.no_wrap_height_limit_inches - 0.55).abs() < 1e-9);
        assert!(!s.text_only);
        assert_eq!(s.dst_break_pages, 1);
        assert_eq!(s.dst_figure_justify, FIGURE_JUSTIFY_USE_REGION);
        assert_eq!(s.dst_figure_justify, -1);
        assert!(!s.dst_figure_rotate);
        assert_eq!(s.dst_negative, 0);
    }

    // ---------------- FigureRotation ----------------

    #[test]
    fn rotation_to_deg_round_trip() {
        assert_eq!(FigureRotation::None.to_deg(), 0);
        assert_eq!(FigureRotation::Cw90.to_deg(), 90);
        assert_eq!(FigureRotation::Ccw90.to_deg(), -90);
    }

    #[test]
    fn rotation_is_rotated_flag() {
        assert!(!FigureRotation::None.is_rotated());
        assert!(FigureRotation::Cw90.is_rotated());
        assert!(FigureRotation::Ccw90.is_rotated());
    }

    #[test]
    fn rotation_default_is_none() {
        assert_eq!(FigureRotation::default(), FigureRotation::None);
    }

    // ---------------- region_is_figure_by_aspect_ratio ----------------

    #[test]
    fn aspect_ratio_zero_height_returns_false() {
        let s = defaults();
        assert!(!region_is_figure_by_aspect_ratio(1.0, 0.0, &s));
        assert!(!region_is_figure_by_aspect_ratio(1.0, -0.1, &s));
    }

    #[test]
    fn aspect_ratio_skinny_line_returns_false() {
        // 4 in x 0.3 in：高度 0.3 < min(0.55, 0.75) 都不满足 height 条件
        let s = defaults();
        assert!(!region_is_figure_by_aspect_ratio(4.0, 0.3, &s));
    }

    #[test]
    fn aspect_ratio_below_threshold_returns_false() {
        // 0.05 in x 1.0 in：ar=0.05 不超过 0.2
        let s = defaults();
        assert!(!region_is_figure_by_aspect_ratio(0.05, 1.0, &s));
    }

    #[test]
    fn aspect_ratio_passes_height_via_no_wrap_height() {
        // height 0.6 > no_wrap_height_limit_inches=0.55 OK，ar 0.5/0.6=0.833 > 0.2 OK
        let s = defaults();
        assert!(region_is_figure_by_aspect_ratio(0.5, 0.6, &s));
    }

    #[test]
    fn aspect_ratio_passes_height_via_dst_min_figure() {
        // 调整 settings：no_wrap_height_limit_inches=2.0 让第一条不达标，
        // 但 dst_min_figure_height_in=0.5 让 height 0.6 > 0.5 满足第二条
        let s = FigureSettings {
            no_wrap_height_limit_inches: 2.0,
            dst_min_figure_height_in: 0.5,
            ..defaults()
        };
        assert!(region_is_figure_by_aspect_ratio(0.5, 0.6, &s));
    }

    // ---------------- is_tall_region ----------------

    #[test]
    fn tall_region_true_when_height_ge_threshold() {
        let s = defaults();
        assert!(is_tall_region(0.75, &s));
        assert!(is_tall_region(1.0, &s));
    }

    #[test]
    fn tall_region_false_below_threshold() {
        let s = defaults();
        assert!(!is_tall_region(0.74, &s));
        assert!(!is_tall_region(0.0, &s));
    }

    // ---------------- classify_figure ----------------

    #[test]
    fn classify_figure_zero_rows_uses_aspect_ratio() {
        let s = defaults();
        // 高 1.0 in & ar 1.0 → figure
        assert!(classify_figure(0, RegionType::Undetermined, 1.0, 1.0, &s));
        // 高 0.3 in & ar 4 → 高度不够 → 非 figure
        assert!(!classify_figure(0, RegionType::Undetermined, 1.0, 0.25, &s));
    }

    #[test]
    fn classify_figure_one_row_uses_first_row_type() {
        let s = defaults();
        // first_row_type==Figure → 是 figure
        assert!(classify_figure(1, RegionType::Figure, 5.0, 5.0, &s));
        // first_row_type!=Figure → 非 figure（即使尺寸够也不算）
        assert!(!classify_figure(1, RegionType::Text, 5.0, 5.0, &s));
    }

    #[test]
    fn classify_figure_multi_row_always_false() {
        let s = defaults();
        // textrows_count >= 2 → 一律 false
        assert!(!classify_figure(2, RegionType::Figure, 5.0, 5.0, &s));
        assert!(!classify_figure(10, RegionType::Figure, 5.0, 5.0, &s));
    }

    // ---------------- evaluate_text_only_skip ----------------

    #[test]
    fn text_only_skip_default_no_skip() {
        let s = defaults();
        let d = evaluate_text_only_skip(true, &s);
        assert!(!d.skip);
        assert!(!d.flush_page_after_skip);
    }

    #[test]
    fn text_only_skip_when_text_only_and_figure() {
        let s = FigureSettings {
            text_only: true,
            ..defaults()
        };
        let d = evaluate_text_only_skip(true, &s);
        assert!(d.skip);
        assert!(!d.flush_page_after_skip); // dst_break_pages=1, not 4
    }

    #[test]
    fn text_only_skip_with_break_pages_4_flushes() {
        let s = FigureSettings {
            text_only: true,
            dst_break_pages: BREAK_PAGES_AFTER_FIGURE_SKIP,
            ..defaults()
        };
        let d = evaluate_text_only_skip(true, &s);
        assert!(d.skip);
        assert!(d.flush_page_after_skip);
    }

    #[test]
    fn text_only_skip_not_triggered_for_non_figure() {
        let s = FigureSettings {
            text_only: true,
            dst_break_pages: BREAK_PAGES_AFTER_FIGURE_SKIP,
            ..defaults()
        };
        // 非 figure 不跳过
        let d = evaluate_text_only_skip(false, &s);
        assert!(!d.skip);
        assert!(!d.flush_page_after_skip);
    }

    // ---------------- should_invert_for_negative ----------------

    #[test]
    fn invert_for_negative_default_off() {
        let s = defaults();
        assert!(!should_invert_for_negative(true, &s));
        assert!(!should_invert_for_negative(false, &s));
    }

    #[test]
    fn invert_for_negative_triggered_only_when_both_conditions() {
        let s = FigureSettings {
            dst_negative: DST_NEGATIVE_TEXT_ONLY,
            ..defaults()
        };
        assert!(should_invert_for_negative(true, &s));
        // 非 figure 不取反
        assert!(!should_invert_for_negative(false, &s));
    }

    #[test]
    fn invert_for_negative_other_dst_negative_values_dont_trigger() {
        let s_off = FigureSettings {
            dst_negative: 0,
            ..defaults()
        };
        let s_all = FigureSettings {
            dst_negative: 2,
            ..defaults()
        };
        assert!(!should_invert_for_negative(true, &s_off));
        assert!(!should_invert_for_negative(true, &s_all));
    }

    // ---------------- compute_figure_rotation_deg ----------------

    #[test]
    fn rotation_none_when_not_figure() {
        let s = FigureSettings {
            dst_figure_rotate: true,
            ..defaults()
        };
        // is_figure=false → 无旋转
        assert_eq!(
            compute_figure_rotation_deg(false, 10.0, 1.0, 4.0, 6.0, false, &s),
            FigureRotation::None
        );
    }

    #[test]
    fn rotation_none_when_dst_figure_rotate_disabled() {
        let s = defaults();
        // dst_figure_rotate=false → 无旋转
        assert_eq!(
            compute_figure_rotation_deg(true, 10.0, 1.0, 4.0, 6.0, false, &s),
            FigureRotation::None
        );
    }

    #[test]
    fn rotation_cw90_for_wide_figure_in_portrait_viewport() {
        let s = FigureSettings {
            dst_figure_rotate: true,
            ..defaults()
        };
        // portrait viewport (vh=6 > vw=4)，landscape figure (w=10 > h=2)，
        // figure too wide (w=10 > vw=4)，landscape=false → +90
        assert_eq!(
            compute_figure_rotation_deg(true, 10.0, 2.0, 4.0, 6.0, false, &s),
            FigureRotation::Cw90
        );
    }

    #[test]
    fn rotation_ccw90_for_wide_figure_in_portrait_viewport_landscape_mode() {
        let s = FigureSettings {
            dst_figure_rotate: true,
            ..defaults()
        };
        // 同上，但 landscape=true → -90
        assert_eq!(
            compute_figure_rotation_deg(true, 10.0, 2.0, 4.0, 6.0, true, &s),
            FigureRotation::Ccw90
        );
    }

    #[test]
    fn rotation_cw90_for_tall_figure_in_landscape_viewport() {
        let s = FigureSettings {
            dst_figure_rotate: true,
            ..defaults()
        };
        // landscape viewport (vh=4 < vw=6)，portrait figure (w=2 < h=10)，
        // figure too tall (h=10 > vh=4)，landscape=false → +90
        assert_eq!(
            compute_figure_rotation_deg(true, 2.0, 10.0, 6.0, 4.0, false, &s),
            FigureRotation::Cw90
        );
    }

    #[test]
    fn rotation_none_when_figure_fits_viewport() {
        let s = FigureSettings {
            dst_figure_rotate: true,
            ..defaults()
        };
        // landscape figure (w=3 > h=2)，portrait viewport (vh=6 > vw=4)，
        // 但 figure 没超过 viewport 宽 (w=3 < vw=4) → 不旋转
        assert_eq!(
            compute_figure_rotation_deg(true, 3.0, 2.0, 4.0, 6.0, false, &s),
            FigureRotation::None
        );
    }

    #[test]
    fn rotation_none_when_aspect_mismatch() {
        let s = FigureSettings {
            dst_figure_rotate: true,
            ..defaults()
        };
        // portrait viewport (vh=6 > vw=4)，portrait figure (w=2 < h=10)
        // → 横纵向方向匹配，不需要旋转
        assert_eq!(
            compute_figure_rotation_deg(true, 2.0, 10.0, 4.0, 6.0, false, &s),
            FigureRotation::None
        );
    }

    #[test]
    fn rotation_none_when_square_viewport_or_figure() {
        let s = FigureSettings {
            dst_figure_rotate: true,
            ..defaults()
        };
        // square viewport (vh==vw)：既不 portrait 也不 landscape，永不旋转
        assert_eq!(
            compute_figure_rotation_deg(true, 10.0, 1.0, 5.0, 5.0, false, &s),
            FigureRotation::None
        );
        // square figure (w==h)：既不 portrait 也不 landscape，永不旋转
        assert_eq!(
            compute_figure_rotation_deg(true, 5.0, 5.0, 4.0, 6.0, false, &s),
            FigureRotation::None
        );
    }

    // ---------------- choose_justification_flags ----------------

    #[test]
    fn justify_uses_region_when_not_tall() {
        let s = FigureSettings {
            dst_figure_justify: 2, // right
            ..defaults()
        };
        // tall_region=false → 用 region 自己的 just
        assert_eq!(choose_justification_flags(false, 0x88, &s), 0x88);
    }

    #[test]
    fn justify_uses_figure_justify_when_tall_and_set() {
        let s = FigureSettings {
            dst_figure_justify: 1, // center
            ..defaults()
        };
        // tall_region=true & dst_figure_justify=1 → 用 figure justify
        assert_eq!(choose_justification_flags(true, 0x88, &s), 1);
    }

    #[test]
    fn justify_uses_region_when_figure_justify_is_negative() {
        let s = defaults(); // dst_figure_justify=-1
                            // tall_region=true 但 dst_figure_justify=-1 → 还是用 region 自己的
        assert_eq!(choose_justification_flags(true, 0x88, &s), 0x88);
    }

    #[test]
    fn justify_uses_figure_justify_zero_left() {
        let s = FigureSettings {
            dst_figure_justify: 0, // left（==0 仍 >= 0，触发覆盖）
            ..defaults()
        };
        assert_eq!(choose_justification_flags(true, 0x88, &s), 0);
    }

    // ---------------- 端到端流水线 smoke ----------------

    #[test]
    fn end_to_end_text_only_skip_with_break_pages_4() {
        // 模拟 C 版主分支的最小决策：text_only && figure && dst_break_pages==4
        let s = FigureSettings {
            text_only: true,
            dst_break_pages: BREAK_PAGES_AFTER_FIGURE_SKIP,
            ..defaults()
        };
        let is_figure = classify_figure(0, RegionType::Undetermined, 4.0, 2.0, &s);
        assert!(is_figure);
        let decision = evaluate_text_only_skip(is_figure, &s);
        assert!(decision.skip);
        assert!(decision.flush_page_after_skip);
    }

    #[test]
    fn end_to_end_figure_rotate_and_negative_invert() {
        // 模拟 figure 处理完整流程：is_figure → invert(negative) → rotate
        let s = FigureSettings {
            dst_figure_rotate: true,
            dst_negative: DST_NEGATIVE_TEXT_ONLY,
            ..defaults()
        };
        let is_figure = classify_figure(0, RegionType::Undetermined, 8.0, 4.0, &s);
        assert!(is_figure);
        // negative=1 + figure → 预反转
        assert!(should_invert_for_negative(is_figure, &s));
        // portrait viewport + landscape figure + figure too wide → 旋转
        let rot = compute_figure_rotation_deg(is_figure, 8.0, 4.0, 5.0, 7.0, false, &s);
        assert_eq!(rot, FigureRotation::Cw90);
        assert_eq!(rot.to_deg(), 90);
    }

    #[test]
    fn end_to_end_tall_text_region_uses_figure_justify() {
        // 模拟 tall text region（非 figure 但够高）→ 用 figure_justify
        let s = FigureSettings {
            dst_figure_justify: 1, // center
            ..defaults()
        };
        let tall = is_tall_region(1.0, &s);
        assert!(tall);
        // classify 仍判定为非 figure（textrows_count=2）
        let is_figure = classify_figure(2, RegionType::Text, 4.0, 1.0, &s);
        assert!(!is_figure);
        // 但 tall + figure_justify=1 还是覆盖 region 的 just
        let just = choose_justification_flags(tall, 0x80, &s);
        assert_eq!(just, 1);
    }
}
