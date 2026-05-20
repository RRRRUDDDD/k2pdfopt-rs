//! `OutlineEntry` —— PDF 书签（outline）的扁平存储节点。
//!
//! 设计来源：
//! - `rust-rewrite-plan.md` v2.1 §9.4（`PdfWriter::add_outline(OutlineEntry)`）
//! - `rust-rewrite-execution-plan.md` Step 7.2（本步骤）
//! - C 对照：`willuslib/wpdfoutline.c` `WPDFOUTLINE` 链表节点
//!
//! # 表达模型选择
//!
//! C 版用链表 + parent / down / next 三向指针表达 outline 树。Rust 版选择**扁平
//! `Vec` 加 `parent_idx`**（与 `k2layout::outline_mapper::OutlineEntry` 一致，避免
//! 引入循环引用 / `Rc<RefCell>`）。PdfWriter 内部按 `parent_idx` 重建树结构后再
//! 生成 PDF outline 对象树（First / Last / Next / Prev / Parent 五指针）。
//!
//! Step 5.6 落地的 `k2layout::OutlineEntry` 与本类型字段完全一致——本步 Step 7.2
//! 把权威定义移到 k2types，`k2layout` 改为 re-export 本类型（消除重复定义）。

/// 单个 outline 条目（书签项）。
///
/// 对应 C 版 `WPDFOUTLINE`（`willuslib/wpdfoutline.c:37`，5 字段：title/srcpage/
/// dstpage/down/next）的单节点。Rust 用 `parent_idx` 替代 down/next 双指针，
/// 树结构由 `Vec<OutlineEntry>` 整体承载。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// outline 标题（书签显示文本）。对应 C `WPDFOUTLINE.title`。
    pub title: String,
    /// 源 PDF 页号（0-based）。对应 C `WPDFOUTLINE.srcpage`。
    pub src_page: i32,
    /// 输出 PDF 页号（0-based）。对应 C `WPDFOUTLINE.dstpage`。
    /// `-1` = 尚未映射到输出页（典型在 flush_page 串联前的中间状态）。
    pub dst_page: i32,
    /// 父条目在同一 `Vec<OutlineEntry>` 内的索引。
    /// `None` = 顶层条目。对应 C 链表的 `parent` 指针。
    pub parent_idx: Option<usize>,
}

impl OutlineEntry {
    /// 用最少字段构造一个顶层条目：title + dst_page；src_page=-1, parent_idx=None。
    #[must_use]
    pub fn top_level<S: Into<String>>(title: S, dst_page: i32) -> Self {
        Self {
            title: title.into(),
            src_page: -1,
            dst_page,
            parent_idx: None,
        }
    }

    /// 用最少字段构造一个子条目：title + dst_page + parent_idx；src_page=-1。
    #[must_use]
    pub fn child<S: Into<String>>(title: S, dst_page: i32, parent_idx: usize) -> Self {
        Self {
            title: title.into(),
            src_page: -1,
            dst_page,
            parent_idx: Some(parent_idx),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn top_level_constructor() {
        let e = OutlineEntry::top_level("Chapter 1", 0);
        assert_eq!(e.title, "Chapter 1");
        assert_eq!(e.src_page, -1);
        assert_eq!(e.dst_page, 0);
        assert!(e.parent_idx.is_none());
    }

    #[test]
    fn child_constructor() {
        let e = OutlineEntry::child("Section 1.1", 1, 0);
        assert_eq!(e.title, "Section 1.1");
        assert_eq!(e.dst_page, 1);
        assert_eq!(e.parent_idx, Some(0));
    }

    #[test]
    fn full_struct_construction() {
        let e = OutlineEntry {
            title: "前言".to_string(),
            src_page: 5,
            dst_page: 3,
            parent_idx: Some(2),
        };
        assert_eq!(e.title, "前言");
        assert_eq!(e.src_page, 5);
        assert_eq!(e.dst_page, 3);
        assert_eq!(e.parent_idx, Some(2));
    }

    #[test]
    fn eq_and_clone() {
        let e1 = OutlineEntry::top_level("A", 0);
        let e2 = e1.clone();
        assert_eq!(e1, e2);
        let e3 = OutlineEntry::top_level("B", 0);
        assert_ne!(e1, e3);
    }
}
