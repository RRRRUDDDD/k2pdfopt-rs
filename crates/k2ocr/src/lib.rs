//! `k2ocr` —— OCR 引擎抽象 + Tesseract CLI 适配（ADR-017）。
//!
//! 来源 C 文件：`k2pdfoptlib/k2ocr.c`、`willuslib/ocr.c`、`willuslib/ocrtess.c`、`willuslib/ocrwords.c`。
//!
//! # 选型决策（ADR-017）
//!
//! - **MVP** 默认 [`TesseractCliEngine`]（命令行子进程 + TSV 输出，零 native 依赖）
//! - **feature `leptess`** 启用 [`LeptessEngine`]：当前是占位 stub，
//!   真实 FFI 实现推迟 M7 末（详见 [`leptess_ffi`] 模块文档）
//!
//! # 设计要点
//!
//! - [`OcrEngine`] trait 是 `Send + Sync` 的对象安全 trait，可用 `Box<dyn OcrEngine>` 传递
//! - 关键类型（[`OcrPageInput`]、[`OcrError`]、[`OcrEngineInfo`]、[`OcrRoi`]、
//!   [`PageSegmentationMode`]、[`OcrEngineMode`]）均在 [`types`] 模块
//! - 单 word 输出复用 [`k2types::OcrWord`]（Step 7.2 已落地的权威定义）
//!
//! # 示例
//!
//! ```no_run
//! use k2ocr::{OcrEngine, OcrPageInput, TesseractCliEngine};
//! use k2types::{Bitmap, PixelFormat};
//!
//! let bmp = Bitmap::new(800, 600, 300.0, PixelFormat::Gray8).unwrap();
//! let engine = TesseractCliEngine::new();
//! let words = engine.recognize(
//!     &OcrPageInput::new(&bmp, 300.0).with_lang("eng"),
//! ).unwrap();
//! for w in &words {
//!     println!("{} @({},{}) {}x{} conf={:.2}", w.text, w.x, w.y, w.w, w.h, w.confidence);
//! }
//! ```
//!
//! 详见 `rust-rewrite-plan.md` v2.1 §5.2 / §9.5 / §10 M7 与
//! `rust-rewrite-execution-plan.md` Step 9.1。

#![forbid(unsafe_code)]

pub mod lang;
pub mod mapping;
mod scoped_tempfile;
pub mod tesseract_cli;
pub mod tsv_parser;
pub mod types;

#[cfg(feature = "leptess")]
pub mod leptess_ffi;

use k2types::OcrWord;

pub use lang::{
    download_hint_default, resolve as resolve_lang, LangResolution, LangResolveError, LangSpec,
    ResolveOptions, DEFAULT_FALLBACK_LANG, DEFAULT_TESSDATA_URL_TEMPLATE,
};
pub use tesseract_cli::TesseractCliEngine;
pub use tsv_parser::{parse_tsv, TsvWord};
pub use types::{
    OcrEngineInfo, OcrEngineMode, OcrError, OcrPageInput, OcrRoi, PageSegmentationMode,
};

#[cfg(feature = "leptess")]
pub use leptess_ffi::LeptessEngine;

/// OCR 引擎的对象安全 trait。
///
/// 所有方法都接 `&self`（不可变借用），因为 OCR 引擎自身不持有可变状态——
/// 缓存（如版本信息、语言列表）由实现侧用 [`std::sync::OnceLock`] 等线程安全机制管理。
///
/// 实现要点：
/// - `engine_name` 返回静态字符串便于日志/调试（与 [`OcrEngineInfo::engine_name`] 一致）
/// - `probe` 自检（执行子进程或加载 native lib），是惰性缓存的首个调用点
/// - `list_langs` 返回引擎可用的语言短名（如 `["eng", "osd"]`）
/// - `recognize` 执行 OCR：输入 [`OcrPageInput`]（含 ROI/lang/PSM/OEM/min_conf），
///   返回 [`OcrWord`] 列表（坐标已加回 ROI offset，confidence 归一化到 `0.0..=1.0`）
pub trait OcrEngine: Send + Sync {
    /// 引擎短名，如 `"tesseract-cli"`、`"leptess-ffi"`。
    fn engine_name(&self) -> &'static str;

    /// 自检：检测引擎可用性，返回版本/数据目录等元信息。
    ///
    /// 实现应缓存结果，多次调用零开销。
    fn probe(&self) -> Result<OcrEngineInfo, OcrError>;

    /// 列出引擎可用的语言短名（不含 `+` 复合）。
    fn list_langs(&self) -> Result<Vec<String>, OcrError>;

    /// 对 `input` 跑 OCR 返回 word 列表。
    ///
    /// 坐标系：返回的 [`OcrWord::x`] / [`OcrWord::y`] 是**原始 Bitmap 局部像素坐标**
    /// （已加回 ROI offset），与 [`OcrPageInput::bitmap`] 同源 top-left 像素原点。
    ///
    /// Confidence 范围：`0.0..=1.0`（Tesseract 0-100 归一化）。
    fn recognize(&self, input: &OcrPageInput<'_>) -> Result<Vec<OcrWord>, OcrError>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use k2types::{Bitmap, PixelFormat};

    /// trait object 必须能 `Box<dyn OcrEngine>`（验证 trait 是对象安全的）。
    #[test]
    fn ocr_engine_is_object_safe() {
        let e: Box<dyn OcrEngine> = Box::new(TesseractCliEngine::new());
        assert_eq!(e.engine_name(), "tesseract-cli");
    }

    /// `OcrEngine` 必须 `Send + Sync` 以满足跨线程使用。
    #[test]
    fn ocr_engine_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Box<dyn OcrEngine>>();
    }

    /// 顶层 re-export 链路：能从 crate 根直接取所有公开类型。
    #[test]
    fn reexport_chain_complete() {
        let _: Option<OcrError> = None;
        let _: PageSegmentationMode = PageSegmentationMode::default();
        let _: OcrEngineMode = OcrEngineMode::default();
        let _: OcrRoi = OcrRoi::new(0, 0, 1, 1);
        let _: OcrEngineInfo = OcrEngineInfo {
            engine_name: String::new(),
            version: String::new(),
            data_path: None,
        };
    }

    /// 从 `TesseractCliEngine` 通过 trait 调用 `engine_name` 也得正确返回。
    #[test]
    fn trait_dispatch_engine_name() {
        let e: &dyn OcrEngine = &TesseractCliEngine::new();
        assert_eq!(e.engine_name(), "tesseract-cli");
    }

    /// `OcrPageInput` 接 Bitmap 引用，确认生命周期约束在 trait 层不漏。
    #[test]
    fn ocr_page_input_lifetime_compiles() {
        let bmp = Bitmap::new(10, 10, 300.0, PixelFormat::Gray8).unwrap();
        let inp = OcrPageInput::new(&bmp, 300.0);
        // 仅验证类型：不真跑 OCR
        let _e = TesseractCliEngine::new();
        let _ = &inp; // 借用保活
    }

    #[cfg(feature = "leptess")]
    #[test]
    fn leptess_feature_gated_visible_when_enabled() {
        let r = LeptessEngine::new();
        assert!(matches!(r, Err(OcrError::FeatureDisabled(_))));
    }
}
