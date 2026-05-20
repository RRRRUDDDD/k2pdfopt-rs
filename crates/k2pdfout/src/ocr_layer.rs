//! `ocr_layer` - OCR 不可见文字层写入薄包装（Step 9.3 落地）。
//!
//! 这一模块是 [`crate::PdfWriter::add_ocr_layer`] 的高层语义入口，
//! 主要价值：
//!
//! - 给 pipeline 一个 **稳定的高层 API**（屏蔽 trait 方法变更）
//! - 集中错误转换（`PdfWriteError` → [`OcrLayerError`]）
//! - 空 word 列表短路 + 调试日志钩子（未来扩展）
//! - 未来 visibility flags（C `dst_ocr_visibility_flags` bit mask：show source / show OCR text /
//!   show boxes）的统一执行点
//!
//! # 来源 C
//!
//! 与 `willuslib/pdfwrite.c::ocrwords_to_pdf_stream`（~1606 行起）行为对齐：
//! 把一组 `OcrWord` 转换为 PDF 内容流的 `BT ... 3 Tr ... Tj ... ET` 字节并附加到当前页 Contents。
//!
//! # Step 9.3 落地范围（最小集）
//!
//! 仅做薄包装 + 错误转换。**完整 visibility flags 决策树**（C `dst_ocr_visibility_flags`
//! bit mask 0x01/0x02/0x04/0x08/0x10）延后到 Step 10.x release 前补全。
//!
//! 详见 `rust-rewrite-execution-plan.md` Step 9.3。

use crate::PdfWriter;
use k2types::OcrWord;

/// OCR 文字层写入错误。
#[derive(Debug, thiserror::Error)]
pub enum OcrLayerError {
    /// `apply_ocr_words_to_writer` 在 `add_page` 之前调用 → `add_ocr_layer`
    /// 内部返 `PdfWriteError::OcrBeforePage`。
    #[error("OCR layer write failed (no prior add_page or writer rejected words): {0}")]
    WriterRejected(#[source] anyhow::Error),
}

/// 把一组 OCR words 写入 writer 当前页的不可见文字层。
///
/// # 调用约定
///
/// - 必须在 [`PdfWriter::add_page`] 之后调用（writer 内部追踪 "最近一页"）
/// - `words` 为空时短路 `Ok(())`（不调 writer，零开销）
/// - 多次调用累加到同一页（与 trait 语义一致）
///
/// # 坐标系
///
/// `words` 中每个 [`OcrWord`] 的 `(x, y)` 应当是**当前页 bitmap 局部坐标**
/// （像素，top-left 原点；y 向下增长）。`y` 是 word 矩形顶部 y（Step 9.2 校正后的语义）。
/// [`PdfWriter`] 实现内部用 `y_bottom() = y + h` 转换到 PDF baseline，
/// 详见 `bitmap_pdf::build_ocr_content_stream`。
///
/// # 错误
///
/// - [`OcrLayerError::WriterRejected`]：writer 返 `add_ocr_layer` 错误
///   （如 `OcrBeforePage` / lopdf 内部异常）
pub fn apply_ocr_words_to_writer(
    writer: &mut dyn PdfWriter,
    words: &[OcrWord],
) -> Result<(), OcrLayerError> {
    if words.is_empty() {
        return Ok(());
    }
    writer
        .add_ocr_layer(words)
        .map_err(OcrLayerError::WriterRejected)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::PdfWriter;
    use anyhow::anyhow;
    use k2types::{OcrWord, OutlineEntry, OutputPage};

    /// 一个最小可控的 PdfWriter mock，记录每次调用。
    struct MockWriter {
        page_added: bool,
        ocr_calls: Vec<usize>,
        fail_next_ocr: bool,
    }

    impl MockWriter {
        fn new() -> Self {
            Self {
                page_added: false,
                ocr_calls: Vec::new(),
                fail_next_ocr: false,
            }
        }
    }

    impl PdfWriter for MockWriter {
        fn add_page(&mut self, _page: &OutputPage) -> anyhow::Result<()> {
            self.page_added = true;
            Ok(())
        }
        fn add_outline(&mut self, _entry: OutlineEntry) -> anyhow::Result<()> {
            Ok(())
        }
        fn add_ocr_layer(&mut self, words: &[OcrWord]) -> anyhow::Result<()> {
            if self.fail_next_ocr {
                self.fail_next_ocr = false;
                return Err(anyhow!("simulated writer failure"));
            }
            if !self.page_added {
                return Err(anyhow!("add_ocr_layer called before any add_page"));
            }
            self.ocr_calls.push(words.len());
            Ok(())
        }
        fn finish(self: Box<Self>) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn empty_words_short_circuits() {
        let mut w = MockWriter::new();
        // 即使 page 未 add，empty 输入也是 Ok（不调用 writer）
        let res = apply_ocr_words_to_writer(&mut w, &[]);
        assert!(res.is_ok());
        assert!(w.ocr_calls.is_empty());
    }

    #[test]
    fn non_empty_words_forwarded_after_page() {
        let mut w = MockWriter::new();
        w.page_added = true; // 模拟已有一页
        let words = vec![
            OcrWord::new("hello", 10.0, 20.0, 50.0, 12.0),
            OcrWord::new("world", 70.0, 20.0, 50.0, 12.0),
        ];
        let res = apply_ocr_words_to_writer(&mut w, &words);
        assert!(res.is_ok());
        assert_eq!(w.ocr_calls, vec![2]);
    }

    #[test]
    fn writer_failure_propagates_as_writer_rejected() {
        let mut w = MockWriter::new();
        w.page_added = true;
        w.fail_next_ocr = true;
        let words = vec![OcrWord::new("x", 0.0, 0.0, 1.0, 1.0)];
        let err = apply_ocr_words_to_writer(&mut w, &words).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("OCR layer write failed"));
    }

    #[test]
    fn before_page_writer_returns_error() {
        let mut w = MockWriter::new();
        // page 未 add → writer 返错
        let words = vec![OcrWord::new("x", 0.0, 0.0, 1.0, 1.0)];
        let err = apply_ocr_words_to_writer(&mut w, &words).unwrap_err();
        let chain = format!("{}", err);
        assert!(chain.contains("OCR layer write failed"));
    }

    #[test]
    fn multiple_calls_accumulate() {
        let mut w = MockWriter::new();
        w.page_added = true;
        let words1 = vec![OcrWord::new("a", 0.0, 0.0, 5.0, 5.0)];
        let words2 = vec![
            OcrWord::new("b", 0.0, 0.0, 5.0, 5.0),
            OcrWord::new("c", 0.0, 0.0, 5.0, 5.0),
        ];
        apply_ocr_words_to_writer(&mut w, &words1).unwrap();
        apply_ocr_words_to_writer(&mut w, &words2).unwrap();
        assert_eq!(w.ocr_calls, vec![1, 2]);
    }

    #[test]
    fn empty_words_with_pre_page_state_is_still_ok() {
        let mut w = MockWriter::new();
        w.page_added = true;
        apply_ocr_words_to_writer(&mut w, &[]).unwrap();
        // 仍未调用 writer
        assert!(w.ocr_calls.is_empty());
    }
}
