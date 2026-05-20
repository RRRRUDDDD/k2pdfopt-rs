//! `page_state` - 源页面元信息桶。
//!
//! 见 [`crate::master`] 模块文档与 `docs/masterinfo-design.md` §2 第 1 行。
//!
//! # C 字段对应
//!
//! 来源：`k2pdfoptlib/k2pdfopt.h:674-742`（MASTERINFO struct 的元信息字段）
//!
//! | C 字段 | Rust 字段 | C 行号 |
//! |--------|-----------|--------|
//! | `srcfilename[MAXFILENAMELEN]` | [`PageState::srcfilename`] | 676 |
//! | `ocrfilename[MAXFILENAMELEN]` | [`PageState::ocrfilename`] | 677 |
//! | `landscape` | [`PageState::landscape`] | 698 |
//! | `landscape_next` | [`PageState::landscape_next`] | 699 |
//! | `nextpage` | [`PageState::nextpage`] | 700 |
//! | `srcpages` | [`PageState::srcpages`] | 701 |
//! | `bgcolor` | [`PageState::bgcolor`] | 704 |
//! | `fit_to_page` | [`PageState::fit_to_page`] | 705 |
//! | `document_scale_factor` | [`PageState::document_scale_factor`] | 694 |
//! | `debugfolder[256]` | [`PageState::debugfolder`] | 712 |
//! | `rcindex` | [`PageState::rcindex`] | 720 |

use std::path::PathBuf;

/// 源页面 + 输出页面级元信息桶。
///
/// 算法部分在 Step 7.1+（M5）逐步落地，本步骤仅承载字段。
#[derive(Debug, Clone, PartialEq)]
pub struct PageState {
    /// 当前处理的源页号（0-based）。对应 C 版 `srcpage` 局部变量。
    pub source_page: i32,
    /// 源 PDF 文件路径。C 版用 `srcfilename[MAXFILENAMELEN]` 固定大小数组。
    pub srcfilename: PathBuf,
    /// OCR 输出文件路径。C 版用 `ocrfilename[MAXFILENAMELEN]`。
    pub ocrfilename: PathBuf,
    /// 当前页是否横向。对应 C `landscape`（v2.32 起由 `masterinfo_new_source_page_init` 维护）。
    pub landscape: bool,
    /// 下一页是否横向。对应 C `landscape_next`。
    pub landscape_next: bool,
    /// 下一个待处理页号。对应 C `nextpage`。
    pub nextpage: i32,
    /// 源 PDF 总页数。对应 C `srcpages`。
    pub srcpages: i32,
    /// 背景色（0-255，常为 255 白）。对应 C `bgcolor`。
    pub bgcolor: u8,
    /// 是否启用 fit-to-page 模式。对应 C `fit_to_page`（非零表示启用）。
    pub fit_to_page: bool,
    /// 读源 bitmap 时的 dpi 缩放因子。对应 C `document_scale_factor`。
    pub document_scale_factor: f64,
    /// 调试 dump 目录。C 版用 `debugfolder[256]` 固定大小数组。
    pub debugfolder: PathBuf,
    /// Row color index（v2.55 字段）。对应 C `rcindex`。
    pub rcindex: i32,
}

impl PageState {
    /// 构造默认空 PageState（对应 C 版 `masterinfo_init` 后的 zero state）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            source_page: 0,
            srcfilename: PathBuf::new(),
            ocrfilename: PathBuf::new(),
            landscape: false,
            landscape_next: false,
            nextpage: 0,
            srcpages: 0,
            bgcolor: 255,
            fit_to_page: false,
            document_scale_factor: 1.0,
            debugfolder: PathBuf::new(),
            rcindex: 0,
        }
    }
}

impl Default for PageState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults_to_white_bg_and_zero_pages() {
        let p = PageState::new();
        assert_eq!(p.source_page, 0);
        assert_eq!(p.bgcolor, 255);
        assert!(!p.landscape);
        assert!(!p.fit_to_page);
        assert!((p.document_scale_factor - 1.0).abs() < f64::EPSILON);
        assert_eq!(p.srcpages, 0);
        assert_eq!(p.nextpage, 0);
        assert_eq!(p.rcindex, 0);
        assert!(p.srcfilename.as_os_str().is_empty());
        assert!(p.ocrfilename.as_os_str().is_empty());
        assert!(p.debugfolder.as_os_str().is_empty());
    }

    #[test]
    fn default_equals_new() {
        let a = PageState::default();
        let b = PageState::new();
        assert_eq!(a, b);
    }

    #[test]
    fn fields_writable() {
        let mut p = PageState::new();
        p.source_page = 5;
        p.srcpages = 100;
        p.landscape = true;
        p.bgcolor = 0;
        assert_eq!(p.source_page, 5);
        assert_eq!(p.srcpages, 100);
        assert!(p.landscape);
        assert_eq!(p.bgcolor, 0);
    }
}
