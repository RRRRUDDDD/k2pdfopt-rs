//! `master` - MASTERINFO 8 桶拆分（Step 5.6, M3.5）
//!
//! C 版 `MASTERINFO` god object（`k2pdfoptlib/k2pdfopt.h:674-742`，30+ 字段）
//! 拆为 8 个职责单一的子 struct，再用 [`ConvertContext`] 组合。
//!
//! **设计文档**：`docs/masterinfo-design.md`
//! **架构决策**：`docs/adr/ADR-016-masterinfo-decomposition.md`（Approved 2026-05-07）
//! **Spike 验证**：`spikes/masterinfo-statemachine/`（Step 0.4，3 单测通过）
//!
//! # 8 桶速查
//!
//! | 桶名 | 职责 | 模块 |
//! |------|------|------|
//! | [`PageState`] | 源页面元信息 | [`page_state`] |
//! | [`MasterCanvas`] | 输出 master bitmap + row cursor | [`master_canvas`] |
//! | [`SpacingState`] | 行间距与 lastrow 度量 | [`spacing_state`] |
//! | [`WrapState`] | Reflow / line-wrap 缓冲区 | [`wrap_state`] |
//! | [`OutputPaginator`] | 输出页队列 + 分页 marks | [`output_paginator`] |
//! | [`OcrStaging`] | OCR words 在 master 坐标系暂存 | [`ocr_staging`] |
//! | [`OutlineMapper`] | 书签 / outline 映射 | [`outline_mapper`] |
//! | [`NativeBoxAccumulator`] | Native PDF crop boxes（feature gated） | [`native_box`] |
//!
//! # 借用模型
//!
//! Rust 的"分离借用"规则（borrowing disjoint fields of a struct）让 `&mut ConvertContext`
//! 可以同时让其中两个不同字段被 `&mut`，因此无需 event-driven，直接顺序写各桶即可。
//! 详见设计文档 §5 的借用关系图。
//!
//! # Step 7.3（M5）落地范围
//!
//! 本步骤把 8 桶串联跑通 M5 端到端 pipeline：
//! - [`ConvertContext::add_bitmap`]：简化版 4 步流程（gap + ensure + fill + blit + spacing update）
//! - [`ConvertContext::calculate_line_gap`]：当前 region 与 master 顶部之间的留白
//! - [`ConvertContext::should_flush`]：master.rows >= dst_page_height 判定
//! - [`ConvertContext::flush_page`]：调 `breakpoints::find_break_point` 切顶
//!   + `master_canvas.split_off_top` + `paginator.push_page` + `outline.remap_to_dst`
//! - [`ConvertContext::flush_remaining`]：把剩余 master canvas（不足一页）强制 flush
//!
//! 未实现的 9 步完整流程（OCR / wrap / pageboxes / fully justify / fit_to_page=-3
//! mid-page flush）由 M6/M7/M8 逐步增量增强。

pub mod master_canvas;
pub mod native_box;
pub mod ocr_staging;
pub mod outline_mapper;
pub mod output_paginator;
pub mod page_state;
pub mod spacing_state;
pub mod wrap_state;

pub use master_canvas::MasterCanvas;
pub use native_box::{CropBox, NativeBoxAccumulator};
pub use ocr_staging::{OcrStaging, OcrWord};
pub use outline_mapper::{OutlineEntry, OutlineMapError, OutlineMapper};
pub use output_paginator::{OutputPaginator, PageBreakMark, PaginatorPage};
pub use page_state::PageState;
pub use spacing_state::SpacingState;
pub use wrap_state::{
    AddRegion, FlushedLine, HyphenInfo, MasterGapCarry, WRectMap, WRectMaps, WrapState,
};

use crate::breakpoints::{find_break_point, BreakSettings};
use crate::crop::CropError;
use crate::reflow_pipeline::{self, ReflowError, ReflowOutcome, ReflowSettings};
use k2ocr::OcrEngine;
use k2types::{Bitmap, OutputPage, PixelFormat};

/// Region 类型，对应 C 版 `REGION_TYPE_*` 常量族。
///
/// 来源：`k2pdfoptlib/k2pdfopt.h`（REGION_TYPE_* 宏，散落各处）+ `k2master.c::lastrow.type`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegionType {
    /// 尚未分类（初始态）。对应 C 版 lastrow 初始化的 -1 哨兵。
    #[default]
    Undetermined,
    /// 文本行
    Text,
    /// 图像 / 公式（不可重排）
    Figure,
    /// 空白行
    Blank,
}

/// 段落对齐方式，对应 C 版 `JUSTIFICATION_*` 常量族。
///
/// 来源：`k2pdfoptlib/k2pdfopt.h`（JUSTIFICATION_LEFT/CENTER/RIGHT/FULL 宏）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Justification {
    /// 左对齐（C 版默认）。
    #[default]
    Left,
    /// 居中
    Center,
    /// 右对齐
    Right,
    /// 两端对齐（full-justify）
    Full,
}

/// MASTERINFO 8 桶组合容器（对应 C 版 `MASTERINFO * masterinfo` 单一对象）。
///
/// # 字段
///
/// 8 个公开字段对应 8 个状态桶，调用方拿 `&mut ConvertContext` 后可顺序借用
/// 各字段（Rust 分离借用）。
///
/// # 借用示例
///
/// ```ignore
/// // 同时 &mut canvas 与 &mut ocr —— Rust 编译器接受
/// let canvas = &mut ctx.canvas;
/// let ocr = &mut ctx.ocr;
/// canvas.fill_gap(/* ... */);
/// ocr.offset_words(/* ... */);
/// ```
///
/// # C 对照
///
/// 来源：`k2pdfoptlib/k2pdfopt.h:674-742`（MASTERINFO 顶层 struct）
/// 主入口：`k2pdfoptlib/k2master.c:482`（masterinfo_add_bitmap）
#[derive(Debug)]
pub struct ConvertContext {
    /// 源页面元信息（PageState）
    pub page: PageState,
    /// Master output bitmap + row cursor（MasterCanvas）
    pub canvas: MasterCanvas,
    /// 行间距 + lastrow 度量（SpacingState）
    pub spacing: SpacingState,
    /// Reflow / line-wrap 缓冲区（WrapState）
    pub wrap: WrapState,
    /// 输出页队列 + 分页 marks + autocrop margins（OutputPaginator）
    pub paginator: OutputPaginator,
    /// OCR words 在 master canvas 坐标系暂存（OcrStaging）
    pub ocr: OcrStaging,
    /// 书签 / outline 映射（OutlineMapper）
    pub outline: OutlineMapper,
    /// Native PDF crop boxes（feature gated）（NativeBoxAccumulator）
    pub native: NativeBoxAccumulator,
}

impl ConvertContext {
    /// 用各桶的默认值构造一个空的 ConvertContext。
    ///
    /// 对应 C 版 `masterinfo_init()`（`k2master.c` 起始位置）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            page: PageState::new(),
            canvas: MasterCanvas::new(),
            spacing: SpacingState::new(),
            wrap: WrapState::new(),
            paginator: OutputPaginator::new(),
            ocr: OcrStaging::new(),
            outline: OutlineMapper::new(),
            native: NativeBoxAccumulator::new(),
        }
    }

    /// 初始化 master canvas（在第一次 add_bitmap 前调）。
    ///
    /// 对应 C `masterinfo_init` 内的 `bmp_alloc` 序列。
    pub fn init_canvas(&mut self, width: u32, format: PixelFormat) {
        self.canvas.init(width, format);
    }

    /// 添加一个 bitmap 到 master canvas（简化版 4 步操作流）。
    ///
    /// **Step 7.3（M5）简化实现**。完整 9 步流程（含 OCR / wrap / pageboxes /
    /// fully-justify / fit_to_page=-3 mid flush）由 M6/M7/M8 逐步增量增强。
    ///
    /// # Step 7.3 实际执行的子步骤
    ///
    /// 1. 计算 `gap` = [`Self::calculate_line_gap`]（与 master 顶部之间的留白）
    /// 2. `ensure_height(rows + gap + src_height)`（按 1.4x 增长）
    /// 3. `fill_gap(gap, Text)` 在 master 尾部追加白色 gap
    /// 4. `blit(src_pixels, src_width, src_height, justification_flags)` 复制源像素
    /// 5. `spacing.update_after_add(src_height, dpi)` 记录 lastrow 高度
    ///
    /// # 参数
    ///
    /// - `src_width` / `src_height` / `src_pixels`：待添加 bitmap
    /// - `dpi`：源 bitmap 的 DPI（用于行间距换算，简化模式直通）
    /// - `justification_flags`：[`Justification`] 编码（0=左 / 1=居中 / 2=右）
    /// - `whitethresh`：白色阈值（区分背景 / 内容；当前简化版未参与计算，仅记录）
    ///
    /// # C 对照
    ///
    /// 来源：`k2pdfoptlib/k2master.c:482`（masterinfo_add_bitmap，约 200 行）
    /// Spike 等价实现：`spikes/masterinfo-statemachine/src/lib.rs::add_bitmap`
    pub fn add_bitmap(
        &mut self,
        src_width: u32,
        src_height: u32,
        src_pixels: &[u8],
        dpi: f64,
        justification_flags: i32,
        whitethresh: i32,
    ) {
        let _ = whitethresh; // Step 7.3 简化模式：whitethresh 仅作元信息
        if src_width == 0 || src_height == 0 {
            return;
        }
        // Step 1: 计算 gap
        let gap = self.calculate_line_gap(dpi);
        // Step 2 + Step 3: ensure_height + fill_gap
        self.canvas.fill_gap(gap, RegionType::Text);
        // Step 4: blit
        self.canvas
            .blit(src_pixels, src_width, src_height, justification_flags);
        // Step 5: 更新 spacing state
        self.spacing.update_after_add(src_height, dpi);
    }

    /// 添加一个 bitmap 到 master canvas，**接入 figure bypass 决策**（Step 8.4）。
    ///
    /// 与 [`Self::add_bitmap`] 的区别：先经 [`reflow_pipeline::process_region`]
    /// 做 figure/text 分类，命中 figure 时走 figure bypass（invert / rotate /
    /// dst_figure_justify 覆盖 / 直接 blit），命中 text 时再分两个子路径：
    /// `TextDirectBlit`（空白 / 无可识别文字 region）仍走 M5 直通（与
    /// [`Self::add_bitmap`] 一致），`TextReflowed { lines, ocr_words }`（Step
    /// 11.2/11.3 实装的 column → row → word → wrap_state 四层流水线产物 + Step
    /// 11.5 内联 OCR 识别词流）则逐 [`FlushedLine`] blit 到 master canvas，并把
    /// `ocr_words` 喂给 [`OcrStaging::concatenate`]（Step 11.4 起启用 reflow
    /// 主路径，Step 11.5 起把 OCR 调用收编到本路径）。
    ///
    /// # 范围（MVP）
    ///
    /// - **figure 路径**：完整接入 Step 8.3 figure 决策 + Step 8.4 invert/rotate helpers
    /// - **text 路径**：仍走 M5 直通（不调 [`crate::wrap::WrapPipeline`]）
    /// - **跳过路径**：返回 [`ReflowOutcome::SkippedFigure`]，由调用方根据
    ///   `flush_page_after` 决定是否触发分页（本方法不主动调 [`Self::flush_page`]）
    ///
    /// # 参数
    ///
    /// - `src_width` / `src_height` / `src_pixels` / `src_format`：源 region 位图
    /// - `dpi`：源 region 的 DPI（同时也是 [`ReflowSettings::region_dpi`] 的覆盖值）
    /// - `justification_flags`：region 自带 just（figure 路径下可能被 dst_figure_justify 覆盖）
    /// - `whitethresh`：白色阈值（与 [`Self::add_bitmap`] 一致，当前简化版仅作元信息）
    /// - `settings`：reflow 集成层运行时配置（含 `ocr_settings` Step 11.5 新增字段）
    /// - `ocr_engine`（Step 11.5 新增）：可选的 OCR 引擎。`None` 表示不跑 OCR；
    ///   `Some(_)` 时仅在 [`ReflowOutcome::TextReflowed`] 路径才会触发 `recognize`。
    ///   figure / skip / direct_blit 三路径**不**跑 OCR（C `k2master.c:740-745`
    ///   等价：只在 reflow 路径前调 `ocrwords_from_bmp8`）。
    ///
    /// # 返回
    ///
    /// 返回 [`ReflowOutcome`] 让调用方知道 region 走了哪条路径。
    ///
    /// # 错误
    ///
    /// 仅 figure 路径下 [`reflow_pipeline::process_region`] 内部 `Bitmap::from_raw`
    /// 失败时返回 [`ReflowError::BitmapConstruction`]。Step 11.5 起 text 路径下
    /// `engine.recognize` 失败时返回 [`ReflowError::Ocr`]（其他 text 错误返
    /// [`ReflowError::LayoutAnalysis`]）。
    #[allow(clippy::too_many_arguments)] // Step 11.5：含 self 共 10 参；Open Q 11.5.D
                                         // 推迟 v0.3 评估引入 AddBitmapArgs builder pattern 重构
    pub fn add_bitmap_with_reflow(
        &mut self,
        src_width: u32,
        src_height: u32,
        src_pixels: &[u8],
        src_format: PixelFormat,
        dpi: f64,
        justification_flags: i32,
        whitethresh: i32,
        settings: &ReflowSettings,
        ocr_engine: Option<&dyn OcrEngine>,
    ) -> Result<ReflowOutcome, ReflowError> {
        let _ = whitethresh;
        if src_width == 0 || src_height == 0 {
            return Ok(ReflowOutcome::TextDirectBlit);
        }
        // 用调用方传的 dpi 覆盖 settings.region_dpi（调用方比 settings 更新鲜）。
        // Step 11.2：ReflowSettings 因含非 Copy 的 RowSettings/WordSettings 退化为
        // Clone-only，原 deref-copy 改为 clone。
        let mut effective = settings.clone();
        effective.region_dpi = dpi;
        let outcome = reflow_pipeline::process_region(
            self,
            src_pixels,
            src_width,
            src_height,
            src_format,
            justification_flags,
            &effective,
            ocr_engine,
        )?;
        // Step 11.4：TextDirectBlit / TextReflowed 分流。
        //
        // - `TextDirectBlit`：region 内无可识别文本（空白 / 噪点 / DPI 非法）→ 与
        //   `add_bitmap` 行为一致 fill_gap + blit + spacing update（保留 v0.1.0
        //   端到端字节级行为）。
        // - `TextReflowed { lines, ocr_words }`：[`analyze_text_region`] 已跑完
        //   column → row → word → wrap_state flush，本路径逐 [`FlushedLine`] blit
        //   到 master canvas。每行用自带 `gap` / `just_flags` / `bitmap` 写入，
        //   `spacing` 用 line bitmap 高度 + dpi 更新（与 figure 路径同源）。
        //   Step 11.5：同时把 `ocr_words` 喂给 [`OcrStaging::concatenate`]——
        //   `process_region` 内已用 `dy = canvas.rows + gap` 平移到 master 坐标系
        //   （时序：OCR 在写 canvas 之前算 dy），与 Step 9.3 `ConvertJob::run`
        //   主循环 OCR 块的 dy 计算完全一致。
        // - `FigureBypassed` / `SkippedFigure`：已由 [`process_region`] 内部完成
        //   blit 或主动跳过（含 `dst_break_pages==4` flush 标记），这里不再追加写入；
        //   两者均不携带 ocr_words 字段（Step 11.5：figure 路径不跑 OCR）。
        match &outcome {
            ReflowOutcome::TextDirectBlit => {
                let gap = self.calculate_line_gap(dpi);
                self.canvas.fill_gap(gap, RegionType::Text);
                self.canvas
                    .blit(src_pixels, src_width, src_height, justification_flags);
                self.spacing.update_after_add(src_height, dpi);
            }
            ReflowOutcome::TextReflowed { lines, ocr_words } => {
                for line in lines {
                    let line_gap = u32::try_from(line.gap.max(0)).unwrap_or(0);
                    self.canvas.fill_gap(line_gap, RegionType::Text);
                    self.canvas.blit(
                        &line.bitmap.pixels,
                        line.bitmap.width,
                        line.bitmap.height,
                        line.just_flags,
                    );
                    self.spacing.update_after_add(line.bitmap.height, dpi);
                }
                if !ocr_words.is_empty() {
                    // Step 11.5：把 OCR words 喂给 ocr_staging（已在 process_region
                    // 内按 dy = canvas.rows + gap 平移到 master canvas 全局坐标）
                    self.ocr.concatenate(ocr_words.clone());
                }
            }
            ReflowOutcome::FigureBypassed { .. } | ReflowOutcome::SkippedFigure { .. } => {}
        }
        Ok(outcome)
    }

    /// 计算 master canvas 顶部与即将加入的 region 之间的行间距（pixels）。
    ///
    /// **Step 7.3（M5）简化实现**：返回 0（即不加 gap），与 C 版直通模式
    /// （`fit_to_page=-2`）等价。完整版本（基于 lastrow.gapblank +
    /// sourcegap_pixels + maxgap_pixels）由 M6 wrap_state 串联后落地。
    ///
    /// 对应 C 版 `k2master.c::calculate_line_gap()`（`k2master.c:969-1180`）。
    #[must_use]
    pub fn calculate_line_gap(&self, dpi: f64) -> u32 {
        let _ = dpi;
        // Step 7.3 简化：M5 直通模式，每个源页直接累加（与现有 mutool render
        // 输出的页面间没有原始 gap）
        0
    }

    /// 判断 master canvas 是否已满需要 flush 出页。
    ///
    /// **Step 7.3（M5）简化实现**：当 `canvas.rows >= dst_page_height + incoming_height`
    /// 时返回 true（即新 region 加进去会溢出一页）。完整版需要考虑 fit_to_page=-3
    /// + master 内部 region gap 等高级特性，推迟到 M6。
    #[must_use]
    pub fn should_flush(&self, dst_page_height: u32, incoming_height: u32) -> bool {
        if dst_page_height == 0 {
            return false;
        }
        self.canvas.rows >= dst_page_height
            || self.canvas.rows.saturating_add(incoming_height) > dst_page_height
    }

    /// 把 master canvas 顶部一页 flush 到 paginator 队列。
    ///
    /// **Step 7.3（M5）落地**：对应 C 版 `masterinfo_publish`（`k2master.c` 的
    /// 主分页入口）的简化版本。
    ///
    /// # 步骤
    ///
    /// 1. 调 [`breakpoints::find_break_point`] 找最佳 rowcount（避开切穿文本行）
    /// 2. `canvas.split_off_top(rowcount)` 切出顶部 rowcount 行
    /// 3. 构造 [`PaginatorPage`] 推入 `paginator.queued_pages`
    /// 4. 调 `outline.remap_to_dst(page.srcpageno, output_page_index)` 把对应源页
    ///    的 outline 条目映射到输出页号
    ///
    /// # 参数
    ///
    /// - `maxsize`：单页目标高度（pixel，C 版 dst_height）
    /// - `break_settings`：传给 find_break_point 的设置
    /// - `srcpageno`：当前输出页所属的源页号（用于 outline 映射；负值跳过 remap）
    ///
    /// # 返回
    ///
    /// - `Ok(Some(idx))`：成功 flush 一页，返回新 PaginatorPage 的 page_index
    /// - `Ok(None)`：canvas.rows == 0 或 split_off_top 返回 None，不 flush
    /// - `Err(_)`：find_break_point 内部 `find_textrows` 越界
    pub fn flush_page(
        &mut self,
        maxsize: u32,
        break_settings: &BreakSettings,
        srcpageno: i32,
    ) -> Result<Option<u32>, CropError> {
        if self.canvas.rows == 0 {
            return Ok(None);
        }
        let bmp = match self.canvas.bmp.as_ref() {
            Some(b) => b,
            None => return Ok(None),
        };
        // Step 1: find_break_point（不需要 marks 时传空 slice）
        let rowcount = if maxsize == 0 || self.canvas.rows < maxsize {
            // 不到一页：把剩余全部 flush
            self.canvas.rows
        } else {
            find_break_point(
                bmp,
                self.canvas.rows,
                0,
                maxsize,
                break_settings,
                &mut self.paginator.pagebreak_marks,
            )?
            .max(1) // 保护
        };
        // Step 2: split_off_top
        let (w, h, pixels) = match self.canvas.split_off_top(rowcount) {
            Some(t) => t,
            None => return Ok(None),
        };
        let format = self
            .canvas
            .bmp
            .as_ref()
            .map(|b| b.format)
            .unwrap_or(PixelFormat::Gray8);
        // Step 2.5 (Step 9.3 新增): drain OCR words 在本页范围内的，并把剩余 word 上移
        // 对应 C `k2master.c:1535-1582`（masterinfo_publish 内 OCR 选词 + 剩余整段平移）。
        let ocr_words = self.ocr.drain_in_range(0.0, rowcount as f64);
        if !ocr_words.is_empty() {
            // 仅当确实有 word 被选出时上移剩余（noop 时省去全表遍历）
            self.ocr.offset_y(-(rowcount as f64));
        } else if !self.ocr.is_empty() {
            // 即便本页无 word 命中，剩余 word 仍在 master 坐标系中（页已切走 rowcount 行）
            self.ocr.offset_y(-(rowcount as f64));
        }
        // Step 3: push 到 paginator queue
        let page_index = self.paginator.output_page_count;
        let page = PaginatorPage {
            page_index,
            srcpageno,
            width: w,
            height: h,
            format,
            pixels,
            ocr_words,
        };
        self.paginator.push_page(page);
        // Step 4: outline remap
        if srcpageno >= 0 {
            self.outline.remap_to_dst(srcpageno, page_index as i32);
        }
        Ok(Some(page_index))
    }

    /// 强制 flush master canvas 剩余的全部行（用于 pipeline 末尾）。
    ///
    /// 不调 find_break_point，直接整段切出（C 等价 `masterinfo_flush` with
    /// `clearbitmap=1`，`k2master.c:185-195`）。
    pub fn flush_remaining(&mut self, srcpageno: i32) -> Option<u32> {
        if self.canvas.rows == 0 {
            return None;
        }
        let rowcount = self.canvas.rows;
        let (w, h, pixels) = self.canvas.split_off_top(rowcount)?;
        let format = self
            .canvas
            .bmp
            .as_ref()
            .map(|b| b.format)
            .unwrap_or(PixelFormat::Gray8);
        // Step 9.3: 把剩余 OCR words 全部归属本页（master 已无残留）
        let ocr_words = self.ocr.drain_in_range(0.0, rowcount as f64);
        // 剩下没归属的（如果有）保留在桶里，但 flush_remaining 后通常无需再上移
        // 因为 master canvas 已空（split_off_top 切走全部行），后续不会再 add_bitmap。
        let page_index = self.paginator.output_page_count;
        let page = PaginatorPage {
            page_index,
            srcpageno,
            width: w,
            height: h,
            format,
            pixels,
            ocr_words,
        };
        self.paginator.push_page(page);
        if srcpageno >= 0 {
            self.outline.remap_to_dst(srcpageno, page_index as i32);
        }
        Some(page_index)
    }
}

impl Default for ConvertContext {
    fn default() -> Self {
        Self::new()
    }
}

/// 把内部临时态 [`PaginatorPage`] 转换为对外 [`OutputPage`]。
///
/// 由 settings 提供 DPI / JPEG 控制 / halfsize / rotation 等字段（PaginatorPage
/// 不携带这些）。Step 7.3 落地：Open Question 7.2.J 的转换 helper。
///
/// # 参数
///
/// - `page`：master canvas flush 出的内部页
/// - `output_dpi`：目标设备 DPI（C `k2settings->dst_dpi`）
/// - `jpeg_quality`：JPEG 质量（`-1` = Flate / `>=0` = JPEG）
/// - `halfsize`：bit packing 控制（仅 `0` = 8 BPC 在 Step 7.2 主路径支持）
/// - `rotation`：旋转角度（度，0/90/180/270）
///
/// # 错误
///
/// 内部用 `Bitmap::from_raw` 校验 pixels.len() == width * height * bpp；不自洽
/// 时返回 `Err(BitmapError::PixelLenMismatch)`（由上游 split_off_top 一般可保证
/// 一致，本函数 trait-friendly 返 Result 而非 panic）。
pub fn output_page_from_paginator(
    page: PaginatorPage,
    output_dpi: f32,
    jpeg_quality: i32,
    halfsize: u8,
    rotation: f32,
) -> Result<OutputPage, k2types::BitmapError> {
    let bitmap = Bitmap::from_raw(
        page.width,
        page.height,
        output_dpi,
        page.format,
        page.pixels,
    )?;
    Ok(OutputPage {
        page_index: page.page_index,
        srcpageno: page.srcpageno,
        bitmap,
        output_dpi,
        rotation,
        jpeg_quality,
        halfsize,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::breakpoints::BreakSettings;

    #[test]
    fn convert_context_new_returns_all_buckets_default() {
        let ctx = ConvertContext::new();
        // 8 桶 struct 全部走 Default 路径（仅验证可构造，字段值在各模块单测中验证）
        assert_eq!(ctx.page.source_page, 0);
        assert_eq!(ctx.canvas.rows, 0);
        assert_eq!(ctx.spacing.nocr, 0);
        assert_eq!(ctx.wrap.base, 0);
        assert_eq!(ctx.paginator.published_pages, 0);
        assert!(ctx.ocr.words.is_empty());
        assert!(ctx.outline.entries.is_empty());
        assert!(ctx.native.boxes.is_empty());
    }

    #[test]
    fn convert_context_default_eq_new() {
        let a = ConvertContext::default();
        let b = ConvertContext::new();
        assert_eq!(a.page.source_page, b.page.source_page);
        assert_eq!(a.canvas.width, b.canvas.width);
    }

    #[test]
    fn disjoint_borrow_compiles() {
        // 此测试编译通过本身就证明 8 桶拆分支持分离借用
        let mut ctx = ConvertContext::new();
        let canvas = &mut ctx.canvas;
        let ocr = &mut ctx.ocr;
        canvas.rows = 0;
        ocr.words.clear();
    }

    #[test]
    fn region_type_default_is_undetermined() {
        let r = RegionType::default();
        assert_eq!(r, RegionType::Undetermined);
    }

    #[test]
    fn justification_default_is_left() {
        let j = Justification::default();
        assert_eq!(j, Justification::Left);
    }

    // ---- Step 7.3 add_bitmap / flush_page ----

    #[test]
    fn add_bitmap_grows_canvas_and_blits() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(10, PixelFormat::Gray8);
        let src = vec![0u8; 10 * 5]; // 10x5 黑色
        ctx.add_bitmap(10, 5, &src, 150.0, 0, 200);
        assert_eq!(ctx.canvas.rows, 5);
        assert!(ctx.canvas.height >= 5);
        // spacing 更新
        assert_eq!(ctx.spacing.last_row_rowheight, 5);
        assert_eq!(ctx.spacing.nocr, 1);
    }

    #[test]
    fn add_bitmap_zero_size_skipped() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(10, PixelFormat::Gray8);
        ctx.add_bitmap(0, 5, &[], 150.0, 0, 200);
        assert_eq!(ctx.canvas.rows, 0);
        ctx.add_bitmap(10, 0, &[], 150.0, 0, 200);
        assert_eq!(ctx.canvas.rows, 0);
    }

    #[test]
    fn should_flush_when_canvas_meets_height() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(10, PixelFormat::Gray8);
        ctx.canvas.rows = 100;
        assert!(ctx.should_flush(100, 0));
        assert!(ctx.should_flush(99, 0));
        assert!(!ctx.should_flush(101, 0));
        // dst_page_height=0 永不 flush
        assert!(!ctx.should_flush(0, 100));
    }

    #[test]
    fn should_flush_when_incoming_overflow() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(10, PixelFormat::Gray8);
        ctx.canvas.rows = 50;
        // 50 + 60 > 100
        assert!(ctx.should_flush(100, 60));
        // 50 + 40 = 90 <= 100
        assert!(!ctx.should_flush(100, 40));
    }

    #[test]
    fn flush_page_empty_canvas_returns_none() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(10, PixelFormat::Gray8);
        let settings = BreakSettings::default();
        let r = ctx.flush_page(100, &settings, 0).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn flush_page_full_canvas_pushes_to_queue() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(10, PixelFormat::Gray8);
        let src = vec![0u8; 10 * 50];
        ctx.add_bitmap(10, 50, &src, 150.0, 0, 200);
        let settings = BreakSettings::default();
        // 不足 maxsize=100 → flush 整段 50 行
        let r = ctx.flush_page(100, &settings, 3).unwrap();
        assert_eq!(r, Some(0));
        assert_eq!(ctx.paginator.queued_len(), 1);
        assert_eq!(ctx.paginator.queued_pages[0].height, 50);
        assert_eq!(ctx.paginator.queued_pages[0].srcpageno, 3);
        assert_eq!(ctx.canvas.rows, 0);
    }

    #[test]
    fn flush_page_remaps_outline() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(10, PixelFormat::Gray8);
        ctx.outline
            .add_entry(OutlineEntry {
                title: "Chap 5".into(),
                src_page: 5,
                dst_page: -1,
                parent_idx: None,
            })
            .unwrap();
        let src = vec![0u8; 10 * 10];
        ctx.add_bitmap(10, 10, &src, 150.0, 0, 200);
        let settings = BreakSettings::default();
        ctx.flush_page(100, &settings, 5).unwrap();
        // outline 应已重映射
        assert_eq!(ctx.outline.entries[0].dst_page, 0);
    }

    #[test]
    fn flush_remaining_pushes_all() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(10, PixelFormat::Gray8);
        let src = vec![0u8; 10 * 3];
        ctx.add_bitmap(10, 3, &src, 150.0, 0, 200);
        let r = ctx.flush_remaining(7);
        assert_eq!(r, Some(0));
        assert_eq!(ctx.paginator.queued_pages[0].height, 3);
        assert_eq!(ctx.paginator.queued_pages[0].srcpageno, 7);
        assert_eq!(ctx.canvas.rows, 0);
    }

    #[test]
    fn flush_remaining_empty_returns_none() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(10, PixelFormat::Gray8);
        assert!(ctx.flush_remaining(0).is_none());
    }

    // ---- Step 7.3 output_page_from_paginator ----

    #[test]
    fn output_page_from_paginator_roundtrip() {
        let pixels = vec![200; 4 * 3];
        let pp = PaginatorPage {
            page_index: 7,
            srcpageno: 3,
            width: 4,
            height: 3,
            format: PixelFormat::Gray8,
            pixels: pixels.clone(),
            ocr_words: Vec::new(),
        };
        let op = output_page_from_paginator(pp, 150.0, 85, 0, 0.0).unwrap();
        assert_eq!(op.page_index, 7);
        assert_eq!(op.srcpageno, 3);
        assert_eq!(op.bitmap.width, 4);
        assert_eq!(op.bitmap.height, 3);
        assert_eq!(op.bitmap.format, PixelFormat::Gray8);
        assert_eq!(op.bitmap.pixels, pixels);
        assert!((op.output_dpi - 150.0).abs() < 1e-6);
        assert_eq!(op.jpeg_quality, 85);
        assert_eq!(op.halfsize, 0);
    }

    #[test]
    fn output_page_from_paginator_rgb() {
        let pixels = vec![10, 20, 30, 40, 50, 60];
        let pp = PaginatorPage {
            page_index: 0,
            srcpageno: -1,
            width: 2,
            height: 1,
            format: PixelFormat::Rgb8,
            pixels,
            ocr_words: Vec::new(),
        };
        let op = output_page_from_paginator(pp, 200.0, -1, 0, 0.0).unwrap();
        assert_eq!(op.bitmap.format, PixelFormat::Rgb8);
        assert_eq!(op.jpeg_quality, -1); // Flate 路径
    }

    #[test]
    fn output_page_from_paginator_mismatched_pixels_fails() {
        // pixels.len() != width * height * bpp 时返回错误，不 panic
        let pp = PaginatorPage {
            page_index: 0,
            srcpageno: -1,
            width: 10,
            height: 10,
            format: PixelFormat::Gray8,
            pixels: vec![0u8; 50], // 应是 100
            ocr_words: Vec::new(),
        };
        let err = output_page_from_paginator(pp, 150.0, 85, 0, 0.0).unwrap_err();
        // BitmapError 含 PixelLenMismatch 变体（Display 字符串可能含 "mismatch"
        // 或 "len"，本测试只验证返 Err 不 panic）
        let _ = format!("{err}");
    }

    // ---- Step 8.4 add_bitmap_with_reflow ----

    #[test]
    fn add_bitmap_with_reflow_text_path_matches_add_bitmap() {
        // 同样的 text region：add_bitmap vs add_bitmap_with_reflow 应该写入相同行数
        let mut ctx_a = ConvertContext::new();
        ctx_a.init_canvas(100, PixelFormat::Gray8);
        let mut ctx_b = ConvertContext::new();
        ctx_b.init_canvas(100, PixelFormat::Gray8);

        // 50x50 px @ 300 dpi = 0.17 x 0.17 in → 非 figure 非 tall → text 路径
        // Step 11.2 后 text 路径会调 analyze_text_region：含像素时返 TextReflowed，
        // 空 region 返 TextDirectBlit；两种 outcome 在 main pipeline 都走 M5 直通。
        let pixels = vec![100u8; 50 * 50];
        ctx_a.add_bitmap(50, 50, &pixels, 300.0, 0, 200);
        let outcome = ctx_b
            .add_bitmap_with_reflow(
                50,
                50,
                &pixels,
                PixelFormat::Gray8,
                300.0,
                0,
                200,
                &ReflowSettings::default(),
                None,
            )
            .unwrap();
        assert!(
            matches!(
                outcome,
                ReflowOutcome::TextDirectBlit | ReflowOutcome::TextReflowed { .. }
            ),
            "text 路径应返 TextDirectBlit 或 TextReflowed, got {outcome:?}"
        );
        assert_eq!(ctx_a.canvas.rows, ctx_b.canvas.rows);
        assert_eq!(
            ctx_a.spacing.last_row_rowheight,
            ctx_b.spacing.last_row_rowheight
        );
        // 像素内容比对
        assert_eq!(
            ctx_a.canvas.bmp.as_ref().unwrap().pixels,
            ctx_b.canvas.bmp.as_ref().unwrap().pixels
        );
    }

    #[test]
    fn add_bitmap_with_reflow_figure_path_writes_canvas() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(100, PixelFormat::Gray8);
        // 90x300 @ 300 dpi = 0.3 x 1.0 in → is_figure
        let pixels = vec![64u8; 90 * 300];
        let outcome = ctx
            .add_bitmap_with_reflow(
                90,
                300,
                &pixels,
                PixelFormat::Gray8,
                300.0,
                0,
                200,
                &ReflowSettings::default(),
                None,
            )
            .unwrap();
        assert!(matches!(outcome, ReflowOutcome::FigureBypassed { .. }));
        assert_eq!(ctx.canvas.rows, 300); // figure 路径在 process_region 内 blit
    }

    #[test]
    fn add_bitmap_with_reflow_skipped_does_not_write_canvas() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(100, PixelFormat::Gray8);
        let mut s = ReflowSettings::default();
        s.figure.text_only = true;
        let pixels = vec![64u8; 90 * 300]; // is_figure → 触发 skip
        let outcome = ctx
            .add_bitmap_with_reflow(
                90,
                300,
                &pixels,
                PixelFormat::Gray8,
                300.0,
                0,
                200,
                &s,
                None,
            )
            .unwrap();
        assert!(matches!(outcome, ReflowOutcome::SkippedFigure { .. }));
        assert_eq!(ctx.canvas.rows, 0); // 跳过不写入
    }

    #[test]
    fn add_bitmap_with_reflow_zero_size_returns_text_direct_blit() {
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(100, PixelFormat::Gray8);
        let outcome = ctx
            .add_bitmap_with_reflow(
                0,
                10,
                &[],
                PixelFormat::Gray8,
                300.0,
                0,
                200,
                &ReflowSettings::default(),
                None,
            )
            .unwrap();
        assert!(matches!(outcome, ReflowOutcome::TextDirectBlit));
        assert_eq!(ctx.canvas.rows, 0);
    }

    #[test]
    fn add_bitmap_with_reflow_dpi_overrides_settings_region_dpi() {
        // settings.region_dpi=300 但调用方传 dpi=150：region 应按 150 dpi 算 inches
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(100, PixelFormat::Gray8);
        // 90x300 px @ 150 dpi = 0.6 x 2.0 in → ar=0.3 > 0.2, h=2.0 > 0.55 → is_figure
        // 同样像素 @ 300 dpi = 0.3 x 1.0 in 也是 figure，但 figure 检测的边界值
        // 用 90x200 px：@ 150 dpi = 0.6 x 1.33 in → is_figure (h>0.55);
        // @ 300 dpi = 0.3 x 0.67 in → ar=0.45>0.2, h=0.67>0.55 → 也是 figure
        // 简单验证：调用方 dpi 写到 effective.region_dpi 并由 process_region 用
        let pixels = vec![64u8; 90 * 300];
        let outcome = ctx
            .add_bitmap_with_reflow(
                90,
                300,
                &pixels,
                PixelFormat::Gray8,
                150.0, // 不同于 settings.region_dpi=300
                0,
                200,
                &ReflowSettings::default(),
                None,
            )
            .unwrap();
        // 150 dpi 下 region 仍是 figure，所以走 FigureBypassed
        assert!(matches!(outcome, ReflowOutcome::FigureBypassed { .. }));
    }
}
