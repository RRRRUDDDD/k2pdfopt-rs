//! `k2pipeline` - 文档级编排，单文件转换流程，对外稳定库 API。
//!
//! 来源 C 文件：`k2file.c`（2357 行）、`k2proc.c`（编排部分，约 3708 行的一部分）。
//!
//! 该 crate 是依赖图的顶层（除 `k2cli` 外），承担：
//! - [`ConvertJob`] / `ConvertContext` 编排（renderer → layout → PdfWriter）
//! - [`ProgressObserver`] 钩子（ADR-013，Step 7.4 落地）
//! - [`CancellationToken`] 协作式取消（ADR-013，Step 7.4 落地）
//! - 未来 GUI / Web 服务 / 批处理工具复用入口
//!
//! 详见 `rust-rewrite-plan.md` v2.1 §3 / §5.2 / §10 M5。
//!
//! # Step 7.3 落地范围（M5 端到端最简化版本）
//!
//! - [`ConvertJob`] struct：input_path / output_path / 简化版 settings 切片
//! - [`ConvertJob::run`]：渲染源 PDF → ConvertContext::add_bitmap → flush_page →
//!   PaginatorPage → OutputPage → LopdfWriter
//! - 当前不做 column / row / word 切分（M4 算法已落地但 M5 直通模式不调用），
//!   不做 reflow（M6 wrap_state，Step 8.x 落地），不做 OCR（M7，Step 9.x 落地）
//! - 输出 PDF：每页一个 source page（mutool 渲染输出的 bitmap 切到 dst_dpi 尺寸）
//!
//! # Step 7.4 增量
//!
//! - 新增 [`observer`] 模块：`ProgressEvent` / `ProgressObserver` / `NopObserver`
//!   / `RecordingObserver` / `CancellationToken`
//! - [`ConvertJob`] 加 `observer` + `cancel` 字段（builder API：`with_observer`
//!   / `with_cancel`）
//! - [`ConvertJob::run`] 在 JobStart / PageStart / PageDone / PdfWrite / JobDone
//!   各阶段 emit 事件；每页前检查 cancel，触发即返 [`ConvertError::Cancelled`]
//!
//! # Step 9.3 增量（M7）
//!
//! - 新增 [`ocr_bridge`] 模块：[`build_ocr_input`] 把 [`k2settings::OcrSettings`]
//!   映射到 [`k2ocr::OcrPageInput`]；[`recognize_for_master`] 跑 OCR + master
//!   坐标系平移一步完成
//! - [`ConvertJob`] 加 `ocr_engine: Option<Arc<dyn OcrEngine>>` + `ocr_settings: OcrSettings`
//!   字段（builder：`with_ocr_engine` / `with_ocr_settings`）
//! - [`ConvertJob::run`] 每页：渲染 → recognize（若启用）→ `ctx.ocr.concatenate`
//!   → add_bitmap → flush_page (内含 `ctx.ocr.drain_in_range` + `offset_y`) →
//!   write_page → `apply_ocr_words_to_writer`

#![forbid(unsafe_code)]

pub mod convert;
pub mod observer;
pub mod ocr_bridge;

pub use convert::{ConvertError, ConvertJob, ConvertJobConfig};
pub use observer::{
    CancellationToken, NopObserver, ProgressEvent, ProgressObserver, RecordingObserver,
};
pub use ocr_bridge::{build_ocr_input, recognize_for_master};
