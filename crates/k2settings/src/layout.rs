//! Layout settings — fields from K2PDFOPT_SETTINGS related to text/column layout detection.
//!
//! Source C: `k2pdfopt.h:237-442` (K2PDFOPT_SETTINGS struct)
//! Default init: `k2settings.c:31-241` (k2pdfopt_settings_init)

/// Text wrapping mode — replaces C int text_wrap (0/1/2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextWrap {
    /// No text wrapping (text_wrap = 0)
    Off,
    /// Standard text wrapping (text_wrap = 1) — default
    #[default]
    On,
    /// Extra text wrapping (text_wrap = 2, C's -wrap+)
    Extra,
}

impl TextWrap {
    pub fn to_c_int(&self) -> i32 {
        match self {
            TextWrap::Off => 0,
            TextWrap::On => 1,
            TextWrap::Extra => 2,
        }
    }

    pub fn from_c_int(v: i32) -> Self {
        match v {
            0 => TextWrap::Off,
            2 => TextWrap::Extra,
            _ => TextWrap::On,
        }
    }
}

/// Reflow pipeline 主路径选择（v0.2 / Step 11.4 新增）。
///
/// 控制 [`k2pipeline::convert::ConvertJob`] 主循环在 `add_bitmap` 与
/// `add_bitmap_with_reflow` 之间切换；`add_bitmap_with_reflow` 走完整的
/// figure/text 分类与 wrap reflow 流（Step 11.2/11.3 实装），`add_bitmap`
/// 维持 v0.1.0 M5 直通行为（兼容回退）。
///
/// # 默认
///
/// [`ReflowMode::Auto`]（执行计划 §11.4 默认 auto）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReflowMode {
    /// 强制走 v0.1.0 M5 直通路径（`ctx.add_bitmap`），不做 reflow 分析。
    ///
    /// 兼容回退；当 v0.2 reflow 链路出现回归或针对 single-column / 单列
    /// 简单文档想要字节级与 v0.1.0 一致时使用。
    Off,
    /// 默认值：自动走 `add_bitmap_with_reflow`（含 figure bypass + text reflow
    /// dispatch）。空 region / 全白页 / 无文字 region 内部自动 fallthrough 到
    /// `TextDirectBlit` M5 路径。
    #[default]
    Auto,
    /// 与 [`ReflowMode::Auto`] 等价当前；预留给 v0.3+ 强制单列也走 reflow 算法
    /// 的语义（即使 region 几何上为单列，也跑完整 column/row/word 切分 + wrap）。
    Force,
}

impl ReflowMode {
    /// CLI flag 字符串（与 `--reflow <MODE>` 的合法取值一一对应）。
    #[must_use]
    pub fn as_arg(&self) -> &'static str {
        match self {
            ReflowMode::Off => "off",
            ReflowMode::Auto => "auto",
            ReflowMode::Force => "force",
        }
    }

    /// 从 CLI 字符串解析 [`ReflowMode`]（大小写不敏感）。
    ///
    /// 返回 `None` 表示未知字符串（CLI 端用 clap 的 `value_parser` 校验，库
    /// 端的兼容兜底返回 `None`）。
    #[must_use]
    pub fn from_arg(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "0" | "false" | "none" => Some(ReflowMode::Off),
            "auto" | "" => Some(ReflowMode::Auto),
            "force" | "on" | "1" | "true" => Some(ReflowMode::Force),
            _ => None,
        }
    }
}

/// Layout settings — column detection, text wrapping, indentation, vertical gaps.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutSettings {
    // from k2pdfopt.h:250
    /// Column detection threshold (cdthresh). Default 0.01.
    pub cdthresh: f64,

    // from k2pdfopt.h:302
    /// Fit columns to screen (fit_columns). Default 1.
    pub fit_columns: bool,

    // from k2pdfopt.h:349
    /// Minimum column gap in inches (min_column_gap_inches). Default 0.1.
    pub min_column_gap_inches: f64,

    // from k2pdfopt.h:350
    /// Maximum column gap in inches (max_column_gap_inches). Default 1.5.
    pub max_column_gap_inches: f64,

    // from k2pdfopt.h:351
    /// Minimum column height in inches (min_column_height_inches). Default 1.5.
    pub min_column_height_inches: f64,

    // from k2pdfopt.h:360
    /// Maximum number of columns (max_columns). Default 2.
    pub max_columns: i32,

    // from k2pdfopt.h:361
    /// Column gap range (column_gap_range). Default 0.33.
    pub column_gap_range: f64,

    // from k2pdfopt.h:362
    /// Maximum column offset (column_offset_max). Default 0.3.
    pub column_offset_max: f64,

    // from k2pdfopt.h:363
    /// Column/row gap height in inches (column_row_gap_height_in). Default 1/72.
    pub column_row_gap_height_in: f64,

    // from k2pdfopt.h:364
    /// Row split figure of merit (row_split_fom). Default 20.0.
    pub row_split_fom: f64,

    // from k2pdfopt.h:365
    /// Text wrapping enabled (text_wrap). Default On.
    pub text_wrap: TextWrap,

    // from k2pdfopt.h:366
    /// Word spacing in inches (word_spacing). Default -0.20 (auto).
    pub word_spacing: f64,

    // from k2pdfopt.h:374
    /// Column fitted flag (column_fitted). Default 0.
    pub column_fitted: bool,

    // from k2pdfopt.h:380
    /// Maximum region width in inches (max_region_width_inches). Default 3.6.
    pub max_region_width_inches: f64,

    // from k2pdfopt.h:382
    /// Preserve indentation (preserve_indentation). Default 1.
    pub preserve_indentation: bool,

    // from k2pdfopt.h:383
    /// Defect size in points (defect_size_pts). Default 0.75.
    pub defect_size_pts: f64,

    // from k2pdfopt.h:384
    /// Maximum vertical gap in inches (max_vertical_gap_inches). Default 0.25.
    pub max_vertical_gap_inches: f64,

    // from k2pdfopt.h:385
    /// Vertical multiplier (vertical_multiplier). Default 1.0.
    pub vertical_multiplier: f64,

    // from k2pdfopt.h:386
    /// Vertical line spacing (vertical_line_spacing). Default -1.2.
    pub vertical_line_spacing: f64,

    // from k2pdfopt.h:387
    /// Vertical break threshold (vertical_break_threshold). Default 1.75.
    pub vertical_break_threshold: f64,

    // from k2pdfopt.h:390
    /// Hyphen detection enabled (hyphen_detect). Default 1.
    pub hyphen_detect: bool,

    // from k2pdfopt.h:391
    /// Overwrite minimum size in MB (overwrite_minsize_mb). Default 10.0.
    pub overwrite_minsize_mb: f64,

    // from k2pdfopt.h:393
    /// Rename output file (rename). Default 0.
    pub rename: bool,

    // from k2pdfopt.h:394
    /// Fit to page (dst_fit_to_page). Default 0.
    pub dst_fit_to_page: bool,

    // from k2pdfopt.h:411
    /// No-wrap aspect ratio limit (no_wrap_ar_limit). Default 0.2.
    pub no_wrap_ar_limit: f64,

    // from k2pdfopt.h:412
    /// No-wrap height limit in inches (no_wrap_height_limit_inches). Default 0.55.
    pub no_wrap_height_limit_inches: f64,

    // from k2pdfopt.h:413
    /// Little piece threshold in inches (little_piece_threshold_inches). Default 0.5.
    pub little_piece_threshold_inches: f64,

    /// Reflow pipeline 主路径选择（v0.2 / Step 11.4 新增 — 不对应 C 字段）。
    ///
    /// 由 CLI flag `--reflow <off|auto|force>` 控制；默认 [`ReflowMode::Auto`]。
    /// 调用方在构造 [`k2pipeline::convert::ConvertJob`] 时把本字段透传。
    pub reflow_mode: ReflowMode,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        // Default values from k2settings.c:31-241
        Self {
            // k2settings.c:38
            cdthresh: 0.01,
            // k2settings.c:79
            fit_columns: true,
            // k2settings.c:111
            min_column_gap_inches: 0.1,
            // k2settings.c:112
            max_column_gap_inches: 1.5,
            // k2settings.c:113
            min_column_height_inches: 1.5,
            // k2settings.c:123
            max_columns: 2,
            // k2settings.c:124
            column_gap_range: 0.33,
            // k2settings.c:125
            column_offset_max: 0.3,
            // k2settings.c:126
            column_row_gap_height_in: 1.0 / 72.0,
            // k2settings.c:114
            row_split_fom: 20.0,
            // k2settings.c:127
            text_wrap: TextWrap::On,
            // k2settings.c:128
            word_spacing: -0.20,
            // k2settings.c:131
            column_fitted: false,
            // k2settings.c:122
            max_region_width_inches: 3.6,
            // k2settings.c:138
            preserve_indentation: true,
            // k2settings.c:139
            defect_size_pts: 0.75,
            // k2settings.c:140
            max_vertical_gap_inches: 0.25,
            // k2settings.c:141
            vertical_multiplier: 1.0,
            // k2settings.c:142
            vertical_line_spacing: -1.2,
            // k2settings.c:143
            vertical_break_threshold: 1.75,
            // k2settings.c:146
            hyphen_detect: true,
            // k2settings.c:147
            overwrite_minsize_mb: 10.0,
            // k2settings.c:148
            rename: false,
            // k2settings.c:149
            dst_fit_to_page: false,
            // k2settings.c:160
            no_wrap_ar_limit: 0.2,
            // k2settings.c:161
            no_wrap_height_limit_inches: 0.55,
            // k2settings.c:162
            little_piece_threshold_inches: 0.5,
            // v0.2 / Step 11.4 默认走 add_bitmap_with_reflow（完整 figure/text 分类
            // + wrap reflow dispatch）；--reflow off 显式回退 v0.1.0 行为。
            reflow_mode: ReflowMode::Auto,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn reflow_mode_default_is_auto() {
        assert_eq!(ReflowMode::default(), ReflowMode::Auto);
        assert_eq!(LayoutSettings::default().reflow_mode, ReflowMode::Auto);
    }

    #[test]
    fn reflow_mode_as_arg_round_trip() {
        for mode in [ReflowMode::Off, ReflowMode::Auto, ReflowMode::Force] {
            let s = mode.as_arg();
            assert_eq!(ReflowMode::from_arg(s), Some(mode));
        }
    }

    #[test]
    fn reflow_mode_from_arg_case_insensitive() {
        assert_eq!(ReflowMode::from_arg("OFF"), Some(ReflowMode::Off));
        assert_eq!(ReflowMode::from_arg("Auto"), Some(ReflowMode::Auto));
        assert_eq!(ReflowMode::from_arg("FORCE"), Some(ReflowMode::Force));
    }

    #[test]
    fn reflow_mode_from_arg_aliases() {
        assert_eq!(ReflowMode::from_arg("0"), Some(ReflowMode::Off));
        assert_eq!(ReflowMode::from_arg("none"), Some(ReflowMode::Off));
        assert_eq!(ReflowMode::from_arg("1"), Some(ReflowMode::Force));
        assert_eq!(ReflowMode::from_arg("on"), Some(ReflowMode::Force));
        assert_eq!(ReflowMode::from_arg(""), Some(ReflowMode::Auto));
    }

    #[test]
    fn reflow_mode_from_arg_unknown() {
        assert_eq!(ReflowMode::from_arg("turbo"), None);
        assert_eq!(ReflowMode::from_arg("?"), None);
    }
}
