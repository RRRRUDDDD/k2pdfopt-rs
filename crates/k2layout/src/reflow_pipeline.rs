//! `reflow_pipeline` - Step 8.4 / M6 收尾：figure bypass + 文本 region 集成层。
//!
//! 把 [`crate::figure`] 决策（Step 8.3）+ [`k2core::rotate`] 像素操作
//! （Step 8.4）+ [`crate::master::ConvertContext`] master canvas 写入串成一个
//! `process_region` 入口，对应 C `k2pdfoptlib/k2proc.c::bmpregion_add`
//! （行 1287-1668）的主路径。
//!
//! # MVP 范围（按用户答复"MVP 主路径"，2026-05-18）
//!
//! - **figure 路径完整**：classify_figure → text_only_skip → invert → rotate
//!   → choose_justification → master canvas blit
//! - **文本 region 走简化直通**：返回 [`ReflowOutcome::TextDirectBlit`]
//!   让调用方自行调 [`crate::master::ConvertContext::add_bitmap`] 走 M5
//!   既有路径（保持端到端 single/two-column 输出与 Step 8.2 一致）
//!
//! # Step 11.2 / 11.3 v0.2 阶段
//!
//! - Step 11.2 起 [`analyze_text_region`] 接通 [`find_columns`] → [`find_textrows`]
//!   → [`one_row_find_textwords`] 三层算法，吐出 word 流（dest = source 占位）
//! - Step 11.3 起 [`analyze_text_region`] 把 word 流喂给 [`WrapPipeline`]，
//!   返回 `Vec<FlushedLine>`；`ReflowOutcome::TextReflowed` 字段类型从
//!   `Vec<WordLayout>` 改为 `Vec<FlushedLine>`。该 Vec 仍只在 lib 层
//!   自闭流转 — main pipeline 仍走 M5 简化直通，等 Step 11.4 才切到逐
//!   `FlushedLine` blit 路径。
//!
//! # 推迟到 Step 11.4 / 11.5 的部分
//!
//! - main pipeline 切换到 `add_bitmap_with_reflow` 默认路径（Step 11.4）
//! - `--reflow off|auto|force` CLI flag（Step 11.4）
//! - OCR 不可见层与 PDF writer 联动 word_layout / wrectmap（Step 11.5）
//! - dropcap detection 完整集成
//! - 多 column 嵌套递归
//!
//! # C 对照
//!
//! - `k2pdfoptlib/k2proc.c:1287-1668`：bmpregion_add 主入口（含 figure 与 text 双分支）
//! - `k2pdfoptlib/k2proc.c:1287-1305`：tall_region + is_figure 判定
//! - `k2pdfoptlib/k2proc.c:1307-1315`：text_only 跳过 + dst_break_pages==4 强制 flush
//! - `k2pdfoptlib/k2proc.c:1448-1449`：dst_negative=1 figure 预反转
//! - `k2pdfoptlib/k2proc.c:1454-1496`：figure rotate 决策
//! - `k2pdfoptlib/k2proc.c:1496`：bmp_rotate_right_angle 实际像素操作
//! - `k2pdfoptlib/k2proc.c:1530`：row 间显式 flush（Step 11.3 等价）
//! - `k2pdfoptlib/k2proc.c:1599-1603`：tall region 用 dst_figure_justify 覆盖

use crate::crop::CropError;
use crate::figure::{
    self, FigureRotation, FigureSettings, SkipDecision, BREAK_PAGES_AFTER_FIGURE_SKIP,
};
use crate::hyphen::{detect_hyphen, HyphenDetectInput};
use crate::master::wrap_state::{AddRegion, FlushedLine, HyphenInfo};
use crate::master::{ConvertContext, RegionType};
use crate::region::RegionView;
use crate::regions::{find_columns, ColumnSettings};
use crate::rows::{find_textrows, RowSettings};
use crate::words::{one_row_find_textwords, WordGapDatabase, WordSettings};
use crate::wrap::{WrapPipeline, WrapPipelineSettings};
use k2core::rotate;
use k2ocr::{mapping, OcrEngine, OcrError, OcrPageInput, PageSegmentationMode};
use k2settings::ocr::{OcrDetectionType, OcrMode, OcrSettings};
use k2types::{Bitmap, BitmapError, OcrWord, PixelFormat};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Settings - 集成层运行时配置
// ---------------------------------------------------------------------------

/// `reflow_pipeline` 的运行时配置（聚合 Step 8.3 figure + viewport / DPI 几何信息）。
///
/// 设计目标：
/// - 不反向依赖 `k2settings::Settings`（与 Step 6.1/6.2/6.3/8.1/8.3 同源约定）
/// - 调用方负责把 `K2PDFOPT_SETTINGS` 映射到本 struct，再传给 [`process_region`]
///
/// # v0.2 变更
///
/// - Step 11.2 加入 `column_settings` / `row_settings` / `word_settings` 三字段
///   以供 [`analyze_text_region`] 调用 column → row → word 三层算法
/// - Step 11.3 加入 `wrap_settings` 字段控制 [`WrapPipeline`] 的 `max_region_width` /
///   `text_wrap` / `src_left_to_right` / `allow_full_justification`
///
/// 因 [`RowSettings`] / [`WordSettings`] 不实现 `Copy`，本 struct 由
/// `Copy + Clone` 降级为只 `Clone`（Step 11.2 起），调用方按值传递改用 [`Clone::clone`]。
#[derive(Debug, Clone)]
pub struct ReflowSettings {
    /// Figure 决策配置（与 Step 8.3 一致）。
    pub figure: FigureSettings,
    /// 目标 viewport 可视宽度（inches）。用于 figure rotate 决策的 `dst_vwidth_in` 参数。
    ///
    /// 对应 C `k2pdfopt_settings_dst_viewable` 输出的 `dst_vwidth_in`。
    pub dst_viewport_width_in: f64,
    /// 目标 viewport 可视高度（inches）。用于 figure rotate 决策的 `dst_vheight_in` 参数。
    pub dst_viewport_height_in: f64,
    /// MASTERINFO `landscape` flag（true=横屏，影响 figure 旋转方向）。
    ///
    /// 对应 C `masterinfo->landscape`。
    pub landscape: bool,
    /// Region 的源 DPI（用于 pixel → inches 换算，水平 = 垂直）。
    ///
    /// 对应 C `src_dpi`（与 [`WrapPipelineSettings::src_dpi`] 同源）。
    pub region_dpi: f64,
    /// 列检测配置（Step 11.2 新增）。仅在 text 路径下被 [`analyze_text_region`] 使用。
    pub column_settings: ColumnSettings,
    /// 行检测配置（Step 11.2 新增）。
    pub row_settings: RowSettings,
    /// 词检测配置（Step 11.2 新增）。
    pub word_settings: WordSettings,
    /// Wrap 链路配置（Step 11.3 新增）。控制 [`WrapPipeline`] 的 `text_wrap` /
    /// `max_region_width_inches` / `src_left_to_right` / `allow_full_justification`。
    ///
    /// `src_dpi` 字段会被 [`analyze_text_region`] 自动同步为 `region_dpi`（调用方
    /// 在 master 层覆盖 `region_dpi` 时无需另外更新 `wrap_settings.src_dpi`）。
    pub wrap_settings: WrapPipelineSettings,
    /// OCR 设置（Step 11.5 新增）。仅在 [`process_region`] 收到 `ocr_engine: Some(_)`
    /// 且 `ocr_settings.dst_ocr == OcrMode::Tesseract` 且本 region 命中
    /// [`ReflowOutcome::TextReflowed`] 路径时才触发 OCR；其余路径
    /// （`FigureBypassed` / `SkippedFigure` / `TextDirectBlit`）一律不跑 OCR。
    ///
    /// 与 C `k2master.c:740-745` 等价：C 版只在 reflow 路径前调
    /// `ocrwords_from_bmp8`，figure 路径不调（避免在 figure 上浪费 tesseract CPU）。
    pub ocr_settings: OcrSettings,
}

impl Default for ReflowSettings {
    fn default() -> Self {
        Self {
            figure: FigureSettings::default(),
            // 与 K2PDFOPT_SETTINGS 默认 dst_userwidth_units / userheight_units 对齐
            // 实际值由调用方按 viewable area 提供；这里用合理默认便于单测
            dst_viewport_width_in: 6.0,
            dst_viewport_height_in: 8.0,
            landscape: false,
            region_dpi: 300.0,
            column_settings: ColumnSettings::default(),
            row_settings: RowSettings::default(),
            word_settings: WordSettings::default(),
            wrap_settings: WrapPipelineSettings::default(),
            ocr_settings: OcrSettings::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Outcome - process_region 返回值
// ---------------------------------------------------------------------------

/// [`process_region`] 的产出（描述对 region 做了什么）。
///
/// **注意**：本枚举派生 `PartialEq` 但因 [`FlushedLine`] 不实现 `PartialEq`
/// （`FlushedLine.bitmap` 含 `Vec<u8>` 但 [`Bitmap`] 未派生 `PartialEq`），
/// Step 11.3 起本枚举去掉 `PartialEq, Eq`，改用 `matches!` 模式匹配比较变体。
#[derive(Debug, Clone)]
pub enum ReflowOutcome {
    /// `text_only=true` 且 region 是 figure：region 被丢弃。
    ///
    /// `flush_page_after`：是否因 `dst_break_pages==4` 触发分页（调用方应在收到
    /// `true` 后调 [`ConvertContext::flush_page`]）。
    SkippedFigure {
        /// `true` 表示 `settings.figure.dst_break_pages == 4`，调用方必须强制分页。
        flush_page_after: bool,
    },
    /// Figure 被旁路（可能含 invert / rotate）后写入 master canvas。
    FigureBypassed {
        /// 旋转角度（[`FigureRotation::None`] 表示未旋转）
        rotation: FigureRotation,
        /// 是否被 negative-mode 预反转
        inverted: bool,
        /// 最终用于 blit 的 justification flags（已应用 `choose_justification_flags`）
        applied_just_flags: i32,
    },
    /// 文本 region：本函数仅做 figure/text 分类。调用方应自行走 M5 [`ConvertContext::add_bitmap`]
    /// 直通路径（Step 9.x / M7 起替换为完整 reflow）。
    TextDirectBlit,
    /// v0.2 / Step 11.3：完整文本 reflow 后的 wrap 已对齐整行结果集合。
    ///
    /// 携带 wrap_state flush 出的 [`FlushedLine`] 流，已按 column → row → word 顺序
    /// 喂给 [`WrapPipeline`] 并触发 `should_flush` 或 row/column 边界后产生。
    /// Step 11.4 main pipeline 切换到 `add_bitmap_with_reflow` 默认路径时，
    /// 调用方应把每条 [`FlushedLine`] 逐个 blit 到 master canvas。
    /// 详见 `post-release-fix-plan.md` §6.1 与 `post-release-fix-execution-plan.md`
    /// Step 11.1 / Step 11.2 / Step 11.3。
    ///
    /// Step 11.5 起 [`ReflowOutcome::TextReflowed`] 还携带 `ocr_words` 字段：当
    /// `process_region` 被传入 `ocr_engine: Some(_)` 且 `ocr_settings.dst_ocr ==
    /// OcrMode::Tesseract` 时，在本路径内对 `region_bitmap` 跑 OCR，并把识别出的
    /// word 坐标平移到 master canvas 全局坐标系（与 Step 9.3 `ConvertJob::run`
    /// 中 `dy = canvas.rows + gap` 同源）。其余路径（figure / skip / direct_blit）
    /// 一律不跑 OCR（C `k2master.c:740-745` 等价语义）。
    TextReflowed {
        /// 本 region 产出的 wrap 已对齐整行流（按 column → row → wrap-flush 顺序排列；
        /// 可能为空表示无文字识别）。
        lines: Vec<FlushedLine>,
        /// 本 region 已平移到 master canvas 全局坐标的 OCR 识别词流。
        ///
        /// - `ocr_engine = None` → 空 `Vec`
        /// - `ocr_settings.dst_ocr != OcrMode::Tesseract` → 空 `Vec`
        /// - OCR 引擎实际返 0 word → 空 `Vec`
        /// - 其余情况：词流已按 `dy = ctx.canvas.rows + gap` 偏移
        ocr_words: Vec<OcrWord>,
    },
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// [`process_region`] 错误类型。
#[derive(Debug, Error)]
pub enum ReflowError {
    /// Region 像素 buffer 大小与 width × height × bpp 不匹配。
    #[error("region bitmap construction failed: {0}")]
    BitmapConstruction(#[from] BitmapError),
    /// 文本 region 三层算法（find_columns / find_textrows）传播的 crop 错误。
    ///
    /// Step 11.2 新增：仅在 [`analyze_text_region`] 路径下触发；零尺寸 / 越界 region
    /// 在更早的早返路径已被拦截，正常 fixture 不会触发本变体。
    #[error("text region analysis failed: {0}")]
    LayoutAnalysis(#[from] CropError),
    /// Step 11.5 新增：OCR 引擎在 [`ReflowOutcome::TextReflowed`] 路径内调
    /// `recognize` 返回错误（tesseract 子进程异常 / 临时文件写盘失败 / TSV 解析错误等）。
    ///
    /// 等价 C `ocrtess_ocrwords_from_bmp8` 失败时通过 `errcnt` 累加的错误位（k2ocr.c
    /// 等价位置）。本 enum 优先于 Layout / Bitmap 变体，调用方应区分对待
    /// （例如把 OCR 错误降级为 warning + 跳过本页 OCR，不中断整个 pipeline）。
    #[error("ocr engine failed in TextReflowed path: {0}")]
    Ocr(#[from] OcrError),
}

// ---------------------------------------------------------------------------
// 主入口
// ---------------------------------------------------------------------------

/// 处理一个 region：figure → bypass / text → 直通 (调用方自行 add_bitmap)。
///
/// # 算法
///
/// 1. **几何换算**：`w_in = region_width / region_dpi`，`h_in = region_height / region_dpi`
/// 2. **figure 判定**：[`figure::region_is_figure_by_aspect_ratio`]（MVP 不调
///    `find_textrows`，按 aspect ratio 即可；完整 [`figure::classify_figure`] 推迟
///    Step 9.x）
/// 3. **tall 判定**：[`figure::is_tall_region`]
/// 4. **text_only 跳过**：[`figure::evaluate_text_only_skip`]，命中则返回
///    [`ReflowOutcome::SkippedFigure`]
/// 5. **若非 figure 且非 tall**：跑 [`analyze_text_region`] → 空 lines 返
///    [`ReflowOutcome::TextDirectBlit`]，非空 lines 在 Step 11.5 起额外调
///    [`run_region_ocr`] 跑 OCR（仅当 `ocr_engine: Some(_)` 且
///    `settings.ocr_settings.dst_ocr == OcrMode::Tesseract`），返
///    [`ReflowOutcome::TextReflowed`]
/// 6. **figure 路径**（is_figure || is_tall）：
///    - 复制 region pixels 到本地 Bitmap
///    - [`figure::should_invert_for_negative`] → [`rotate::invert`]
///    - [`figure::compute_figure_rotation_deg`] → [`rotate::rotate_right_angle`]
///    - [`figure::choose_justification_flags`] 选 just flags
///    - 调 `ctx.canvas.fill_gap(0, Figure)` + `ctx.canvas.blit(...)` + `ctx.spacing.update_after_add`
///    - figure / tall 路径**不**跑 OCR（C `k2master.c:740-745` 等价）
///
/// # 参数
///
/// - `ctx`：master canvas 容器
/// - `region_pixels` / `region_width` / `region_height` / `region_format`：源 region 位图
/// - `region_just_flags`：region 自带的 justification flags（C `region->ka.just` /
///   `region->bbox.type` 经 wrap_state 处理后传过来）。MVP 直接透传
/// - `settings`：本步配置（含 `ocr_settings` Step 11.5 新增字段）
/// - `ocr_engine`（Step 11.5 新增）：可选的 OCR 引擎。`None` 表示完全不跑 OCR
///   （即使 `settings.ocr_settings.dst_ocr == Tesseract` 也只是个声明，没有
///   引擎实例就不能识别）；`Some(_)` 时仅在 [`ReflowOutcome::TextReflowed`]
///   路径才会触发 `engine.recognize(...)`
///
/// # 错误
///
/// - [`ReflowError::BitmapConstruction`]：figure 路径下 `Bitmap::from_raw` 失败
///   （像素 buffer 大小不匹配；text 路径不构造 Bitmap 不会触发）
/// - [`ReflowError::Ocr`]（Step 11.5 新增）：TextReflowed 路径下
///   `engine.recognize` 返回错误（tesseract 子进程异常 / 临时文件错误等）
pub fn process_region(
    ctx: &mut ConvertContext,
    region_pixels: &[u8],
    region_width: u32,
    region_height: u32,
    region_format: PixelFormat,
    region_just_flags: i32,
    settings: &ReflowSettings,
    ocr_engine: Option<&dyn OcrEngine>,
) -> Result<ReflowOutcome, ReflowError> {
    if region_width == 0 || region_height == 0 {
        // 零尺寸 region 视为 text 直通（与 add_bitmap 早返路径一致）
        return Ok(ReflowOutcome::TextDirectBlit);
    }
    if settings.region_dpi <= 0.0 {
        // DPI 非法 → 直通（防御性 fallback，与 C 不调用 figure 决策一致）
        return Ok(ReflowOutcome::TextDirectBlit);
    }

    let w_in = f64::from(region_width) / settings.region_dpi;
    let h_in = f64::from(region_height) / settings.region_dpi;

    // ---- Step 1+2+3：figure / tall 判定 ----
    let is_figure = figure::region_is_figure_by_aspect_ratio(w_in, h_in, &settings.figure);
    let is_tall = figure::is_tall_region(h_in, &settings.figure);

    // ---- Step 4：text_only 跳过 ----
    let skip: SkipDecision = figure::evaluate_text_only_skip(is_figure, &settings.figure);
    if skip.skip {
        return Ok(ReflowOutcome::SkippedFigure {
            flush_page_after: skip.flush_page_after_skip,
        });
    }

    // ---- Step 5：text 路径（非 figure / 非 tall） ----
    // Step 11.2：构造 region 副本喂 column → row → word 三层算法。
    // Step 11.3：再把 word 流喂给 WrapPipeline 拿 Vec<FlushedLine>。
    // Step 11.5：非空 lines 时额外在本 region 上跑 OCR（仅当 ocr_engine: Some(_)
    //   且 settings.ocr_settings.dst_ocr == OcrMode::Tesseract）。figure / skip
    //   / direct_blit 三路径不跑 OCR，与 C `k2master.c:740-745` 等价：C 版只在
    //   reflow 路径前调 ocrwords_from_bmp8，figure 路径不调（避免浪费 CPU）。
    //
    // Step 11.4 Open Question 11.4.A：main pipeline 整页输入时本判定会 figure-bypass
    // 双列文本（h ≥ 1.5 in 整页满足 is_figure/is_tall）。完整修复需要 main pipeline
    // 先做 column/row 切分再喂局部 region，对应 C `bmpregion_add` 上游栈的 column
    // 拆分（k2proc.c:2230-2280）。本步保留 figure-first 顺序以维持 Step 8.4 / 11.3
    // 全部 figure 测试不退步，端到端验收「two-column ≤ 4 页」推迟 Step 11.5+。
    if !is_figure && !is_tall {
        let region_bitmap = Bitmap::from_raw(
            region_width,
            region_height,
            settings.region_dpi as f32,
            region_format,
            region_pixels.to_vec(),
        )?;
        let lines = analyze_text_region(&region_bitmap, region_just_flags, settings)?;
        if lines.is_empty() {
            return Ok(ReflowOutcome::TextDirectBlit);
        }
        // Step 11.5：TextReflowed 路径调 OCR。
        // 注意：必须在 process_region 写 canvas 之前（即此 return 之前）跑 OCR，
        // 拿到的 dy = ctx.canvas.rows + gap 与 Step 9.3 ConvertJob::run
        // 主循环 OCR 块的 dy 计算同源（"region 写入前 + gap"）。
        let ocr_words = run_region_ocr(ocr_engine, &settings.ocr_settings, &region_bitmap, ctx)?;
        return Ok(ReflowOutcome::TextReflowed { lines, ocr_words });
    }

    // ---- Step 6：figure / tall 路径 ----
    // 6.1 region 副本（避免修改源 pixels）
    let mut bmp = Bitmap::from_raw(
        region_width,
        region_height,
        settings.region_dpi as f32,
        region_format,
        region_pixels.to_vec(),
    )?;

    // 6.2 invert（dst_negative==1 且 is_figure）
    let inverted = figure::should_invert_for_negative(is_figure, &settings.figure);
    if inverted {
        rotate::invert(&mut bmp);
    }

    // 6.3 rotate（dst_figure_rotate=true 且 viewport/figure 方向不匹配）
    let rotation = figure::compute_figure_rotation_deg(
        is_figure,
        w_in,
        h_in,
        settings.dst_viewport_width_in,
        settings.dst_viewport_height_in,
        settings.landscape,
        &settings.figure,
    );
    if rotation.is_rotated() {
        rotate::rotate_right_angle(&mut bmp, rotation.to_deg());
    }

    // 6.4 选 just flags
    let final_just =
        figure::choose_justification_flags(is_tall, region_just_flags, &settings.figure);

    // 6.5 blit 到 master canvas
    let gap = ctx.calculate_line_gap(settings.region_dpi);
    ctx.canvas.fill_gap(gap, RegionType::Figure);
    ctx.canvas
        .blit(&bmp.pixels, bmp.width, bmp.height, final_just);
    ctx.spacing
        .update_after_add(bmp.height, settings.region_dpi);

    Ok(ReflowOutcome::FigureBypassed {
        rotation,
        inverted,
        applied_just_flags: final_just,
    })
}

/// 检查 [`ReflowOutcome::SkippedFigure`] 的 `flush_page_after` 是否对应
/// C `dst_break_pages==4` 行为（便于调用方文档化分支）。
///
/// 等价 `settings.figure.dst_break_pages == BREAK_PAGES_AFTER_FIGURE_SKIP`。
#[must_use]
pub fn skip_triggers_page_flush(settings: &ReflowSettings) -> bool {
    settings.figure.dst_break_pages == BREAK_PAGES_AFTER_FIGURE_SKIP
}

// ---------------------------------------------------------------------------
// run_region_ocr - Step 11.5: TextReflowed 路径 OCR helper
// ---------------------------------------------------------------------------

/// 在 [`ReflowOutcome::TextReflowed`] 路径内对 `region_bitmap` 跑 OCR，并把
/// 识别出的 word 坐标平移到 master canvas 全局坐标系。
///
/// **Step 11.5 阶段范围**：把 OCR 调用点从 `ConvertJob::run` 主循环（Step 9.3
/// 实装位置）移到 `process_region` 内 [`ReflowOutcome::TextReflowed`] 路径
/// 之前，保证 figure / skip / direct_blit 三路径不跑 OCR（C `k2master.c:740-745`
/// 等价语义：只在 reflow 路径前调 `ocrwords_from_bmp8`，figure 路径不调）。
///
/// # 短路条件（返空 Vec）
///
/// - `engine = None`：调用方未注入 OCR 引擎
/// - `settings.dst_ocr != OcrMode::Tesseract`：当前仅 Tesseract 路径触发 OCR
///   （`Off` / `Mupdf` 都不跑——`Mupdf` 是 native text extraction，由 M8+ writer
///   端处理而非 OCR）
///
/// # OCR 调用参数（同 [`k2pipeline::ocr_bridge::build_ocr_input`] / `recognize_for_master`）
///
/// - `lang`：`settings.dst_ocr_lang.is_empty()` 时回退 `"eng"`（与 Step 9.3 / 9.4 一致）
/// - `psm`：按 `settings.ocr_detection_type` 映射：
///   - [`OcrDetectionType::Word`] → [`PageSegmentationMode::SingleWord`]（PSM 8）
///   - [`OcrDetectionType::Line`] → [`PageSegmentationMode::SingleTextLine`]（PSM 7）
///   - [`OcrDetectionType::Paragraph`] → [`PageSegmentationMode::SingleColumnVarSize`]（PSM 4）
/// - `dpi`：`settings.ocr_dpi > 0` 时用 `ocr_dpi`，否则用 `region_bitmap.dpi`
///
/// # 坐标系平移
///
/// `dy = ctx.canvas.rows as f64 + gap`，其中 `gap = ctx.calculate_line_gap(region_bitmap.dpi)`。
/// 这与 Step 9.3 `ConvertJob::run` 主循环 OCR 块的 `dy` 计算完全一致——保证 Step
/// 11.5 切换 OCR 调用点后 PDF 输出 OCR 不可见层坐标不变（v0.1.0 → v0.2 兼容）。
///
/// # 错误
///
/// 直接透传 `engine.recognize` 返回的 [`OcrError`] 经 [`ReflowError::Ocr`]
/// 包装（`#[from] OcrError` 自动转换）。
///
/// # C 对照
///
/// `k2pdfoptlib/k2master.c:740-745`：
///
/// ```c
/// // 调 OCR 拿 word 列表
/// ocrtess_ocrwords_from_bmp8(words, src, x1, y1, x2, y2, lang, ...);
/// // 平移到 master 坐标系
/// ocrwords_offset(words, dw, masterinfo->rows + gap_start);
/// ```
fn run_region_ocr(
    engine: Option<&dyn OcrEngine>,
    settings: &OcrSettings,
    region_bitmap: &Bitmap,
    ctx: &ConvertContext,
) -> Result<Vec<OcrWord>, ReflowError> {
    let engine = match engine {
        Some(e) => e,
        None => return Ok(Vec::new()),
    };
    if !matches!(settings.dst_ocr, OcrMode::Tesseract) {
        return Ok(Vec::new());
    }
    let psm = match settings.ocr_detection_type {
        OcrDetectionType::Word => PageSegmentationMode::SingleWord,
        OcrDetectionType::Line => PageSegmentationMode::SingleTextLine,
        OcrDetectionType::Paragraph => PageSegmentationMode::SingleColumnVarSize,
    };
    let ocr_dpi = if settings.ocr_dpi > 0 {
        settings.ocr_dpi as f32
    } else {
        region_bitmap.dpi
    };
    let lang = if settings.dst_ocr_lang.is_empty() {
        "eng".to_string()
    } else {
        settings.dst_ocr_lang.clone()
    };
    let input = OcrPageInput::new(region_bitmap, ocr_dpi)
        .with_lang(lang)
        .with_psm(psm);
    let mut words = engine.recognize(&input)?;
    // dy 与 Step 9.3 ConvertJob::run 主循环 OCR 块同源（dy = canvas.rows + gap）
    let gap = ctx.calculate_line_gap(f64::from(region_bitmap.dpi));
    let dy = ctx.canvas.rows as f64 + f64::from(gap);
    if dy != 0.0 {
        mapping::offset(&mut words, 0.0, dy);
    }
    Ok(words)
}

// ---------------------------------------------------------------------------
// analyze_text_region - Step 11.3: column → row → word → wrap_state 四层管线
// ---------------------------------------------------------------------------

/// 在一个 text region 上跑完整 column → row → word → wrap_state 四层管线，
/// 输出 [`FlushedLine`] 流。
///
/// **Step 11.3 阶段范围**（与执行计划 §11.3 一致）：
///
/// - 调 [`find_columns`] 把 region 拆为多列 [`crate::regions::PageRegion`]
/// - 对每个列调 [`find_textrows`] 拿 [`crate::rows::TextRows`]
/// - 对每个 row 先 [`detect_hyphen`] 检测行尾 hyphen，再调
///   [`one_row_find_textwords`] 拿 word bbox
/// - 把 word 流喂给 self-contained 实例化的 [`WrapPipeline`]，每个 word
///   `add_word` 后查 `should_flush`；触发或 row / column 结束时 `flush`
///   收 [`FlushedLine`]
///
/// # 输入
///
/// - `region_bitmap`：text region 的源位图副本（由 [`process_region`] 用
///   `Bitmap::from_raw(region_pixels.to_vec())` 构造；本函数仅 borrow 不
///   mutate）
/// - `region_just_flags`：region 自带 justification flags（来自调用方，
///   传给 [`WrapPipeline::add_word`]）
/// - `settings`：必读 `column_settings` / `row_settings` / `word_settings` /
///   `wrap_settings` / `region_dpi`。`figure` 与 `dst_viewport_*` 字段在本
///   函数中不使用
///
/// # 输出
///
/// - `Ok(Vec<FlushedLine>)`：按 column → row → wrap-flush 顺序的整行流；空
///   `Vec` 表示 region 无可识别文本（全白或仅噪点）
///
/// # 错误
///
/// - [`ReflowError::LayoutAnalysis`]：column / row 检测内部 crop 越界
///   （正常 fixture 不会触发；零尺寸已在 [`process_region`] 早返路径拦截）
/// - [`ReflowError::BitmapConstruction`]：wrap_state 内部 [`Bitmap::from_raw`]
///   尺寸溢出（罕见 — 单 region 累积 wrap 缓冲区 > 4GiB 才触发）
///
/// # C 对照
///
/// `k2pdfoptlib/k2proc.c:1287-1668` 主路径中 column 拆分 + textrow 迭代 +
/// word 切分 + wrap_state add/flush 四层调用顺序。`k2proc.c:1530` 行尾显式
/// flush（row 之间清空 wrap 缓冲区）对应本函数 row 末尾的 `wrap.flush()`。
fn analyze_text_region(
    region_bitmap: &Bitmap,
    region_just_flags: i32,
    settings: &ReflowSettings,
) -> Result<Vec<FlushedLine>, ReflowError> {
    let columns = find_columns(region_bitmap, &settings.column_settings)?;
    let view = RegionView::full(region_bitmap);
    let mut dbase = WordGapDatabase::new();
    let mut lines: Vec<FlushedLine> = Vec::new();

    // Step 11.3：WrapPipeline self-contained 实例化（不污染 ctx.wrap）。
    // src_dpi 用 region_dpi 覆盖 wrap_settings 默认（与 master::add_bitmap_with_reflow 中
    // effective.region_dpi = dpi 的覆盖策略同源）。
    let mut wrap_settings = settings.wrap_settings;
    wrap_settings.src_dpi = settings.region_dpi;
    // is_color 由源 bitmap format 决定（与 wrapbmp_set_color 同源）
    let is_color = !matches!(region_bitmap.format, PixelFormat::Gray8);
    let mut wrap = WrapPipeline::new(wrap_settings, is_color);

    for page_region in &columns.regions {
        let column_view = view.with_rect(page_region.rect);
        // Step 11.2 MVP：dynamic_aperture=true / remove_small_rows=false /
        // join_figure_captions=false 与 C k2proc.c:1314-1320 默认调用模式一致；
        // minrowgap_in 取 row_settings.max_vertical_gap_inches（同 C 行 1319）。
        let rows = find_textrows(
            &column_view,
            &settings.row_settings,
            true,
            false,
            settings.row_settings.max_vertical_gap_inches,
            false,
        )?;
        for textrow in &rows.rows {
            let row_rect = textrow.rect();
            // Step 11.2 MVP：lcheight 取 textrow.lcheight（C 行 1808 同源）。
            let lcheight_for_words = textrow.lcheight.max(1);
            let words = one_row_find_textwords(
                &column_view,
                row_rect,
                lcheight_for_words,
                &settings.word_settings,
                &mut dbase,
                false,
            );
            if words.rows.is_empty() {
                continue;
            }
            // Step 11.3：row 整体 hyphen 检测（C wrapbmp_add 在 add_word 之前调
            // bmpregion_hyphen_detect）。只对该 row 最后一个 word 填 hyphen 信息，
            // 前面的 word 用 HyphenInfo::none() — 等价 C 语义：hyphen 是行尾属性。
            let hyphen_info = detect_hyphen(&HyphenDetectInput::new(
                region_bitmap,
                textrow.c1,
                textrow.c2,
                textrow.r1,
                textrow.r2,
                textrow.rowbase,
                textrow.capheight.max(0),
                textrow.lcheight.max(0),
                column_view.bgcolor,
                settings.wrap_settings.src_left_to_right,
            ));
            let last_idx = words.rows.len() - 1;
            for (word_idx, word) in words.rows.iter().enumerate() {
                // colgap 计算：第一个 word colgap = 0；其余取前一个 word.gap
                // （由 one_row_find_textwords::compute_col_gaps 算好，从 word[i-1]
                // 到 word[i] 的横向 gap，与 C `wrapbmp_add` 的 `colgap` 入参同源）
                let colgap = if word_idx == 0 {
                    0
                } else {
                    words.rows[word_idx - 1].gap.max(0)
                };
                let region_param = AddRegion {
                    pixels: &region_bitmap.pixels,
                    src_full_width: region_bitmap.width,
                    src_full_height: region_bitmap.height,
                    format: region_bitmap.format,
                    c1: word.c1,
                    c2: word.c2,
                    r1: word.r1,
                    r2: word.r2,
                    rowbase: word.rowbase,
                    rowheight: textrow.rowheight,
                    gap: textrow.gap,
                    gapblank: textrow.gapblank,
                    bgcolor: column_view.bgcolor,
                    pageno: 0,
                    dpi: settings.region_dpi,
                    rotdeg: 0,
                    // 行尾 hyphen 只在该 row 的最后一个 word 上携带；
                    // 其余 word 用 HyphenInfo::none()
                    hyphen: if word_idx == last_idx {
                        hyphen_info
                    } else {
                        HyphenInfo::none()
                    },
                };
                let outcome = wrap.add_word(&region_param, colgap, region_just_flags, 0, 0.0)?;
                if outcome.should_flush {
                    if let Some(line) = wrap.flush()? {
                        lines.push(line);
                    }
                }
            }
            // Step 11.3：row 之间显式 flush（对应 C k2proc.c:1530）
            if let Some(line) = wrap.flush()? {
                lines.push(line);
            }
        }
        // Step 11.3：column 之间显式 flush（防御性 — row 末尾已 flush，
        // 这里幂等保证不会把 column1 末尾与 column2 开头拼成一行）
        if let Some(line) = wrap.flush()? {
            lines.push(line);
        }
    }

    Ok(lines)
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;
    use crate::figure::FigureRotation;
    use k2types::PixelFormat;

    // ---- helper ----

    fn mk_ctx_gray8(width: u32) -> ConvertContext {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(width, PixelFormat::Gray8);
        ctx
    }

    // ---- ReflowSettings defaults ----

    #[test]
    fn settings_default_matches_c_init() {
        let s = ReflowSettings::default();
        assert_eq!(s.figure.dst_min_figure_height_in, 0.75);
        assert!((s.dst_viewport_width_in - 6.0).abs() < 1e-9);
        assert!((s.dst_viewport_height_in - 8.0).abs() < 1e-9);
        assert!(!s.landscape);
        assert_eq!(s.region_dpi, 300.0);
    }

    // ---- 边界：零尺寸 / 非法 DPI ----

    #[test]
    fn zero_size_region_returns_text_direct_blit() {
        let mut ctx = mk_ctx_gray8(100);
        let s = ReflowSettings::default();
        let out = process_region(&mut ctx, &[], 0, 10, PixelFormat::Gray8, 0, &s, None).unwrap();
        assert!(matches!(out, ReflowOutcome::TextDirectBlit));
        let out = process_region(&mut ctx, &[], 10, 0, PixelFormat::Gray8, 0, &s, None).unwrap();
        assert!(matches!(out, ReflowOutcome::TextDirectBlit));
    }

    #[test]
    fn nonpositive_dpi_returns_text_direct_blit() {
        let mut ctx = mk_ctx_gray8(100);
        let mut s = ReflowSettings::default();
        s.region_dpi = 0.0;
        let pixels = vec![128u8; 10 * 10];
        let out =
            process_region(&mut ctx, &pixels, 10, 10, PixelFormat::Gray8, 0, &s, None).unwrap();
        assert!(matches!(out, ReflowOutcome::TextDirectBlit));
    }

    // ---- text 路径（非 figure 非 tall） ----

    #[test]
    fn small_text_region_returns_text_outcome() {
        let mut ctx = mk_ctx_gray8(100);
        let s = ReflowSettings::default();
        // 100x100 px @ 300 dpi = 0.33 x 0.33 in → 既不 figure (ar=1.0 > 0.2)
        // 也不 tall (h=0.33 < dst_min_figure_height_in=0.75)
        // figure 判定：aspect ratio=1/0.33=3, h=0.33 < no_wrap_height_limit=0.55
        //   → 不满足 ar > 0.2 && h > 0.55 → 不是 figure
        // tall 判定：h=0.33 < 0.75 → 不 tall
        // Step 11.2 后：text 路径会跑 column → row → word 三层分析；
        // 全灰 region（128）整片被视为单个大文本块 → TextReflowed。
        let pixels = vec![128u8; 100 * 100];
        let out =
            process_region(&mut ctx, &pixels, 100, 100, PixelFormat::Gray8, 0, &s, None).unwrap();
        assert!(
            matches!(
                out,
                ReflowOutcome::TextDirectBlit | ReflowOutcome::TextReflowed { .. }
            ),
            "text 路径应返 TextDirectBlit 或 TextReflowed, got {out:?}"
        );
        // text 路径下 process_region 自身不写 canvas（Step 11.3 wrap_state 接入前）
        assert_eq!(ctx.canvas.rows, 0);
    }

    #[test]
    fn blank_text_region_returns_text_direct_blit() {
        let mut ctx = mk_ctx_gray8(100);
        let s = ReflowSettings::default();
        // 全白 100x100 region：column → row 都返回空 → analyze_text_region 返空 Vec
        // → process_region 返 TextDirectBlit（与 Step 11.1 之前完全一致）
        let pixels = vec![255u8; 100 * 100];
        let out =
            process_region(&mut ctx, &pixels, 100, 100, PixelFormat::Gray8, 0, &s, None).unwrap();
        assert!(matches!(out, ReflowOutcome::TextDirectBlit));
        assert_eq!(ctx.canvas.rows, 0);
    }

    // ---- figure 路径：直接 blit ----

    #[test]
    fn figure_region_blits_to_canvas() {
        let mut ctx = mk_ctx_gray8(100);
        let s = ReflowSettings::default();
        // 用 width=90, height=300 测试：w_in=0.3, h_in=1.0
        // ar = 0.3/1.0 = 0.3 > 0.2 ✓; h=1.0 > 0.55 ✓ → is_figure
        let pixels = vec![64u8; 90 * 300];
        let out =
            process_region(&mut ctx, &pixels, 90, 300, PixelFormat::Gray8, 0, &s, None).unwrap();
        match out {
            ReflowOutcome::FigureBypassed {
                rotation, inverted, ..
            } => {
                assert_eq!(rotation, FigureRotation::None); // dst_figure_rotate=false
                assert!(!inverted); // dst_negative=0
            }
            other => panic!("expected FigureBypassed, got {other:?}"),
        }
        // canvas 应写入 300 行
        assert_eq!(ctx.canvas.rows, 300);
    }

    // ---- text_only 跳过 ----

    #[test]
    fn text_only_skips_figure_no_flush() {
        let mut ctx = mk_ctx_gray8(100);
        let mut s = ReflowSettings::default();
        s.figure.text_only = true;
        s.figure.dst_break_pages = 1; // 非 4 → no flush
        let pixels = vec![128u8; 90 * 300]; // 同上：is_figure
        let out =
            process_region(&mut ctx, &pixels, 90, 300, PixelFormat::Gray8, 0, &s, None).unwrap();
        assert!(matches!(
            out,
            ReflowOutcome::SkippedFigure {
                flush_page_after: false
            }
        ));
        // canvas 未被写入
        assert_eq!(ctx.canvas.rows, 0);
    }

    #[test]
    fn text_only_skips_figure_with_flush_when_break_pages_4() {
        let mut ctx = mk_ctx_gray8(100);
        let mut s = ReflowSettings::default();
        s.figure.text_only = true;
        s.figure.dst_break_pages = BREAK_PAGES_AFTER_FIGURE_SKIP; // 4
        let pixels = vec![128u8; 90 * 300];
        let out =
            process_region(&mut ctx, &pixels, 90, 300, PixelFormat::Gray8, 0, &s, None).unwrap();
        assert!(matches!(
            out,
            ReflowOutcome::SkippedFigure {
                flush_page_after: true
            }
        ));
    }

    #[test]
    fn skip_triggers_page_flush_helper() {
        let mut s = ReflowSettings::default();
        assert!(!skip_triggers_page_flush(&s));
        s.figure.dst_break_pages = BREAK_PAGES_AFTER_FIGURE_SKIP;
        assert!(skip_triggers_page_flush(&s));
    }

    // ---- figure 路径：dst_negative=1 触发 invert ----

    #[test]
    fn figure_invert_applied_when_dst_negative_1() {
        let mut ctx = mk_ctx_gray8(100);
        let mut s = ReflowSettings::default();
        s.figure.dst_negative = 1;
        // pixels=64 → invert 后应该是 255-64=191
        let pixels = vec![64u8; 90 * 300];
        let out =
            process_region(&mut ctx, &pixels, 90, 300, PixelFormat::Gray8, 0, &s, None).unwrap();
        match out {
            ReflowOutcome::FigureBypassed { inverted, .. } => assert!(inverted),
            other => panic!("expected FigureBypassed, got {other:?}"),
        }
        // 验证 canvas 第一行像素为 191（invert 后）
        // canvas width=100, region width=90 → 默认左对齐
        let bmp = ctx.canvas.bmp.as_ref().unwrap();
        let row0 = bmp.row(0).unwrap();
        // 第 0-89 列应该是 191
        assert_eq!(row0[0], 191);
        assert_eq!(row0[89], 191);
        // 第 90-99 列是 fill_gap 后的左对齐右白
        assert_eq!(row0[90], 255);
    }

    // ---- figure 路径：rotate 改变 canvas blit 尺寸 ----

    #[test]
    fn figure_rotate_cw90_changes_blit_dimensions() {
        let mut ctx = mk_ctx_gray8(400); // canvas wide enough
        let mut s = ReflowSettings::default();
        s.figure.dst_figure_rotate = true;
        // viewport: portrait (vh=8 > vw=6)
        // region: 600x100 px @ 300 dpi = 2.0 x 0.33 in → landscape figure (w>h)
        // figure too wide? w=2.0 > vw=6? 不 → 不旋转
        // 调到 region 2000x100 px @ 300 dpi = 6.67 x 0.33 in → w > vw=6 ✓ → 旋转
        // 但 region 高度 0.33 < 0.55 → 不满足 figure aspect ratio (h>=0.55)
        // 改 region 300x500 px @ 300 dpi = 1.0 x 1.67 in（portrait figure 不需要旋转
        // 因为 portrait viewport + portrait figure 不匹配条件）
        // 终极方案：横向 figure + 横向超 viewport
        // 2400x300 px @ 300 dpi = 8.0 x 1.0 in。w=8 > vw=6 ✓，h=1.0 > 0.55 ✓
        // ar = w/h = 8/1 = 8 > 0.2 ✓ → is_figure
        // portrait viewport ✓, landscape figure (8>1) ✓, w > vw ✓ → Cw90 (+90)
        let w = 2400u32;
        let h = 300u32;
        // canvas 宽 400，但 region 宽 2400 → 旋转后 region 维度变 (300, 2400)
        // blit 时 src_width=300 ≤ dst_w=400 → 左对齐，rows += 2400
        let pixels = vec![64u8; (w * h) as usize];
        let out = process_region(&mut ctx, &pixels, w, h, PixelFormat::Gray8, 0, &s, None).unwrap();
        match out {
            ReflowOutcome::FigureBypassed { rotation, .. } => {
                assert_eq!(rotation, FigureRotation::Cw90);
            }
            other => panic!("expected FigureBypassed rotated, got {other:?}"),
        }
        // 旋转后 region 维度 (300, 2400)：canvas.rows 应为 2400
        assert_eq!(ctx.canvas.rows, 2400);
    }

    #[test]
    fn figure_rotate_ccw90_landscape_mode() {
        let mut ctx = mk_ctx_gray8(400);
        let mut s = ReflowSettings::default();
        s.figure.dst_figure_rotate = true;
        s.landscape = true;
        // 同上 region 2400x300 px @ 300 dpi
        let w = 2400u32;
        let h = 300u32;
        let pixels = vec![64u8; (w * h) as usize];
        let out = process_region(&mut ctx, &pixels, w, h, PixelFormat::Gray8, 0, &s, None).unwrap();
        match out {
            ReflowOutcome::FigureBypassed { rotation, .. } => {
                assert_eq!(rotation, FigureRotation::Ccw90);
            }
            other => panic!("expected FigureBypassed Ccw90, got {other:?}"),
        }
    }

    // ---- figure 路径：tall_region 用 dst_figure_justify 覆盖 ----

    #[test]
    fn tall_region_uses_figure_justify_when_set() {
        let mut ctx = mk_ctx_gray8(100);
        let mut s = ReflowSettings::default();
        s.figure.dst_figure_justify = 2; // right
                                         // region 90x300 → is_figure & tall → dst_figure_justify >= 0 覆盖
        let pixels = vec![64u8; 90 * 300];
        let out = process_region(
            &mut ctx,
            &pixels,
            90,
            300,
            PixelFormat::Gray8,
            0x88,
            &s,
            None,
        )
        .unwrap();
        match out {
            ReflowOutcome::FigureBypassed {
                applied_just_flags, ..
            } => {
                assert_eq!(
                    applied_just_flags, 2,
                    "tall + figure_justify>=0 应覆盖 region just"
                );
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn tall_region_uses_region_just_when_figure_justify_negative() {
        let mut ctx = mk_ctx_gray8(100);
        let s = ReflowSettings::default(); // dst_figure_justify=-1 默认
        let pixels = vec![64u8; 90 * 300];
        let out = process_region(
            &mut ctx,
            &pixels,
            90,
            300,
            PixelFormat::Gray8,
            0x88,
            &s,
            None,
        )
        .unwrap();
        match out {
            ReflowOutcome::FigureBypassed {
                applied_just_flags, ..
            } => {
                assert_eq!(
                    applied_just_flags, 0x88,
                    "figure_justify<0 时透传 region just"
                );
            }
            other => panic!("got {other:?}"),
        }
    }

    // ---- BitmapError 路径 ----

    #[test]
    fn figure_path_bitmap_construction_error_propagates() {
        let mut ctx = mk_ctx_gray8(100);
        let mut s = ReflowSettings::default();
        s.figure.dst_negative = 1; // 触发 invert → 需要构造 Bitmap
                                   // region 90x300 → is_figure
        let pixels = vec![64u8; 100]; // 故意短：应是 90*300=27000
        let err = process_region(&mut ctx, &pixels, 90, 300, PixelFormat::Gray8, 0, &s, None);
        match err {
            Err(ReflowError::BitmapConstruction(_)) => {} // OK
            other => panic!("expected BitmapConstruction error, got {other:?}"),
        }
    }

    // ---- 全 PixelFormat 覆盖 figure 路径 ----

    #[test]
    fn figure_path_rgb8_works() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(100, PixelFormat::Rgb8);
        let mut s = ReflowSettings::default();
        s.figure.dst_negative = 1;
        let pixels = vec![100u8; 90 * 300 * 3];
        let out =
            process_region(&mut ctx, &pixels, 90, 300, PixelFormat::Rgb8, 0, &s, None).unwrap();
        match out {
            ReflowOutcome::FigureBypassed { inverted, .. } => assert!(inverted),
            other => panic!("got {other:?}"),
        }
        // canvas Rgb8 写入 300 行
        assert_eq!(ctx.canvas.rows, 300);
    }

    // ---- 端到端 smoke：figure → flush_page 调用方流程 ----

    #[test]
    fn end_to_end_figure_then_flush_simulated() {
        let mut ctx = mk_ctx_gray8(100);
        let s = ReflowSettings::default();
        let pixels = vec![64u8; 90 * 300];
        let out1 =
            process_region(&mut ctx, &pixels, 90, 300, PixelFormat::Gray8, 0, &s, None).unwrap();
        assert!(matches!(out1, ReflowOutcome::FigureBypassed { .. }));
        assert_eq!(ctx.canvas.rows, 300);

        // 再加一张 figure
        let out2 =
            process_region(&mut ctx, &pixels, 90, 300, PixelFormat::Gray8, 0, &s, None).unwrap();
        assert!(matches!(out2, ReflowOutcome::FigureBypassed { .. }));
        assert_eq!(ctx.canvas.rows, 600);
    }
}
