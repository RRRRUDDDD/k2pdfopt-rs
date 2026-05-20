//! `k2layout` - 版面分析与重排核心算法。
//!
//! 来源 C 文件：`bmpregion.c`、`pageregions.c`、`textrows.c`、`textwords.c`、
//! `wrapbmp.c`、`k2proc.c`、`k2master.c`（合计约 12000 行）。
//!
//! v2.1 关键变更：MASTERINFO 拆分由 5 桶扩到 8 桶（ADR-016）：
//! `PageState / MasterCanvas / SpacingState / WrapState / OutputPaginator /
//! OcrStaging / OutlineMapper / NativeBoxAccumulator`。
//!
//! 详见 `rust-rewrite-plan.md` v2.1 §5.2 / §8.3。
//!
//! # 模块构成
//!
//! - [`region`] + [`crop`]（Step 5.2）：bbox / blank / trim margins
//! - [`master`]（Step 5.6）：MASTERINFO 8 桶 + [`master::ConvertContext`]
//! - [`event`]（Step 5.6，fallback only）：LayoutEvent + EventConsumer（暂未启用，见 ADR-016）
//! - [`regions`]（Step 6.1）：列检测 / [`regions::PageRegions`] / [`regions::find_columns`]
//! - [`rows`]（Step 6.2）：行检测 / [`rows::TextRows`] / [`rows::find_textrows`]
//! - [`words`]（Step 6.3）：词分割 / [`words::TextWords`] / [`words::one_row_find_textwords`]
//! - [`breakpoints`]（Step 7.1）：垂直分页点 / [`breakpoints::find_break_point`]
//! - [`wrap`]（Step 8.1）：文本行 reflow 高层流水线 / [`wrap::WrapPipeline`]
//! - [`hyphen`]（Step 8.2）：行尾连字符检测 / [`hyphen::detect_hyphen`]
//! - [`justify`]（Step 8.2）：段落对齐 / [`justify::JustFlags`] + [`justify::fully_justify_with_gaps`]
//! - [`figure`]（Step 8.3，本步）：figure 直通分支 /
//!   [`figure::classify_figure`] + [`figure::compute_figure_rotation_deg`]
//! - [`reflow_pipeline`]（Step 8.4）：figure bypass + 文本 region 集成层 /
//!   [`reflow_pipeline::process_region`]

#![forbid(unsafe_code)]

pub mod breakpoints;
pub mod crop;
pub mod event;
pub mod figure;
pub mod hyphen;
pub mod justify;
pub mod master;
pub mod reflow_pipeline;
pub mod region;
pub mod regions;
pub mod rows;
pub mod words;
pub mod wrap;

pub use breakpoints::{
    apply_page_break_marks, find_break_point, find_break_point_ignoring_marks, BreakSettings,
    MARK_TYPE_BREAKPAGE, MARK_TYPE_DISABLED, MARK_TYPE_NOBREAK, MAX_PAGE_BREAK_MARKS,
};
pub use crop::{
    calc_bbox, is_blank, trim_margins, trim_margins_with_bbox, BBox, CropError, CropSettings,
    TextRowStats, TRIM_ALL, TRIM_ALL_AND_TEXT, TRIM_C1, TRIM_C2, TRIM_CALC_TEXT, TRIM_R1, TRIM_R2,
};
pub use event::{EventConsumer, LayoutEvent};
pub use figure::{
    choose_justification_flags, classify_figure, compute_figure_rotation_deg,
    evaluate_text_only_skip, is_tall_region, region_is_figure_by_aspect_ratio,
    should_invert_for_negative, FigureRotation, FigureSettings, SkipDecision,
    BREAK_PAGES_AFTER_FIGURE_SKIP, DST_NEGATIVE_TEXT_ONLY, FIGURE_JUSTIFY_USE_REGION,
};
pub use hyphen::{detect_hyphen, HyphenDetectInput};
pub use justify::{
    classify_horizontal, fully_justify_with_gaps, should_full_justify, wrectmaps_add_gap,
    JustFlags, JustifyMode,
};
pub use master::{
    output_page_from_paginator, AddRegion, ConvertContext, CropBox, FlushedLine, HyphenInfo,
    Justification, MasterCanvas, MasterGapCarry, NativeBoxAccumulator, OcrStaging, OcrWord,
    OutlineEntry, OutlineMapError, OutlineMapper, OutputPaginator, PageBreakMark, PageState,
    PaginatorPage, RegionType, SpacingState, WRectMap, WRectMaps, WrapState,
};
pub use reflow_pipeline::{
    process_region, skip_triggers_page_flush, ReflowError, ReflowOutcome, ReflowSettings,
};
pub use region::{RegionView, DEFAULT_BGCOLOR};
pub use regions::{
    col_black_count, column_height_and_gap_test, find_columns, is_clear, row_black_count,
    ClearStatus, ColumnSettings, ColumnTestStatus, PageRegion, PageRegions,
};
pub use rows::{
    compute_row_gaps, determine_type, fill_row_threshold_array, find_textrows, font_size_is_same,
    line_spacing_is_same, region_is_figure, remove_defects, remove_small_rows, scale_textrow,
    sort_by_gap, sort_by_row_position, RowSettings, RowType, TextRow, TextRows,
};
pub use words::{
    add_word_gaps, compute_col_gaps, compute_median_gap, one_row_find_textwords,
    remove_small_col_gaps, TextWords, WordGapDatabase, WordSettings,
};
pub use wrap::{AddOutcome, WrapPipeline, WrapPipelineSettings};
