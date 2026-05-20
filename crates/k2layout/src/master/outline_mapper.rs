//! `outline_mapper` - 书签 / outline 映射桶。
//!
//! 见 [`crate::master`] 模块文档与 `docs/masterinfo-design.md` §2 第 7 行。
//!
//! # C 字段对应
//!
//! 来源：`k2pdfoptlib/k2pdfopt.h:674-742`（MASTERINFO struct 的 outline 字段）
//!
//! | C 字段 | Rust 字段 | C 行号 |
//! |--------|-----------|--------|
//! | `outline_srcpage_completed` | [`OutlineMapper::outline_srcpage_completed`] | 678 |
//! | `outline` (WPDFOUTLINE*) | [`OutlineMapper::entries`] | 680 |
//!
//! # Step 7.2 调整
//!
//! `OutlineEntry` 权威定义已迁到 [`k2types::OutlineEntry`]，本模块从 k2types
//! re-export（字段集与迁移前完全一致：title/src_page/dst_page/parent_idx）。
//! 理由：v2.1 §9.4 要求 PdfWriter trait 入参类型来自 k2types。
//!
//! # Step 7.3 落地范围
//!
//! 本步骤落地 `add_entry`（验证 parent_idx 越界）+ `remap_to_dst`（按 flush_page
//! 时机为 src_page 匹配的未映射条目设置 dst_page）+ `collect_for_writer`（克隆
//! 已映射条目供 PdfWriter 消费）。算法语义与 C 版 `wpdfoutline.c`
//! `wpdfoutline_dstpage_for_srcpage` 等价。

// `OutlineEntry` 权威定义在 k2types crate（Step 7.2 落地），本 crate re-export 之。
pub use k2types::OutlineEntry;

/// Outline 映射桶：维护源 → 输出页的书签映射。
///
/// Step 7.3（M5）落地 add_entry / remap_to_dst 两个核心算法。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineMapper {
    /// outline 条目（扁平存储，parent_idx 表达层级）。
    /// 对应 C `outline` 链表头。
    pub entries: Vec<OutlineEntry>,
    /// 已检查过 outline 的最后一个源页号。
    /// 对应 C `outline_srcpage_completed`。`-1` = 尚未开始检查。
    pub outline_srcpage_completed: i32,
}

impl OutlineMapper {
    /// 构造默认空 OutlineMapper。
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            outline_srcpage_completed: -1,
        }
    }

    /// 添加一个 outline 条目。
    ///
    /// # 行为
    ///
    /// - 若 `entry.parent_idx == Some(p)` 且 `p >= entries.len()`，返回 `Err`
    ///   含错误信息（与 PdfWriter trait 同一类错误对齐）。
    /// - 否则推入 `entries` 并返回新条目的 index。
    ///
    /// 对应 C 版 `wpdfoutline_add_entry`（`wpdfoutline.c:48`）。
    pub fn add_entry(&mut self, entry: OutlineEntry) -> Result<usize, OutlineMapError> {
        if let Some(p) = entry.parent_idx {
            if p >= self.entries.len() {
                return Err(OutlineMapError::ParentOutOfBounds {
                    parent_idx: p,
                    current_len: self.entries.len(),
                });
            }
        }
        let idx = self.entries.len();
        self.entries.push(entry);
        Ok(idx)
    }

    /// 把所有 `src_page == src` 且 `dst_page == -1`（尚未映射）的条目的
    /// `dst_page` 设为 `dst`。
    ///
    /// 对应 C 版 flush_page 时调用 `wpdfoutline_set_dstpage_by_srcpage`
    /// （`wpdfoutline.c` 中的对应循环）。
    ///
    /// # 参数
    ///
    /// - `src_page`：源 PDF 页号（0-based）；负值视作"任意 srcpage"，匹配所有
    ///   未映射的条目（用于强制 flush 兜底）
    /// - `dst_page`：输出 PDF 页号（0-based）
    ///
    /// # 返回
    ///
    /// 重映射的条目数量。
    pub fn remap_to_dst(&mut self, src_page: i32, dst_page: i32) -> usize {
        let mut count = 0usize;
        for e in &mut self.entries {
            if e.dst_page < 0 && (src_page < 0 || e.src_page == src_page) {
                e.dst_page = dst_page;
                count += 1;
            }
        }
        // 更新已处理的 src page sentinel
        if src_page >= 0 && src_page > self.outline_srcpage_completed {
            self.outline_srcpage_completed = src_page;
        }
        count
    }

    /// 收集所有已映射（`dst_page >= 0`）的条目，按 entries 原顺序克隆。
    ///
    /// 用于 ConvertContext 结束时把 outline 发给 PdfWriter。未映射的条目（dst=-1）
    /// 跳过（C 版语义：找不到 dst 的书签不写入输出）。
    #[must_use]
    pub fn collect_for_writer(&self) -> Vec<OutlineEntry> {
        self.entries
            .iter()
            .filter(|e| e.dst_page >= 0)
            .cloned()
            .collect()
    }

    /// 已添加但尚未映射的条目数（dst_page=-1）。
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.entries.iter().filter(|e| e.dst_page < 0).count()
    }
}

impl Default for OutlineMapper {
    fn default() -> Self {
        Self::new()
    }
}

/// Outline 映射算法的错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutlineMapError {
    /// `parent_idx` 指向尚未添加的条目（必须严格小于当前 entries.len()）。
    #[error("outline parent_idx={parent_idx} out of bounds (current entries={current_len})")]
    ParentOutOfBounds {
        /// 越界的 parent_idx
        parent_idx: usize,
        /// 当前 entries 长度
        current_len: usize,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn new_is_empty() {
        let m = OutlineMapper::new();
        assert!(m.entries.is_empty());
        assert_eq!(m.outline_srcpage_completed, -1);
    }

    #[test]
    fn default_eq_new() {
        let a = OutlineMapper::default();
        let b = OutlineMapper::new();
        assert_eq!(a, b);
    }

    #[test]
    fn outline_entry_construct() {
        let e = OutlineEntry {
            title: "Chapter 1".to_string(),
            src_page: 0,
            dst_page: 0,
            parent_idx: None,
        };
        assert_eq!(e.title, "Chapter 1");
        assert_eq!(e.src_page, 0);
        assert_eq!(e.dst_page, 0);
        assert!(e.parent_idx.is_none());
    }

    #[test]
    fn outline_entry_with_parent() {
        let e = OutlineEntry {
            title: "Section 1.1".to_string(),
            src_page: 1,
            dst_page: -1, // 尚未映射
            parent_idx: Some(0),
        };
        assert_eq!(e.parent_idx, Some(0));
        assert_eq!(e.dst_page, -1);
    }

    #[test]
    fn entries_writable() {
        let mut m = OutlineMapper::new();
        m.entries.push(OutlineEntry {
            title: "a".to_string(),
            src_page: 0,
            dst_page: 0,
            parent_idx: None,
        });
        m.outline_srcpage_completed = 5;
        assert_eq!(m.entries.len(), 1);
        assert_eq!(m.outline_srcpage_completed, 5);
    }

    // ---- Step 7.3 add_entry ----

    #[test]
    fn add_entry_top_level_succeeds() {
        let mut m = OutlineMapper::new();
        let idx = m
            .add_entry(OutlineEntry::top_level("A", 0))
            .expect("top-level ok");
        assert_eq!(idx, 0);
        assert_eq!(m.entries.len(), 1);
    }

    #[test]
    fn add_entry_with_valid_parent_succeeds() {
        let mut m = OutlineMapper::new();
        m.add_entry(OutlineEntry::top_level("A", 0)).unwrap();
        let idx = m.add_entry(OutlineEntry::child("A.1", 1, 0)).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(m.entries[1].parent_idx, Some(0));
    }

    #[test]
    fn add_entry_parent_out_of_bounds_fails() {
        let mut m = OutlineMapper::new();
        let err = m
            .add_entry(OutlineEntry::child("orphan", 0, 5))
            .unwrap_err();
        match err {
            OutlineMapError::ParentOutOfBounds {
                parent_idx,
                current_len,
            } => {
                assert_eq!(parent_idx, 5);
                assert_eq!(current_len, 0);
            }
        }
        assert!(m.entries.is_empty());
    }

    // ---- Step 7.3 remap_to_dst ----

    #[test]
    fn remap_to_dst_sets_matching_unmapped() {
        let mut m = OutlineMapper::new();
        // src_page=3, dst=-1
        m.add_entry(OutlineEntry {
            title: "x".into(),
            src_page: 3,
            dst_page: -1,
            parent_idx: None,
        })
        .unwrap();
        // src_page=5, dst=-1
        m.add_entry(OutlineEntry {
            title: "y".into(),
            src_page: 5,
            dst_page: -1,
            parent_idx: None,
        })
        .unwrap();
        let n = m.remap_to_dst(3, 0);
        assert_eq!(n, 1);
        assert_eq!(m.entries[0].dst_page, 0);
        assert_eq!(m.entries[1].dst_page, -1);
        assert_eq!(m.outline_srcpage_completed, 3);
    }

    #[test]
    fn remap_to_dst_skips_already_mapped() {
        let mut m = OutlineMapper::new();
        m.add_entry(OutlineEntry {
            title: "x".into(),
            src_page: 3,
            dst_page: 7, // 已映射
            parent_idx: None,
        })
        .unwrap();
        let n = m.remap_to_dst(3, 0);
        assert_eq!(n, 0);
        assert_eq!(m.entries[0].dst_page, 7); // 不变
    }

    #[test]
    fn remap_to_dst_negative_src_matches_all() {
        let mut m = OutlineMapper::new();
        m.add_entry(OutlineEntry {
            title: "x".into(),
            src_page: 3,
            dst_page: -1,
            parent_idx: None,
        })
        .unwrap();
        m.add_entry(OutlineEntry {
            title: "y".into(),
            src_page: 5,
            dst_page: -1,
            parent_idx: None,
        })
        .unwrap();
        let n = m.remap_to_dst(-1, 10);
        assert_eq!(n, 2);
        assert!(m.entries.iter().all(|e| e.dst_page == 10));
    }

    #[test]
    fn remap_to_dst_updates_completed_sentinel() {
        let mut m = OutlineMapper::new();
        m.remap_to_dst(5, 0);
        assert_eq!(m.outline_srcpage_completed, 5);
        m.remap_to_dst(3, 1); // 3 < 5, 不更新
        assert_eq!(m.outline_srcpage_completed, 5);
        m.remap_to_dst(10, 2);
        assert_eq!(m.outline_srcpage_completed, 10);
    }

    // ---- Step 7.3 collect_for_writer ----

    #[test]
    fn collect_for_writer_skips_unmapped() {
        let mut m = OutlineMapper::new();
        m.add_entry(OutlineEntry {
            title: "mapped".into(),
            src_page: 0,
            dst_page: 0,
            parent_idx: None,
        })
        .unwrap();
        m.add_entry(OutlineEntry {
            title: "unmapped".into(),
            src_page: 1,
            dst_page: -1,
            parent_idx: None,
        })
        .unwrap();
        let v = m.collect_for_writer();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].title, "mapped");
    }

    #[test]
    fn collect_for_writer_preserves_order() {
        let mut m = OutlineMapper::new();
        for (title, dst) in [("A", 0), ("B", 1), ("C", 2)] {
            m.add_entry(OutlineEntry::top_level(title, dst)).unwrap();
        }
        let v = m.collect_for_writer();
        assert_eq!(
            v.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            vec!["A", "B", "C"]
        );
    }

    #[test]
    fn pending_count_tracks_unmapped() {
        let mut m = OutlineMapper::new();
        m.add_entry(OutlineEntry {
            title: "x".into(),
            src_page: 0,
            dst_page: -1,
            parent_idx: None,
        })
        .unwrap();
        m.add_entry(OutlineEntry::top_level("y", 0)).unwrap();
        assert_eq!(m.pending_count(), 1);
        m.remap_to_dst(0, 0);
        assert_eq!(m.pending_count(), 0);
    }
}
