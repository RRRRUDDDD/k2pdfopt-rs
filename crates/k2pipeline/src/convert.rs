//! `convert` —— ConvertJob 编排（Step 7.3 M5 端到端最简化版本）。
//!
//! 把 [`k2render::MutoolRenderer`] → [`k2layout::ConvertContext`] →
//! [`k2pdfout::LopdfWriter`] 串成一条 pipeline，把源 PDF 渲染并写回目标 PDF。
//!
//! # 简化策略（Step 7.3）
//!
//! - **不做 column 切分**：源页直接当作单列处理（M4 算法已落地但 M5 直通模式不调用）
//! - **不做 reflow / wrap**：M6 wrap_state 推迟到 Step 8.x
//! - **不做 OCR**：M7 推迟到 Step 9.x
//! - **不做 fully-justify**：M6 推迟
//! - **DPI 策略**：用 settings.dst_dpi 作为 source render DPI，让 mutool 渲染出
//!   接近目标尺寸的位图；ConvertContext canvas 初始化为 dst_width × ?，blit 时
//!   左对齐 + 右侧补白
//! - **分页策略**：每渲完一源页就尝试 flush_page；末尾 flush_remaining
//!
//! 完整版本（含 column/row/word 切分 + reflow + 复杂分页）由 M6/M7/M8 增量增强。
//!
//! # Step 7.4 增量
//!
//! - 加 `observer: Arc<dyn ProgressObserver>` + `cancel: CancellationToken` 字段
//! - run() 在 JobStart / PageStart / PageDone / PdfWrite / JobDone 各阶段 emit 事件
//! - 每页前检查 cancel；触发即返 [`ConvertError::Cancelled`]，RAII 资源由 Drop 清理
//! - 新增 builder API：[`ConvertJob::with_observer`] / [`ConvertJob::with_cancel`]

use crate::observer::{CancellationToken, NopObserver, ProgressEvent, ProgressObserver};
use crate::ocr_bridge::{build_ocr_input, recognize_for_master};
use k2layout::reflow_pipeline::{ReflowError, ReflowOutcome, ReflowSettings};
use k2layout::{
    output_page_from_paginator, BreakSettings, ConvertContext, RowSettings, MAX_PAGE_BREAK_MARKS,
};
use k2ocr::{OcrEngine, OcrError};
use k2pdfout::{apply_ocr_words_to_writer, LopdfWriter, PdfWriter};
use k2render::{DocumentRenderer, MutoolRenderer};
use k2settings::ocr::OcrSettings;
use k2settings::{ReflowMode, Settings};
use k2types::PixelFormat;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// ConvertJob 错误。
#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    /// 渲染失败（mutool 异常 / PDF 加密 / 文件损坏）。
    #[error("render error: {0}")]
    Render(#[source] anyhow::Error),
    /// PDF 写出失败（lopdf 异常 / 路径不可写）。
    #[error("write error: {0}")]
    Write(#[source] anyhow::Error),
    /// Layout / breakpoint 算法返回错误。
    #[error("layout error: {0}")]
    Layout(#[source] anyhow::Error),
    /// Settings 配置不合法（例如 dst_width <= 0）。
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    /// 用户取消（CancellationToken 翻 true 后下一检查点抛出）。
    ///
    /// 退出码语义：CLI 退出 130（128 + SIGINT），与 POSIX shell 约定一致。
    #[error("cancelled by user")]
    Cancelled,
    /// OCR 引擎错误（Step 9.3 新增）。
    #[error("ocr error: {0}")]
    Ocr(#[source] anyhow::Error),
}

/// 简化版 ConvertJob 配置（从 [`Settings`] 提取核心字段）。
///
/// Step 7.3 仅取 M5 端到端最简化模式所需的字段；后续 Milestone 扩字段。
#[derive(Debug, Clone)]
pub struct ConvertJobConfig {
    /// 目标设备 DPI（C `k2settings->dst_dpi`）。
    pub dst_dpi: i32,
    /// 目标设备宽度（pixels，C `dst_width`）。
    pub dst_width: u32,
    /// 目标设备高度（pixels，C `dst_height`）。
    pub dst_height: u32,
    /// JPEG 质量（`-1` = Flate / `>=0` = JPEG quality）。
    pub jpeg_quality: i32,
    /// 是否启用分页 break point 算法（false = 直接 split）。Step 7.3 默认 true。
    pub use_breakpoint: bool,
    /// 是否合并图与图注（C `join_figure_captions`）。Step 7.3 默认 false。
    pub join_figure_captions: bool,
}

impl Default for ConvertJobConfig {
    fn default() -> Self {
        Self {
            dst_dpi: 167,
            dst_width: 560,
            dst_height: 735,
            jpeg_quality: 85,
            use_breakpoint: true,
            join_figure_captions: false,
        }
    }
}

impl ConvertJobConfig {
    /// 从 [`Settings`] 提取 M5 端到端所需的核心字段。
    ///
    /// # C 字段对照
    ///
    /// - `dst_dpi` ← `settings.destination.dst_dpi`
    /// - `dst_width` ← `settings.destination.dst_width`
    /// - `dst_height` ← `settings.destination.dst_height`
    /// - `jpeg_quality` ← `settings.output.jpeg_quality`
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            dst_dpi: settings.destination.dst_dpi.max(72),
            dst_width: u32::try_from(settings.destination.dst_width.max(1)).unwrap_or(560),
            dst_height: u32::try_from(settings.destination.dst_height.max(1)).unwrap_or(735),
            jpeg_quality: settings.output.jpeg_quality,
            use_breakpoint: true,
            join_figure_captions: settings.behavior.join_figure_captions != 0,
        }
    }
}

/// 单文件转换任务。
///
/// # Step 7.3 字段
///
/// - `input_path` / `output_path` / `config`：基础三件
///
/// # Step 7.4 字段（ADR-013）
///
/// - `observer: Arc<dyn ProgressObserver>`：默认 [`NopObserver`]
/// - `cancel: CancellationToken`：默认未取消
///
/// 典型用法：
///
/// ```no_run
/// use k2pipeline::{ConvertJob, ConvertJobConfig};
///
/// let job = ConvertJob::new("input.pdf", "output.pdf", ConvertJobConfig::default());
/// job.run().expect("conversion ok");
/// ```
///
/// 注入 observer + cancel：
///
/// ```no_run
/// use k2pipeline::{
///     CancellationToken, ConvertJob, ConvertJobConfig, NopObserver, ProgressObserver,
/// };
/// use std::sync::Arc;
///
/// let token = CancellationToken::new();
/// let observer: Arc<dyn ProgressObserver> = Arc::new(NopObserver);
/// let job = ConvertJob::new("input.pdf", "output.pdf", ConvertJobConfig::default())
///     .with_observer(observer)
///     .with_cancel(token);
/// let _ = job.run();
/// ```
#[derive(Clone)]
pub struct ConvertJob {
    /// 输入 PDF 路径
    pub input_path: PathBuf,
    /// 输出 PDF 路径
    pub output_path: PathBuf,
    /// 转换配置
    pub config: ConvertJobConfig,
    /// 进度观察者（默认 [`NopObserver`]）
    pub observer: Arc<dyn ProgressObserver>,
    /// 取消令牌（默认未取消）
    pub cancel: CancellationToken,
    /// OCR 引擎（Step 9.3 新增；`None` = OCR off）
    pub ocr_engine: Option<Arc<dyn OcrEngine>>,
    /// OCR 设置（Step 9.3 新增；`dst_ocr != Tesseract` 时即便 engine 存在也不跑 OCR）
    pub ocr_settings: OcrSettings,
    /// Reflow pipeline 主路径选择（Step 11.4 新增；默认 [`ReflowMode::Auto`]）。
    ///
    /// - [`ReflowMode::Off`]：调 [`ConvertContext::add_bitmap`] 走 v0.1.0 M5 直通路径
    /// - [`ReflowMode::Auto`] / [`ReflowMode::Force`]：调
    ///   [`ConvertContext::add_bitmap_with_reflow`] 走 figure bypass + text reflow
    pub reflow_mode: ReflowMode,
    /// Reflow pipeline 运行时配置（Step 11.4 新增）。
    ///
    /// 仅在 `reflow_mode != Off` 时使用。`region_dpi` 字段由 `run()` 主循环每次调用
    /// 时按 `dst_dpi` 自动覆盖（同 master::add_bitmap_with_reflow 内部 `effective.region_dpi`
    /// 覆盖策略）。其余字段调用方通过 [`Self::with_reflow_settings`] 注入。
    pub reflow_settings: ReflowSettings,
}

impl std::fmt::Debug for ConvertJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Arc<dyn ProgressObserver> 不实现 Debug，单独跳过
        f.debug_struct("ConvertJob")
            .field("input_path", &self.input_path)
            .field("output_path", &self.output_path)
            .field("config", &self.config)
            .field("observer", &"<dyn ProgressObserver>")
            .field("cancel", &self.cancel)
            .field(
                "ocr_engine",
                &self.ocr_engine.as_ref().map(|_| "<dyn OcrEngine>"),
            )
            .field("ocr_settings", &self.ocr_settings)
            .field("reflow_mode", &self.reflow_mode)
            .field("reflow_settings", &self.reflow_settings)
            .finish()
    }
}

impl ConvertJob {
    /// 构造单文件转换任务（observer = NopObserver / cancel = 未取消 / OCR off）。
    #[must_use]
    pub fn new<P: AsRef<Path>, Q: AsRef<Path>>(
        input: P,
        output: Q,
        config: ConvertJobConfig,
    ) -> Self {
        Self {
            input_path: input.as_ref().to_path_buf(),
            output_path: output.as_ref().to_path_buf(),
            config,
            observer: Arc::new(NopObserver),
            cancel: CancellationToken::new(),
            ocr_engine: None,
            ocr_settings: OcrSettings::default(),
            reflow_mode: ReflowMode::default(),
            reflow_settings: ReflowSettings::default(),
        }
    }

    /// 注入进度观察者（builder）。
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn ProgressObserver>) -> Self {
        self.observer = observer;
        self
    }

    /// 注入取消令牌（builder）。
    #[must_use]
    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// 注入 OCR 引擎（builder，Step 9.3）。
    ///
    /// 同时需要在 [`Self::with_ocr_settings`] 把 `dst_ocr` 设为
    /// [`k2settings::ocr::OcrMode::Tesseract`]，否则即便 engine 已注入也不跑 OCR
    /// （`ocr_settings.dst_ocr` 是开关）。
    #[must_use]
    pub fn with_ocr_engine(mut self, engine: Arc<dyn OcrEngine>) -> Self {
        self.ocr_engine = Some(engine);
        self
    }

    /// 注入 OCR 设置（builder，Step 9.3）。
    #[must_use]
    pub fn with_ocr_settings(mut self, settings: OcrSettings) -> Self {
        self.ocr_settings = settings;
        self
    }

    /// 注入 reflow 主路径模式（builder，Step 11.4）。
    ///
    /// - [`ReflowMode::Off`]：主循环走 [`ConvertContext::add_bitmap`]（v0.1.0 兼容回退）
    /// - [`ReflowMode::Auto`]（默认）/ [`ReflowMode::Force`]：主循环走
    ///   [`ConvertContext::add_bitmap_with_reflow`]，命中 text 时跑完整 column/row/word
    ///   wrap reflow，命中 figure 时走 bypass
    #[must_use]
    pub fn with_reflow_mode(mut self, mode: ReflowMode) -> Self {
        self.reflow_mode = mode;
        self
    }

    /// 注入 reflow pipeline 运行时配置（builder，Step 11.4）。
    ///
    /// `region_dpi` 字段会被 [`Self::run`] 主循环按 `dst_dpi` 自动覆盖；其余字段
    /// （`figure` / `dst_viewport_*` / `landscape` / `column_settings` /
    /// `row_settings` / `word_settings` / `wrap_settings`）由调用方提供。
    #[must_use]
    pub fn with_reflow_settings(mut self, settings: ReflowSettings) -> Self {
        self.reflow_settings = settings;
        self
    }

    /// 在关键检查点抛 [`ConvertError::Cancelled`]。
    ///
    /// 调用点：每个 source page 渲染前、每次 add_page 前、finish 前。
    /// 见 ADR-013「取消语义」。
    #[inline]
    fn check_cancel(&self) -> Result<(), ConvertError> {
        if self.cancel.is_cancelled() {
            Err(ConvertError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// 执行单文件转换：渲染 → layout → PDF 写出。
    ///
    /// # 错误
    ///
    /// - [`ConvertError::Render`]：mutool 调用失败 / PDF 加密
    /// - [`ConvertError::Write`]：输出路径不可写 / lopdf 错误
    /// - [`ConvertError::Layout`]：breakpoint 内部 find_textrows 越界
    /// - [`ConvertError::InvalidConfig`]：dst_width/dst_height = 0 等
    /// - [`ConvertError::Cancelled`]：CancellationToken 触发
    pub fn run(&self) -> Result<(), ConvertError> {
        if self.config.dst_width == 0 || self.config.dst_height == 0 {
            return Err(ConvertError::InvalidConfig(format!(
                "dst_width={} dst_height={} must be > 0",
                self.config.dst_width, self.config.dst_height
            )));
        }

        let started = Instant::now();

        // 注：取消可能在 mutool 启动子进程时插入；mutool 失败时 Render 错误优先。
        self.check_cancel()?;

        // ---- 1) Renderer ----
        let renderer = MutoolRenderer::new(&self.input_path).map_err(ConvertError::Render)?;
        let page_count = renderer.page_count().map_err(ConvertError::Render)?;

        self.observer.on_event(&ProgressEvent::JobStart {
            total_pages: page_count,
        });

        // ---- 2) ConvertContext ----
        let mut ctx = ConvertContext::new();
        ctx.init_canvas(self.config.dst_width, PixelFormat::Gray8);

        let break_settings = BreakSettings {
            fit_to_page: 0,
            dst_dpi: self.config.dst_dpi,
            join_figure_captions: self.config.join_figure_captions,
            row_settings: RowSettings::default(),
            bgcolor: 255,
        };

        // ---- 3) PdfWriter ----
        // Step 11.11 P1-2：把 OcrSettings.dst_ocr_visibility_flags 注入 LopdfWriter
        // 控制 add_ocr_layer 写文字层/box 的策略（默认 SHOW_SOURCE → 不写 OCR 文字层）。
        let writer = LopdfWriter::new(&self.output_path)
            .map_err(ConvertError::Write)?
            .with_ocr_visibility(self.ocr_settings.dst_ocr_visibility_flags);
        let mut writer = Box::new(writer);
        let mut dst_pages_written: usize = 0;

        // ---- 4) 主循环：渲染 → [OCR →] add_bitmap → flush ----
        for source_page in 0..page_count {
            // 4a) cancel check
            self.check_cancel()?;

            self.observer
                .on_event(&ProgressEvent::PageStart { source_page });

            let bp = renderer
                .render_page(source_page, self.config.dst_dpi as f32)
                .map_err(ConvertError::Render)?;

            // 转灰度（mutool 默认 RGBA，我们走 Gray8 路径）
            let gray = render_to_gray8(&bp.bitmap);
            // Step 11.4：根据 reflow_mode 在 add_bitmap (v0.1.0 直通) 与
            // add_bitmap_with_reflow (figure bypass + text reflow) 之间分流。
            // Step 11.5：OCR 调用点也按 reflow_mode 分流——
            //   - `ReflowMode::Off`：保留 Step 9.3 实装的「主循环跑整页 OCR」路径
            //     （v0.1.0 兼容回退，整页都跑不区分 figure / text）
            //   - `ReflowMode::Auto | Force`：OCR 调用收编到
            //     `add_bitmap_with_reflow` 内部的 `TextReflowed` 路径（figure /
            //     skip / direct_blit 不跑），由 `process_region` 接管，
            //     与 C `k2master.c:740-745` 等价语义；OcrPage 事件在拿到 outcome
            //     后按 `ocr_words.len()` emit（仅 TextReflowed 路径）。
            match self.reflow_mode {
                ReflowMode::Off => {
                    // v0.1.0 主循环 OCR 块：保留以维持 `--reflow off` 路径下用户
                    // 端到端 OCR 行为不变（Off 路径不知道 figure / text，按页跑）。
                    if let (Some(engine), Some(input)) = (
                        self.ocr_engine.as_ref(),
                        build_ocr_input(&self.ocr_settings, &bp.bitmap),
                    ) {
                        let gap = ctx.calculate_line_gap(self.config.dst_dpi as f64) as f64;
                        let dy = ctx.canvas.rows as f64 + gap;
                        // Step 11.8 P0-5：OcrError::Cancelled 单独映射到
                        // ConvertError::Cancelled（保 ExitCode 130），其余 OCR 错误
                        // 仍走 ConvertError::Ocr 通道（保 ExitCode 1）。
                        let words = recognize_for_master(engine.as_ref(), &input, 0.0, dy)
                            .map_err(map_ocr_error_to_convert_error)?;
                        self.observer.on_event(&ProgressEvent::OcrPage {
                            source_page,
                            words_found: words.len(),
                        });
                        ctx.ocr.concatenate(words);
                    }
                    ctx.add_bitmap(gray.0, gray.1, &gray.2, self.config.dst_dpi as f64, 0, 200);
                }
                ReflowMode::Auto | ReflowMode::Force => {
                    // Step 11.5：把 ocr_settings 写到 effective.ocr_settings，让
                    // `process_region` 内的 OCR helper 自洽决定是否触发；同时透传
                    // `ocr_engine: Option<&dyn OcrEngine>` 引用。
                    let mut effective = self.reflow_settings.clone();
                    effective.region_dpi = self.config.dst_dpi as f64;
                    effective.ocr_settings = self.ocr_settings.clone();
                    let ocr_engine_ref: Option<&dyn OcrEngine> =
                        self.ocr_engine.as_ref().map(|arc| arc.as_ref());
                    // Step 11.8 P0-5：解构 ReflowError::Ocr(OcrError::Cancelled) →
                    // ConvertError::Cancelled，避免被吞成 ConvertError::Layout。
                    // 其余 ReflowError 仍走 Layout 通道维持 v0.2 P0-2 行为。
                    let outcome = ctx
                        .add_bitmap_with_reflow(
                            gray.0,
                            gray.1,
                            &gray.2,
                            PixelFormat::Gray8,
                            self.config.dst_dpi as f64,
                            0,
                            200,
                            &effective,
                            ocr_engine_ref,
                        )
                        .map_err(map_reflow_error_to_convert_error)?;
                    // Step 11.5：仅在 TextReflowed 路径 emit OcrPage（figure /
                    // skip / direct_blit 路径不跑 OCR，对应「OcrPage 事件数 <
                    // source page 数」语义）。Off 路径 emit 已在上面那一支处理。
                    if let ReflowOutcome::TextReflowed { ocr_words, .. } = &outcome {
                        self.observer.on_event(&ProgressEvent::OcrPage {
                            source_page,
                            words_found: ocr_words.len(),
                        });
                    }
                }
            }
            // 立即把可 flush 的页推到 paginator
            self.drain_full_pages(&mut ctx, &break_settings, source_page as i32)?;
            // 把已 queued 的页 → writer
            let written_now =
                self.flush_queue_to_writer(&mut ctx, &mut *writer, &mut dst_pages_written)?;

            self.observer.on_event(&ProgressEvent::PageDone {
                source_page,
                dst_pages: written_now,
            });
        }

        // 末尾 flush 前最后一次 cancel check
        self.check_cancel()?;

        // ---- 5) 末尾：flush 剩余 + 最后一次 drain ----
        if !self.config.use_breakpoint {
            ctx.flush_remaining(-1);
        } else {
            // 还有不足一页的内容 → flush_page maxsize=0 走早退路径切整段
            let _ = ctx.flush_remaining(-1);
        }
        let final_written =
            self.flush_queue_to_writer(&mut ctx, &mut *writer, &mut dst_pages_written)?;
        if final_written > 0 {
            // emit 一个收尾的 PdfWrite，使 observer 能拿到最终页数
            self.observer.on_event(&ProgressEvent::PdfWrite {
                dst_pages_written,
                total_dst_pages: dst_pages_written,
            });
        }

        // ---- 6) outline → writer ----
        let entries = ctx.outline.collect_for_writer();
        for e in entries {
            writer.add_outline(e).map_err(ConvertError::Write)?;
        }

        // ---- 7) writer.finish ----
        self.check_cancel()?;
        writer.finish().map_err(ConvertError::Write)?;

        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.observer.on_event(&ProgressEvent::JobDone {
            total_input_pages: page_count,
            total_output_pages: dst_pages_written,
            elapsed_ms,
        });

        Ok(())
    }

    /// 把 ConvertContext 中已 queued 的 PaginatorPage 转 OutputPage 喂 writer。
    ///
    /// 返回本次 flush 真实写入的 dst page 数（observer 用）。
    fn flush_queue_to_writer(
        &self,
        ctx: &mut ConvertContext,
        writer: &mut dyn PdfWriter,
        dst_pages_written: &mut usize,
    ) -> Result<usize, ConvertError> {
        let mut n_this_call: usize = 0;
        while let Some(mut pp) = ctx.paginator.pop_page() {
            // 写盘前检查 cancel，与 ADR-013 一致
            self.check_cancel()?;
            // Step 9.3: 把 ocr_words 取出（pp 接着被 move 给 output_page_from_paginator）
            let ocr_words = std::mem::take(&mut pp.ocr_words);
            let op = output_page_from_paginator(
                pp,
                self.config.dst_dpi as f32,
                self.config.jpeg_quality,
                0, // halfsize=0 (Step 7.2 主路径)
                0.0,
            )
            .map_err(|e| ConvertError::Layout(anyhow::anyhow!("{}", e)))?;
            writer.add_page(&op).map_err(ConvertError::Write)?;
            // Step 9.3: 把本页 OCR 不可见层附加到刚 add 的 page
            if !ocr_words.is_empty() {
                apply_ocr_words_to_writer(writer, &ocr_words)
                    .map_err(|e| ConvertError::Write(anyhow::anyhow!("{}", e)))?;
            }
            *dst_pages_written += 1;
            n_this_call += 1;
            // 每写一页 emit 一个 PdfWrite（total 仍是 running 值，结束时再发一次终值）
            self.observer.on_event(&ProgressEvent::PdfWrite {
                dst_pages_written: *dst_pages_written,
                total_dst_pages: *dst_pages_written,
            });
        }
        Ok(n_this_call)
    }

    /// 在 canvas 满一页时反复 flush_page，把多页内容推到 queue。
    fn drain_full_pages(
        &self,
        ctx: &mut ConvertContext,
        break_settings: &BreakSettings,
        srcpageno: i32,
    ) -> Result<(), ConvertError> {
        let dst_height = self.config.dst_height;
        // 防御性：避免极小 dst_height 导致死循环
        if dst_height == 0 {
            return Ok(());
        }
        // 最多循环 MAX_PAGE_BREAK_MARKS * 4 次（保护值，正常 case 远不会到）
        let safety_cap = MAX_PAGE_BREAK_MARKS * 4 + 4;
        let mut iter = 0;
        while ctx.canvas.rows >= dst_height && iter < safety_cap {
            // 在每次 flush_page 前检查 cancel
            self.check_cancel()?;
            ctx.flush_page(dst_height, break_settings, srcpageno)
                .map_err(|e| ConvertError::Layout(anyhow::anyhow!("{}", e)))?;
            iter += 1;
        }
        Ok(())
    }
}

/// 把任意 PixelFormat 的 [`k2types::Bitmap`] 转换为 Gray8 像素（按亮度公式）。
///
/// 返回 (width, height, pixels)，pixels 长度 = width * height。
fn render_to_gray8(bmp: &k2types::Bitmap) -> (u32, u32, Vec<u8>) {
    match bmp.format {
        PixelFormat::Gray8 => (bmp.width, bmp.height, bmp.pixels.clone()),
        PixelFormat::Rgb8 => {
            let n = (bmp.width as usize) * (bmp.height as usize);
            let mut out = Vec::with_capacity(n);
            for chunk in bmp.pixels.chunks_exact(3) {
                let lum = (0.299_f32 * f32::from(chunk[0])
                    + 0.587_f32 * f32::from(chunk[1])
                    + 0.114_f32 * f32::from(chunk[2]))
                .round()
                .clamp(0.0, 255.0) as u8;
                out.push(lum);
            }
            (bmp.width, bmp.height, out)
        }
        PixelFormat::Rgba8 => {
            let n = (bmp.width as usize) * (bmp.height as usize);
            let mut out = Vec::with_capacity(n);
            for chunk in bmp.pixels.chunks_exact(4) {
                let lum = (0.299_f32 * f32::from(chunk[0])
                    + 0.587_f32 * f32::from(chunk[1])
                    + 0.114_f32 * f32::from(chunk[2]))
                .round()
                .clamp(0.0, 255.0) as u8;
                out.push(lum);
            }
            (bmp.width, bmp.height, out)
        }
    }
}

/// Step 11.8 P0-5：把 OCR 错误映射成 ConvertError。
///
/// - [`OcrError::Cancelled`] → [`ConvertError::Cancelled`]（ExitCode 130，POSIX 约定）
/// - 其余 [`OcrError`] → [`ConvertError::Ocr`]（ExitCode 1）
///
/// 这层映射保 ctrl-c 在 OCR 子进程跑到一半时也能正确退出 130，与
/// ADR-013 / Step 7.4 既定语义一致。
fn map_ocr_error_to_convert_error(e: OcrError) -> ConvertError {
    match e {
        OcrError::Cancelled => ConvertError::Cancelled,
        other => ConvertError::Ocr(anyhow::anyhow!("{}", other)),
    }
}

/// Step 11.8 P0-5：把 Reflow 错误映射成 ConvertError。
///
/// `ReflowError::Ocr(OcrError::Cancelled)` 是 Auto/Force 路径下 OCR 取消的传播路径，
/// 单独解构出来映射到 [`ConvertError::Cancelled`]，避免被吞成 `Layout` 通道。
/// 其余 ReflowError 仍走 [`ConvertError::Layout`] 维持 v0.2 P0-2 行为。
fn map_reflow_error_to_convert_error(e: ReflowError) -> ConvertError {
    match e {
        ReflowError::Ocr(OcrError::Cancelled) => ConvertError::Cancelled,
        other => ConvertError::Layout(anyhow::anyhow!("{}", other)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::observer::RecordingObserver;
    use k2types::Bitmap;

    #[test]
    fn config_default_values() {
        let c = ConvertJobConfig::default();
        assert_eq!(c.dst_dpi, 167);
        assert_eq!(c.dst_width, 560);
        assert_eq!(c.dst_height, 735);
        assert_eq!(c.jpeg_quality, 85);
        assert!(c.use_breakpoint);
        assert!(!c.join_figure_captions);
    }

    #[test]
    fn config_from_settings() {
        let mut s = Settings::default();
        s.destination.dst_dpi = 200;
        s.destination.dst_width = 800;
        s.destination.dst_height = 1024;
        s.output.jpeg_quality = 90;
        let c = ConvertJobConfig::from_settings(&s);
        assert_eq!(c.dst_dpi, 200);
        assert_eq!(c.dst_width, 800);
        assert_eq!(c.dst_height, 1024);
        assert_eq!(c.jpeg_quality, 90);
    }

    #[test]
    fn config_clamps_invalid_dpi_to_minimum() {
        let mut s = Settings::default();
        s.destination.dst_dpi = 10; // below 72
        let c = ConvertJobConfig::from_settings(&s);
        assert_eq!(c.dst_dpi, 72);
    }

    #[test]
    fn render_to_gray8_gray_passthrough() {
        let pixels = vec![100, 200, 50];
        let bmp = Bitmap::from_raw(3, 1, 72.0, PixelFormat::Gray8, pixels.clone()).unwrap();
        let (w, h, out) = render_to_gray8(&bmp);
        assert_eq!(w, 3);
        assert_eq!(h, 1);
        assert_eq!(out, pixels);
    }

    #[test]
    fn render_to_gray8_rgb_luminance() {
        // 红=(255,0,0) → 0.299*255 = 76.245 → 76
        let pixels = vec![255, 0, 0, 0, 255, 0, 0, 0, 255];
        let bmp = Bitmap::from_raw(3, 1, 72.0, PixelFormat::Rgb8, pixels).unwrap();
        let (_, _, out) = render_to_gray8(&bmp);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], 76); // red → 76
        assert_eq!(out[1], 150); // green → 0.587*255 = 149.685 → 150
        assert_eq!(out[2], 29); // blue → 0.114*255 = 29.07 → 29
    }

    #[test]
    fn render_to_gray8_rgba_drops_alpha() {
        // RGBA: red w/ alpha=128 → 应丢 alpha → 76
        let pixels = vec![255, 0, 0, 128];
        let bmp = Bitmap::from_raw(1, 1, 72.0, PixelFormat::Rgba8, pixels).unwrap();
        let (_, _, out) = render_to_gray8(&bmp);
        assert_eq!(out, vec![76]);
    }

    #[test]
    fn convert_job_construct() {
        let job = ConvertJob::new("a.pdf", "b.pdf", ConvertJobConfig::default());
        assert_eq!(job.input_path.to_str().unwrap(), "a.pdf");
        assert_eq!(job.output_path.to_str().unwrap(), "b.pdf");
        assert_eq!(job.config.dst_dpi, 167);
        // 默认未取消，observer 为 NopObserver
        assert!(!job.cancel.is_cancelled());
    }

    #[test]
    fn convert_job_with_observer_attaches_observer() {
        let obs: Arc<dyn ProgressObserver> = Arc::new(RecordingObserver::new());
        let job = ConvertJob::new("a.pdf", "b.pdf", ConvertJobConfig::default())
            .with_observer(Arc::clone(&obs));
        // 通过 Arc::ptr_eq 验证（需要 Arc<dyn> 的指针比较）
        let job_obs: *const dyn ProgressObserver = Arc::as_ptr(&job.observer);
        let want_obs: *const dyn ProgressObserver = Arc::as_ptr(&obs);
        // 仅比较数据指针；vtable ptr 可能不同但数据相同
        let job_data = job_obs.cast::<()>();
        let want_data = want_obs.cast::<()>();
        assert_eq!(job_data, want_data);
    }

    #[test]
    fn convert_job_with_cancel_attaches_token() {
        let tok = CancellationToken::new();
        let job =
            ConvertJob::new("a.pdf", "b.pdf", ConvertJobConfig::default()).with_cancel(tok.clone());
        tok.cancel();
        assert!(job.cancel.is_cancelled());
    }

    #[test]
    fn run_with_zero_dst_width_fails() {
        let config = ConvertJobConfig {
            dst_width: 0,
            ..ConvertJobConfig::default()
        };
        let job = ConvertJob::new("a.pdf", "b.pdf", config);
        let err = job.run().unwrap_err();
        assert!(matches!(err, ConvertError::InvalidConfig(_)));
    }

    #[test]
    fn run_with_zero_dst_height_fails() {
        let config = ConvertJobConfig {
            dst_height: 0,
            ..ConvertJobConfig::default()
        };
        let job = ConvertJob::new("a.pdf", "b.pdf", config);
        let err = job.run().unwrap_err();
        assert!(matches!(err, ConvertError::InvalidConfig(_)));
    }

    #[test]
    fn run_with_pre_cancelled_token_returns_cancelled_before_renderer() {
        // 预先翻 cancel，run() 应在 mutool 启动之前就退出
        let tok = CancellationToken::new();
        tok.cancel();
        let job = ConvertJob::new(
            "/nonexistent/path/should_not_be_reached.pdf",
            "/nonexistent/output.pdf",
            ConvertJobConfig::default(),
        )
        .with_cancel(tok);
        let err = job.run().unwrap_err();
        assert!(matches!(err, ConvertError::Cancelled));
    }

    #[test]
    fn convert_error_cancelled_display() {
        let e = ConvertError::Cancelled;
        let s = format!("{e}");
        assert!(s.contains("cancelled"));
    }

    #[test]
    fn check_cancel_returns_ok_when_not_cancelled() {
        let job = ConvertJob::new("a", "b", ConvertJobConfig::default());
        assert!(job.check_cancel().is_ok());
    }

    #[test]
    fn check_cancel_returns_cancelled_after_token_flip() {
        let job = ConvertJob::new("a", "b", ConvertJobConfig::default());
        job.cancel.cancel();
        let err = job.check_cancel().unwrap_err();
        assert!(matches!(err, ConvertError::Cancelled));
    }

    #[test]
    fn convert_job_debug_format_works() {
        let job = ConvertJob::new("a.pdf", "b.pdf", ConvertJobConfig::default());
        let s = format!("{job:?}");
        assert!(s.contains("ConvertJob"));
        assert!(s.contains("a.pdf"));
        assert!(s.contains("dyn ProgressObserver"));
    }
}
