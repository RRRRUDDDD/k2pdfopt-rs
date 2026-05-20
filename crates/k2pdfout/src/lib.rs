//! `k2pdfout` - Bitmap PDF writer + outline + 可选 OCR 不可见文字层。
//!
//! 来源 C 文件：`k2publish.c`、`pdfwrite.c`（1783 行）、`wpdf.c`、
//! `wpdfoutline.c`、`wpdfutil.c`。
//!
//! 选型决策（ADR-014）：基于 `lopdf 0.40` 薄封装。`printpdf` 评估后未选用。
//!
//! 详见 `rust-rewrite-plan.md` v2.1 §5.2 / §9.4 / §10 M5。
//!
//! # 模块构成
//!
//! - [`bitmap_pdf`]：`LopdfWriter` 主实现（Step 7.2）
//! - [`outline`]：扁平 `OutlineEntry` → 层级树投影（Step 7.3，本步骤）
//!
//! # 使用示例
//!
//! ```no_run
//! use k2pdfout::{LopdfWriter, PdfWriter};
//! use k2types::{Bitmap, OutputPage, PixelFormat};
//!
//! # fn main() -> anyhow::Result<()> {
//! let mut writer = LopdfWriter::new("/tmp/out.pdf")?;
//! let bmp = Bitmap::new(100, 100, 150.0, PixelFormat::Gray8)?;
//! let page = OutputPage::from_bitmap(0, bmp, 150.0);
//! writer.add_page(&page)?;
//! Box::new(writer).finish()?;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod bitmap_pdf;
pub mod ocr_layer;
pub mod outline;

pub use bitmap_pdf::LopdfWriter;
use k2types::{OcrWord, OutlineEntry, OutputPage};
pub use ocr_layer::{apply_ocr_words_to_writer, OcrLayerError};
pub use outline::{OutlineTree, OutlineTreeError};

/// PdfWriter trait（v2.1 §9.4）：把版面分析、OCR 后的页与书签写入 PDF 输出。
///
/// 实现者承诺：
/// - `add_page` 按 `page.page_index` 升序调用（典型由 layout pipeline 保证）
/// - `add_outline` 在 `add_page` 之间任意调用（dst_page 引用未来或已写入的 page index）
/// - `add_ocr_layer` 必须紧跟在 `add_page` 之后调用，把 words 归属到最近添加的页
/// - `finish` 是写盘 + 退出口，调用后实例 drop
///
/// 入参类型（OutputPage / OutlineEntry / OcrWord）来自 [`k2types`]，避免本 crate 依赖 layout。
pub trait PdfWriter: Send {
    /// 追加一页到输出 PDF。
    fn add_page(&mut self, page: &OutputPage) -> anyhow::Result<()>;

    /// 追加一个 outline 条目（书签）。所有条目在 [`PdfWriter::finish`] 时一次性写入。
    fn add_outline(&mut self, entry: OutlineEntry) -> anyhow::Result<()>;

    /// 为**最近一次** [`PdfWriter::add_page`] 添加的页注入 OCR 不可见文字层。
    /// 多次调用累积到同一页。在 `add_page` 之前调用返回 `Err`。
    fn add_ocr_layer(&mut self, words: &[OcrWord]) -> anyhow::Result<()>;

    /// 写盘 + 收尾（构建 Pages 字典、outline 树、Catalog，写入磁盘）。
    fn finish(self: Box<Self>) -> anyhow::Result<()>;
}

/// PDF writer 可恢复错误。
///
/// 与 ADR-008 错误模型对齐：库内用 `thiserror`，trait 表面用 `anyhow::Result` 透出。
/// 调用方按需要把 `PdfWriteError` `.into()` 为 `anyhow::Error`。
#[derive(Debug, thiserror::Error)]
pub enum PdfWriteError {
    /// 输出路径不可写（含父目录不存在 / 权限不足 / 磁盘满）。
    #[error("PDF output path not writable: {path} ({source})")]
    OutputPathNotWritable {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// 图像编码失败（JPEG 编码器 / Flate 压缩器异常）。
    #[error("image encoding failed for page {page_index}: {reason}")]
    ImageEncode { page_index: u32, reason: String },

    /// `add_ocr_layer` 在 `add_page` 之前调用。
    #[error("add_ocr_layer called before any add_page")]
    OcrBeforePage,

    /// outline 条目的 `parent_idx` 越界（指向尚未添加的条目）。
    #[error("outline entry parent_idx={parent_idx} out of bounds (current entries={current_len})")]
    OutlineParentOutOfBounds {
        parent_idx: usize,
        current_len: usize,
    },

    /// outline 条目的 `dst_page` 越界（指向尚未添加的页）。
    #[error("outline entry dst_page={dst_page} out of bounds (current pages={current_pages})")]
    OutlineDstPageOutOfBounds { dst_page: i32, current_pages: u32 },

    /// 不支持的位深参数（仅 halfsize=0 在 Step 7.2 支持）。
    #[error("unsupported halfsize={halfsize}; only halfsize=0 (8 bits) supported in Step 7.2")]
    UnsupportedHalfsize { halfsize: u8 },

    /// `lopdf` 内部错误（写盘、对象引用等）。
    #[error("lopdf error: {0}")]
    Lopdf(#[from] lopdf::Error),

    /// `image` 内部错误（JPEG 编解码）。
    #[error("image crate error: {0}")]
    Image(#[from] image::ImageError),

    /// 底层 IO 错误。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod trait_tests {
    use super::*;

    /// PdfWriter trait 必须 `Send`（与 v2.1 §9.4 签名一致）。
    #[test]
    fn pdf_writer_trait_object_is_send() {
        fn assert_send<T: Send + ?Sized>() {}
        assert_send::<dyn PdfWriter>();
    }

    /// PdfWriteError 在 anyhow 链中可被转换。
    #[test]
    fn pdf_write_error_converts_to_anyhow() {
        let err = PdfWriteError::OcrBeforePage;
        let _: anyhow::Error = err.into();
    }

    /// 错误 Display 文本含可读信息。
    #[test]
    fn ocr_before_page_display() {
        let err = PdfWriteError::OcrBeforePage;
        let s = format!("{err}");
        assert!(s.contains("add_ocr_layer"));
        assert!(s.contains("add_page"));
    }

    #[test]
    fn unsupported_halfsize_display() {
        let err = PdfWriteError::UnsupportedHalfsize { halfsize: 1 };
        let s = format!("{err}");
        assert!(s.contains("halfsize=1"));
        assert!(s.contains("Step 7.2"));
    }

    #[test]
    fn outline_dst_page_display() {
        let err = PdfWriteError::OutlineDstPageOutOfBounds {
            dst_page: 5,
            current_pages: 3,
        };
        let s = format!("{err}");
        assert!(s.contains("dst_page=5"));
        assert!(s.contains("current pages=3"));
    }
}
