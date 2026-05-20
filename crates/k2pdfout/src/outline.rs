//! `outline` —— Step 7.3 (M5) 新建独立 outline 模块。
//!
//! 设计来源：
//! - `rust-rewrite-execution-plan.md` Step 7.3 输出清单要求 `k2pdfout/src/outline.rs`
//! - C 对照：`willuslib/pdfwrite.c:193-263` `pdffile_add_outline` + `willuslib/
//!   wpdfoutline.c:37/48/65` 链表节点 / 添加 / 遍历
//!
//! # 职责
//!
//! 与 [`crate::bitmap_pdf::LopdfWriter::build_outline_tree`]（内部辅助）解耦：本模块
//! 提供"纯算法"工具，把扁平 `Vec<OutlineEntry>` 转换为父子层级结构（HashMap +
//! 拓扑校验），供 LopdfWriter 在 finish 时一次性构建 PDF 对象树用，也可供 GUI /
//! 验证工具复用。
//!
//! # 与 `k2layout::OutlineMapper` 的区别
//!
//! - `OutlineMapper`（k2layout）：在 ConvertContext 内累积 src → dst 映射，是
//!   _状态桶_
//! - `outline`（本模块，k2pdfout）：把已映射好的 entries 投影成 _层级结构_，是
//!   _投影 / 校验_
//!
//! # 主入口
//!
//! - [`OutlineTree::from_entries`]：扁平 Vec → 层级树（含 cycle / out-of-bounds 校验）
//! - [`OutlineTree::children_of`]：取指定 parent 的子 indices 列表
//! - [`OutlineTree::is_empty`]：判断是否需要写入 PDF Outlines 字典

use k2types::OutlineEntry;
use std::collections::HashMap;

/// 错误类型：把扁平 entries 转换为层级树时的校验失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutlineTreeError {
    /// `parent_idx` 指向超出 entries 范围的索引（包括自己 / 后续条目）。
    /// `parent_idx >= entries.len()` 或 `parent_idx >= idx`（必须指向之前的条目）。
    #[error("entry[{idx}] parent_idx={parent_idx} out of bounds or forward-reference (entries={current_len})")]
    InvalidParent {
        /// 出错条目的索引
        idx: usize,
        /// 越界的 parent_idx
        parent_idx: usize,
        /// 当前 entries 长度
        current_len: usize,
    },
}

/// Outline 层级结构（投影自扁平 entries）。
///
/// 由 [`OutlineTree::from_entries`] 构造。内部用 `HashMap<Option<usize>,
/// Vec<usize>>` 索引子条目（key=None 表示顶层）。
#[derive(Debug, Clone)]
pub struct OutlineTree<'a> {
    entries: &'a [OutlineEntry],
    /// children[Some(parent_idx)] = Vec<child_idx>，按 entries 原顺序
    /// children[None] = 顶层条目 indices
    children: HashMap<Option<usize>, Vec<usize>>,
}

impl<'a> OutlineTree<'a> {
    /// 从扁平 `Vec<OutlineEntry>` 构造层级树。
    ///
    /// 校验项：
    /// - 每个 entry 的 `parent_idx` 必须严格小于自己的 index（避免循环 / 前向引用）
    /// - `parent_idx` 必须小于 `entries.len()`
    ///
    /// # 错误
    ///
    /// 任何 invalid parent → [`OutlineTreeError::InvalidParent`]
    pub fn from_entries(entries: &'a [OutlineEntry]) -> Result<Self, OutlineTreeError> {
        let mut children: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
        for (idx, e) in entries.iter().enumerate() {
            if let Some(p) = e.parent_idx {
                // 必须严格小于 idx（避免自引用 / 前向引用 / 越界）
                if p >= idx || p >= entries.len() {
                    return Err(OutlineTreeError::InvalidParent {
                        idx,
                        parent_idx: p,
                        current_len: entries.len(),
                    });
                }
            }
            children.entry(e.parent_idx).or_default().push(idx);
        }
        Ok(Self { entries, children })
    }

    /// 取指定 parent 下的所有直接子条目索引。
    ///
    /// - `parent = None` → 返回顶层条目 indices
    /// - `parent = Some(p)` → 返回 entries[p] 的直接子 indices
    ///
    /// 返回值顺序与 entries 原顺序一致。
    #[must_use]
    pub fn children_of(&self, parent: Option<usize>) -> &[usize] {
        match self.children.get(&parent) {
            Some(v) => v.as_slice(),
            None => &[],
        }
    }

    /// 顶层条目数量（用于 PDF Outlines 字典的 /Count 字段决策）。
    #[must_use]
    pub fn top_level_count(&self) -> usize {
        self.children_of(None).len()
    }

    /// 总条目数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空（不需要写入 PDF Outlines 字典）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 取 entries slice 引用（便于消费方按 index 访问字段）。
    #[must_use]
    pub fn entries(&self) -> &[OutlineEntry] {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn entry(title: &str, parent: Option<usize>) -> OutlineEntry {
        OutlineEntry {
            title: title.into(),
            src_page: -1,
            dst_page: 0,
            parent_idx: parent,
        }
    }

    #[test]
    fn empty_entries_yields_empty_tree() {
        let v: Vec<OutlineEntry> = vec![];
        let t = OutlineTree::from_entries(&v).unwrap();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert_eq!(t.top_level_count(), 0);
        assert!(t.children_of(None).is_empty());
    }

    #[test]
    fn flat_top_level_entries() {
        let v = vec![entry("A", None), entry("B", None), entry("C", None)];
        let t = OutlineTree::from_entries(&v).unwrap();
        assert_eq!(t.top_level_count(), 3);
        assert_eq!(t.children_of(None), &[0, 1, 2]);
        assert!(t.children_of(Some(0)).is_empty());
    }

    #[test]
    fn nested_two_levels() {
        let v = vec![
            entry("A", None),
            entry("A.1", Some(0)),
            entry("A.2", Some(0)),
            entry("B", None),
        ];
        let t = OutlineTree::from_entries(&v).unwrap();
        assert_eq!(t.top_level_count(), 2);
        assert_eq!(t.children_of(None), &[0, 3]);
        assert_eq!(t.children_of(Some(0)), &[1, 2]);
        assert!(t.children_of(Some(3)).is_empty());
    }

    #[test]
    fn three_level_nesting() {
        let v = vec![
            entry("A", None),        // 0
            entry("A.1", Some(0)),   // 1
            entry("A.1.1", Some(1)), // 2
            entry("A.1.2", Some(1)), // 3
            entry("B", None),        // 4
        ];
        let t = OutlineTree::from_entries(&v).unwrap();
        assert_eq!(t.children_of(None), &[0, 4]);
        assert_eq!(t.children_of(Some(0)), &[1]);
        assert_eq!(t.children_of(Some(1)), &[2, 3]);
        assert_eq!(t.children_of(Some(2)), &[] as &[usize]);
    }

    #[test]
    fn invalid_parent_self_reference_fails() {
        // entries[0].parent_idx = Some(0) → 自引用
        let v = vec![entry("x", Some(0))];
        let err = OutlineTree::from_entries(&v).unwrap_err();
        match err {
            OutlineTreeError::InvalidParent {
                idx,
                parent_idx,
                current_len,
            } => {
                assert_eq!(idx, 0);
                assert_eq!(parent_idx, 0);
                assert_eq!(current_len, 1);
            }
        }
    }

    #[test]
    fn invalid_parent_forward_reference_fails() {
        // entries[0].parent_idx = Some(1) → 前向引用
        let v = vec![entry("x", Some(1)), entry("y", None)];
        let err = OutlineTree::from_entries(&v).unwrap_err();
        match err {
            OutlineTreeError::InvalidParent {
                idx, parent_idx, ..
            } => {
                assert_eq!(idx, 0);
                assert_eq!(parent_idx, 1);
            }
        }
    }

    #[test]
    fn invalid_parent_out_of_bounds_fails() {
        let v = vec![entry("x", Some(99))];
        assert!(matches!(
            OutlineTree::from_entries(&v).unwrap_err(),
            OutlineTreeError::InvalidParent { .. }
        ));
    }

    #[test]
    fn entries_accessor() {
        let v = vec![entry("A", None)];
        let t = OutlineTree::from_entries(&v).unwrap();
        assert_eq!(t.entries().len(), 1);
        assert_eq!(t.entries()[0].title, "A");
    }
}
