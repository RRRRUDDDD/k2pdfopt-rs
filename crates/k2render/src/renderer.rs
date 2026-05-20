//! `DocumentRenderer` trait + 通用渲染错误类型。
//!
//! 详见 `rust-rewrite-plan.md` v2.1 §9.2（trait 契约）+ ADR-008（错误模型分层）+
//! ADR-015（mutool stdout PAM 决策）。
//!
//! `trait` 签名固定为 `anyhow::Result<T>`（与 §9.2 一致），typed `RenderError`
//! 通过 `From<RenderError> for anyhow::Error` 注入，调用方可 `downcast_ref` 取回。

use k2types::BitmapPage;

/// 文档渲染抽象。M2 MVP 仅 [`crate::MutoolRenderer`]；M8+ 引入 `MupdfFfiRenderer` 等。
pub trait DocumentRenderer: Send {
    /// 返回文档页数。
    fn page_count(&self) -> anyhow::Result<usize>;
    /// 返回指定页的原始物理尺寸 `(width_pt, height_pt)`，1 pt = 1/72 inch。
    fn page_size(&self, page_index: usize) -> anyhow::Result<(f32, f32)>;
    /// 渲染指定页为 [`BitmapPage`]（`dpi` 为输出栅格分辨率）。
    fn render_page(&self, page_index: usize, dpi: f32) -> anyhow::Result<BitmapPage>;
}

/// 渲染过程的 typed 错误。`anyhow::Error` 路径上可
/// `downcast_ref::<RenderError>()` 取回结构化信息。
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// 找不到渲染器二进制（mutool 等）。
    #[error("renderer binary `{0}` not found in PATH")]
    BinaryNotFound(String),
    /// 调用方传入的页号超出 `[0, page_count)`。
    #[error("page index {requested} out of range (page count = {total})")]
    PageOutOfRange { requested: usize, total: usize },
    /// PDF 加密，需要密码。
    #[error(
        "source document `{path}` is encrypted; provide password via MutoolRenderer::with_options"
    )]
    Encrypted { path: String },
    /// 源文件损坏或不可解析。
    #[error("source document is invalid: {0}")]
    InvalidSource(String),
    /// 子进程退出码非零且非加密类错误。
    #[error("renderer exited with code {code}: {stderr}")]
    SubprocessFailed { code: i32, stderr: String },
    /// PAM 数据流头部异常。
    #[error("malformed PAM stream: {0}")]
    InvalidPam(String),
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn render_error_downcast_via_anyhow() {
        let err: anyhow::Error = RenderError::PageOutOfRange {
            requested: 5,
            total: 3,
        }
        .into();
        let typed = err.downcast_ref::<RenderError>().unwrap();
        match typed {
            RenderError::PageOutOfRange { requested, total } => {
                assert_eq!(*requested, 5);
                assert_eq!(*total, 3);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn render_error_display_contains_context() {
        let e = RenderError::BinaryNotFound("mutool".into());
        assert!(format!("{e}").contains("mutool"));
    }
}
