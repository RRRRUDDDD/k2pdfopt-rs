//! `k2types` - v2.1 新增。承载跨 crate 共享类型，避免 `k2layout` ↔ `k2ocr` ↔ `k2pdfout`
//! 概念循环依赖。
//!
//! 计划承载：`OutputPage` / `OcrWord` / `OutlineEntry` / `PageId` / `BitmapPage` 等。
//! 没有对应 C 文件，是纯 Rust 类型层。
//!
//! Step 4.1 首批落地：`BitmapPage` + `Bitmap` + `PixelFormat`（详见 `bitmap_page` 模块头）。
//! Step 7.2 新增：`OutputPage` + `OutlineEntry` + `OcrWord`（PdfWriter trait 三个入参类型）。
//!
//! 详见 `rust-rewrite-plan.md` v2.1 §5.1（v2.1 修订）/ §5.2 / §8.2 / §9.4。

#![forbid(unsafe_code)]

pub mod bitmap_page;
pub mod ocr_word;
pub mod outline_entry;
pub mod output_page;
pub mod word_layout;

pub use bitmap_page::{Bitmap, BitmapError, BitmapPage, PixelFormat};
pub use ocr_word::OcrWord;
pub use outline_entry::OutlineEntry;
pub use output_page::OutputPage;
#[allow(deprecated)]
pub use word_layout::WordLayout;
