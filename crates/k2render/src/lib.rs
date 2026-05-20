//! `k2render` - 文档渲染抽象与多后端。
//!
//! 来源 C 文件：`bmpmupdf.c`、`wmupdf.c`、`wmupdfinfo.c`、`bmpdjvu.c`、`wgs.c`。
//!
//! M2 MVP 后端为 mutool（默认 stdout PAM 管道，ADR-015）；M8+ 起接 MuPDF FFI / Ghostscript /
//! DjVu，[`DocumentRenderer`] trait 隔离实现切换。
//!
//! 详见 `rust-rewrite-plan.md` v2.1 §5.2 / §9.2 / §10 M2 + ADR-015。
//! Step 4.1 首批落地：`DocumentRenderer` trait + `RenderError` + `MutoolRenderer`。
//! Step 4.2 追加：[`PdfInfo`] 元信息提取（`mutool info` 文本解析）。

#![forbid(unsafe_code)]

pub mod mutool;
pub mod pdfinfo;
pub mod renderer;

pub use mutool::{MutoolOptions, MutoolRenderer};
pub use pdfinfo::{PdfInfo, PdfInfoOptions};
pub use renderer::{DocumentRenderer, RenderError};
