//! `output_paginator` - 输出页队列 + 分页 marks + autocrop margins 桶。
//!
//! 见 [`crate::master`] 模块文档与 `docs/masterinfo-design.md` §2 第 5 行。
//!
//! # C 字段对应
//!
//! 来源：`k2pdfoptlib/k2pdfopt.h:674-742`（MASTERINFO struct 的分页 / 输出字段）
//!
//! | C 字段 | Rust 字段 | C 行号 |
//! |--------|-----------|--------|
//! | `outfile` (PDFFILE) | (略——M5 由 k2pdfout crate 承载) | 679 |
//! | `queued_page_info` (QUEUED_PAGE_INFO) | [`OutputPaginator::queued_pages`] | 685 |
//! | `k2pagebreakmarks` (K2PAGEBREAKMARKS) | [`OutputPaginator::pagebreak_marks`] | 687 |
//! | `published_pages` | [`OutputPaginator::published_pages`] | 703 |
//! | `output_page_count` (v2.42) | [`OutputPaginator::output_page_count`] | 708 |
//! | `filecount` (v2.42) | [`OutputPaginator::filecount`] | 709 |
//! | `autocrop_margins[4]` (v2.42) | [`OutputPaginator::autocrop_margins`] | 710 |
//! | `wordcount` | [`OutputPaginator::wordcount`] | 706 |
//!
//! # Step 7.1 (M5) 新增
//!
//! - [`OutputPaginator::add_pagebreak_mark`] 落地（C `masterinfo_add_pagebreakmark`，
//!   `k2master.c:394-409`）
//! - 重新导出 [`crate::breakpoints`] 的 `MARK_TYPE_*` / `MAX_PAGE_BREAK_MARKS` 常量
//!
//! `push_page` / `pop_page` 仍是 Step 7.3 的占位（M5）。
//!
//! # Step 7.2 (M5) 调整
//!
//! 原 `OutputPage` 重命名为 [`PaginatorPage`]，避免与 v2.1 §9.4 要求的
//! [`k2types::OutputPage`]（PdfWriter trait 入参）同名冲突。
//!
//! 语义区分：
//! - [`PaginatorPage`]: master canvas → flush queue 的内部临时形态（5 字段 +
//!   pixel format + srcpage_dst_index，不含 output DPI）
//! - [`k2types::OutputPage`]: flush queue → PdfWriter 的最终形态（含 Bitmap +
//!   DPI + JPEG 控制）
//!
//! Step 7.3 串联 `ConvertContext::flush_page` 时由 settings 合并完成转换。
//!
//! # Step 7.3 落地范围
//!
//! 本步骤落地 [`OutputPaginator::push_page`]（FIFO append）+ [`pop_page`]（FIFO
//! pop_front）+ [`PaginatorPage::format`]（携带 pixel format 字段，避免下游推断）。

use crate::breakpoints::{MARK_TYPE_BREAKPAGE, MARK_TYPE_NOBREAK, MAX_PAGE_BREAK_MARKS};
use k2types::{OcrWord, PixelFormat};

/// 单页输出（已从 master canvas flush 出，等待写入 PDF）的**内部临时形态**。
///
/// 对应 C 版 `QUEUED_PAGE` struct（`k2pdfopt.h:656-660`）。Step 7.3 串联时
/// 由 settings 提供 DPI/JPEG 参数转换为 [`k2types::OutputPage`] 喂给 PdfWriter。
///
/// # Step 9.3 新增
///
/// [`PaginatorPage::ocr_words`]：本页对应的 OCR words 列表（坐标系：page bitmap
/// 局部，y 顶部原点 top-left）。`ConvertContext::flush_page` 在切出 `rowcount` 行
/// 后调 `ocr.drain_in_range(0, rowcount)` 把本页归属词填到此字段；
/// `ConvertJob::flush_queue_to_writer` 在 `add_page` 后用
/// `apply_ocr_words_to_writer` 写入不可见文字层。
#[derive(Debug, Clone, PartialEq)]
pub struct PaginatorPage {
    /// 输出页索引（0-based）。
    pub page_index: u32,
    /// 输出页源页号（C 版 `srcpageno`）。`-1` = 来源不可追溯（合并 / cover）
    pub srcpageno: i32,
    /// bitmap 宽度（pixel）。
    pub width: u32,
    /// bitmap 高度（pixel）。
    pub height: u32,
    /// 像素布局（Gray8/Rgb8/Rgba8）。Step 7.3 新增以承载 k2types::PixelFormat。
    pub format: PixelFormat,
    /// bitmap 像素数据（长度 = `width * height * format.bytes_per_pixel()`）。
    pub pixels: Vec<u8>,
    /// 本页对应的 OCR words（Step 9.3 新增）。
    ///
    /// 当不开 OCR 时保持空 Vec；不影响写盘性能。
    pub ocr_words: Vec<OcrWord>,
}

/// 用户指定的强制分页标记。
///
/// 对应 C 版 `K2PAGEBREAKMARK`（`k2pdfopt.h:221-226`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageBreakMark {
    /// 分页发生在 master canvas 的哪一行（C `row`，绝对坐标）。
    pub row: u32,
    /// 分页类型（C `type`）。值见 [`crate::breakpoints::MARK_TYPE_BREAKPAGE`] 等。
    pub mark_type: i32,
}

/// 输出分页器桶：管理已 flush 的页队列、分页 marks、autocrop margins。
///
/// 算法部分（publish / advance / autocrop apply）在 Step 7.1/7.3+（M5）落地。
#[derive(Debug, Clone, PartialEq)]
pub struct OutputPaginator {
    /// 已 flush 到队列的输出页（等待写入 PDF）。
    /// 对应 C `queued_page_info.page[]`。
    pub queued_pages: Vec<PaginatorPage>,
    /// 用户指定的强制分页标记。
    /// 对应 C `k2pagebreakmarks`。
    pub pagebreak_marks: Vec<PageBreakMark>,
    /// 已发布（写入 PDF）的页数。对应 C `published_pages`。
    pub published_pages: u32,
    /// 输出 PDF 中累计页数（含 cover 等）。对应 C `output_page_count`（v2.42）。
    pub output_page_count: u32,
    /// 输出文件计数（分卷场景）。对应 C `filecount`（v2.42）。
    pub filecount: u32,
    /// Autocrop margins (left/top/right/bottom)。对应 C `autocrop_margins[4]`（v2.42）。
    pub autocrop_margins: [i32; 4],
    /// 累计 word 数。对应 C `wordcount`。
    pub wordcount: u32,
}

impl OutputPaginator {
    /// 构造默认空 OutputPaginator。
    #[must_use]
    pub fn new() -> Self {
        Self {
            queued_pages: Vec::new(),
            pagebreak_marks: Vec::new(),
            published_pages: 0,
            output_page_count: 0,
            filecount: 0,
            autocrop_margins: [0; 4],
            wordcount: 0,
        }
    }

    /// 把一页 flush 到队列尾部（FIFO）。
    ///
    /// 对应 C `masterinfo_pagequeue_queue_page`（`k2master.c:127-160`）。
    /// 同时累加 `output_page_count`。
    pub fn push_page(&mut self, page: PaginatorPage) {
        self.queued_pages.push(page);
        self.output_page_count = self.output_page_count.saturating_add(1);
    }

    /// 取出队首页（已发布），FIFO 顺序。
    ///
    /// 对应 C `masterinfo_pagequeue_pop_queue`（`k2master.c:1181-1194`）。
    /// 同时累加 `published_pages`。
    pub fn pop_page(&mut self) -> Option<PaginatorPage> {
        if self.queued_pages.is_empty() {
            return None;
        }
        let page = self.queued_pages.remove(0);
        self.published_pages = self.published_pages.saturating_add(1);
        Some(page)
    }

    /// 队列内还有未发布的页数。
    #[must_use]
    pub fn queued_len(&self) -> usize {
        self.queued_pages.len()
    }

    /// 在 master canvas 当前行（`canvas_rows`）记录一个 pagebreak mark。
    ///
    /// 对应 C `masterinfo_add_pagebreakmark`（`k2master.c:394-409`）：
    /// - 若 marks 数已达 [`MAX_PAGE_BREAK_MARKS`] 上限，丢弃（C 同样静默丢弃）
    /// - 否则追加 `{row: canvas_rows, mark_type}` 到 marks 末尾
    ///
    /// `mark_type` 通常取 [`crate::breakpoints::MARK_TYPE_BREAKPAGE`] 或
    /// [`crate::breakpoints::MARK_TYPE_NOBREAK`]。
    ///
    /// # 返回
    ///
    /// `true` 表示已记录，`false` 表示已达上限被丢弃。
    pub fn add_pagebreak_mark(&mut self, canvas_rows: u32, mark_type: i32) -> bool {
        if self.pagebreak_marks.len() >= MAX_PAGE_BREAK_MARKS {
            return false;
        }
        self.pagebreak_marks.push(PageBreakMark {
            row: canvas_rows,
            mark_type,
        });
        true
    }

    /// 便捷方法：在 `canvas_rows` 处加一条 BREAKPAGE 类型 mark。
    pub fn add_breakpage_mark(&mut self, canvas_rows: u32) -> bool {
        self.add_pagebreak_mark(canvas_rows, MARK_TYPE_BREAKPAGE)
    }

    /// 便捷方法：在 `canvas_rows` 处加一条 NOBREAK 类型 mark。
    pub fn add_nobreak_mark(&mut self, canvas_rows: u32) -> bool {
        self.add_pagebreak_mark(canvas_rows, MARK_TYPE_NOBREAK)
    }
}

impl Default for OutputPaginator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::breakpoints::{MARK_TYPE_BREAKPAGE, MARK_TYPE_DISABLED, MARK_TYPE_NOBREAK};

    #[test]
    fn new_paginator_is_empty() {
        let p = OutputPaginator::new();
        assert!(p.queued_pages.is_empty());
        assert!(p.pagebreak_marks.is_empty());
        assert_eq!(p.published_pages, 0);
        assert_eq!(p.output_page_count, 0);
        assert_eq!(p.filecount, 0);
        assert_eq!(p.autocrop_margins, [0; 4]);
        assert_eq!(p.wordcount, 0);
    }

    #[test]
    fn default_eq_new() {
        let a = OutputPaginator::default();
        let b = OutputPaginator::new();
        assert_eq!(a, b);
    }

    #[test]
    fn pagebreak_mark_copyable() {
        let m1 = PageBreakMark {
            row: 100,
            mark_type: MARK_TYPE_BREAKPAGE,
        };
        let m2 = m1;
        assert_eq!(m1, m2);
    }

    #[test]
    fn paginator_page_construct() {
        let page = PaginatorPage {
            page_index: 5,
            srcpageno: 3,
            width: 800,
            height: 1200,
            format: PixelFormat::Gray8,
            pixels: vec![255; 800 * 1200],
            ocr_words: Vec::new(),
        };
        assert_eq!(page.page_index, 5);
        assert_eq!(page.srcpageno, 3);
        assert_eq!(page.width, 800);
        assert_eq!(page.height, 1200);
        assert_eq!(page.format, PixelFormat::Gray8);
        assert_eq!(page.pixels.len(), 960_000);
        assert!(page.ocr_words.is_empty());
    }

    #[test]
    fn autocrop_margins_writable() {
        let mut p = OutputPaginator::new();
        p.autocrop_margins = [10, 20, 30, 40];
        assert_eq!(p.autocrop_margins, [10, 20, 30, 40]);
    }

    // ---- Step 7.1 add_pagebreak_mark ----

    #[test]
    fn add_pagebreak_mark_appends_to_marks() {
        let mut p = OutputPaginator::new();
        assert!(p.add_pagebreak_mark(100, MARK_TYPE_BREAKPAGE));
        assert_eq!(p.pagebreak_marks.len(), 1);
        assert_eq!(p.pagebreak_marks[0].row, 100);
        assert_eq!(p.pagebreak_marks[0].mark_type, MARK_TYPE_BREAKPAGE);
    }

    #[test]
    fn add_pagebreak_mark_returns_false_when_full() {
        let mut p = OutputPaginator::new();
        for i in 0..MAX_PAGE_BREAK_MARKS {
            assert!(
                p.add_pagebreak_mark(i as u32, MARK_TYPE_BREAKPAGE),
                "mark #{i} 应当成功"
            );
        }
        assert_eq!(p.pagebreak_marks.len(), MAX_PAGE_BREAK_MARKS);
        // 再加一个 → 应失败
        assert!(!p.add_pagebreak_mark(999, MARK_TYPE_BREAKPAGE));
        assert_eq!(
            p.pagebreak_marks.len(),
            MAX_PAGE_BREAK_MARKS,
            "上限达到后不应继续累积"
        );
    }

    #[test]
    fn add_breakpage_and_nobreak_helpers_work() {
        let mut p = OutputPaginator::new();
        assert!(p.add_breakpage_mark(50));
        assert!(p.add_nobreak_mark(100));
        assert_eq!(p.pagebreak_marks.len(), 2);
        assert_eq!(p.pagebreak_marks[0].mark_type, MARK_TYPE_BREAKPAGE);
        assert_eq!(p.pagebreak_marks[1].mark_type, MARK_TYPE_NOBREAK);
    }

    #[test]
    fn add_disabled_mark_explicitly_allowed() {
        // 用户也可以直接添加 disabled mark（虽然实际场景中由 apply 函数翻转产生）
        let mut p = OutputPaginator::new();
        assert!(p.add_pagebreak_mark(10, MARK_TYPE_DISABLED));
        assert_eq!(p.pagebreak_marks[0].mark_type, MARK_TYPE_DISABLED);
    }

    // ---- Step 7.3 push_page / pop_page ----

    fn make_page(idx: u32) -> PaginatorPage {
        PaginatorPage {
            page_index: idx,
            srcpageno: idx as i32,
            width: 10,
            height: 10,
            format: PixelFormat::Gray8,
            pixels: vec![255; 100],
            ocr_words: Vec::new(),
        }
    }

    #[test]
    fn push_page_appends_and_increments_output_count() {
        let mut p = OutputPaginator::new();
        p.push_page(make_page(0));
        p.push_page(make_page(1));
        assert_eq!(p.queued_len(), 2);
        assert_eq!(p.output_page_count, 2);
        assert_eq!(p.published_pages, 0); // pop 才增
    }

    #[test]
    fn pop_page_fifo_and_increments_published() {
        let mut p = OutputPaginator::new();
        p.push_page(make_page(0));
        p.push_page(make_page(1));
        let first = p.pop_page().expect("first");
        assert_eq!(first.page_index, 0);
        assert_eq!(p.queued_len(), 1);
        assert_eq!(p.published_pages, 1);
        let second = p.pop_page().expect("second");
        assert_eq!(second.page_index, 1);
        assert_eq!(p.queued_len(), 0);
        assert_eq!(p.published_pages, 2);
        assert!(p.pop_page().is_none());
    }

    #[test]
    fn pop_page_empty_returns_none() {
        let mut p = OutputPaginator::new();
        assert!(p.pop_page().is_none());
        assert_eq!(p.published_pages, 0);
    }
}
