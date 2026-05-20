//! `ocr_staging` - OCR words 在 master canvas 坐标系暂存桶。
//!
//! 见 [`crate::master`] 模块文档与 `docs/masterinfo-design.md` §2 第 6 行。
//!
//! # C 字段对应
//!
//! 来源：`k2pdfoptlib/k2pdfopt.h:674-742`（MASTERINFO struct 的 OCR 字段）
//!
//! | C 字段 | Rust 字段 | C 行号 |
//! |--------|-----------|--------|
//! | `mi_ocrwords` (OCRWORDS) | [`OcrStaging::words`] | 683 |
//!
//! # Step 7.2 调整
//!
//! `OcrWord` 权威定义已迁到 [`k2types::OcrWord`]，本模块从 k2types re-export
//! （字段集与迁移前完全一致 + 手动 PartialEq）。理由：v2.1 §9.4 要求 PdfWriter
//! trait 入参类型来自 k2types，避免 `k2pdfout` 依赖 `k2layout`。
//!
//! # Step 9.2 落地
//!
//! 把 [`OcrStaging::offset_words`] / [`OcrStaging::drain_in_range`] 由占位
//! `unimplemented!()` 改为 1:1 复刻 C 版 master canvas OCR pipeline。
//! 新增 [`OcrStaging::concatenate`] / [`OcrStaging::sort_by_position`] /
//! [`OcrStaging::clear`] 与 C `ocrwords_concatenate` / `ocrwords_sort_by_position`
//! / `ocrwords_clear` 对齐。底层算法委托给 [`k2ocr::mapping`]，本桶仅做容器
//! 语义封装（add / drain / offset）。
//!
//! 关键 C 路径：
//!
//! | C 调用点 | Rust 等价 | 说明 |
//! |----------|-----------|------|
//! | `k2master.c:744 ocrwords_offset(words, dw, masterinfo->rows+gap_start)` | [`OcrStaging::offset_words`] | region 局部 → master 全局坐标 |
//! | `k2master.c:763 ocrwords_concatenate(&masterinfo->mi_ocrwords, words)` | [`OcrStaging::concatenate`] | region words 入 master 桶 |
//! | `k2master.c:1243 ocrwords_offset(&masterinfo->mi_ocrwords, 0, -rows)` | [`OcrStaging::offset_y`] | 页 flush 后剩余 words 上移 |
//! | `k2master.c:1535-1544 r-maxheight+h/2 < rowcount` 选词 | [`OcrStaging::drain_in_range`] | flush 出页时按中线选词 |
//! | `k2master.c:1582 ocrwords_sort_by_position(...)` | [`OcrStaging::sort_by_position`] | 输出前按位置排序 |

// `OcrWord` 权威定义在 k2types crate（Step 7.2 落地），本 crate re-export 之。
pub use k2types::OcrWord;

/// OCR words 暂存桶。
///
/// 添加 bitmap 到 master canvas 时，OCR words 的 y 坐标需要随之 offset；
/// flush 页时，已发布页的 words 需要从队列中取出并交给 PDF writer。
///
/// 坐标系：[`OcrWord::y`] 是 word 矩形**顶部** y（image top-left 原点，y 向下增）；
/// 与 C `OCRWORD.r`（C `r` 是底部行号）相比，需用 [`OcrWord::y_bottom`] / [`OcrWord::y_center`]
/// 做语义换算。详见 [`k2types::OcrWord`] 文档。
///
/// Step 9.2 落地：算法部分（offset / drain / concatenate / sort）由 Step 0.4 spike
/// + ADR-016 8 桶设计指导，配合 C `willuslib/ocrwords.c` 1:1 复刻。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrStaging {
    /// 当前暂存的 words。对应 C `mi_ocrwords.word[]`。
    pub words: Vec<OcrWord>,
}

impl OcrStaging {
    /// 构造默认空 OcrStaging。
    #[must_use]
    pub fn new() -> Self {
        Self { words: Vec::new() }
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// 当前 word 数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// 清空所有 words。对应 C `ocrwords_clear` (`ocrwords.c:344`)。
    pub fn clear(&mut self) {
        self.words.clear();
    }

    /// 所有 word 的坐标加 `(dx, dy)`。对应 C `ocrwords_offset(dx, dy)`
    /// (`ocrwords.c:423-433`)。
    ///
    /// 注意 C 版 `r` 是底部行号、Rust [`OcrWord::y`] 是顶部 y，但 `dy` 都是
    /// "整段 word 在 y 方向平移多少像素"，语义一致；本函数直接对 Rust `y` 加 `dy`。
    ///
    /// 典型场景：
    /// - region 局部坐标 → master canvas 全局坐标（`k2master.c:744`，`dy = rows + gap_start`）
    /// - 页 flush 后剩余 words 上移（`k2master.c:1243`，`dy = -rows`），用
    ///   [`Self::offset_y`] 便捷调用
    pub fn offset_words(&mut self, dx: f64, dy: f64) {
        for w in &mut self.words {
            w.x += dx;
            w.y += dy;
        }
    }

    /// [`Self::offset_words`] 的便捷别名，仅平移 y。
    ///
    /// 对应 C `ocrwords_offset(0, dy)`（`k2master.c:1243` 页 flush 后剩余 words 平移）。
    pub fn offset_y(&mut self, dy: f64) {
        self.offset_words(0.0, dy);
    }

    /// 取出 y_center 在 `[y_min, y_max)` 范围内的 words，返回独立 [`Vec<OcrWord>`]，
    /// 并从原桶删除。
    ///
    /// 对应 C `k2master.c:1535-1544`（masterinfo_publish 内 OCR 选词循环）：
    ///
    /// ```text
    /// if (word.r - word.maxheight + word.h/2 < rowcount) {
    ///     ocrwords_add_word(ocrwords, &word);
    ///     ocrwords_remove_words(mi_ocrwords, i, i);
    /// }
    /// ```
    ///
    /// Rust 等价（中线 [`OcrWord::y_center`] = `y + h/2`，不依赖 maxheight 字段；
    /// 详见 Open Question 9.2.A）：
    ///
    /// ```text
    /// drain: y_center < y_max && y_center >= y_min
    /// ```
    ///
    /// 顺序保持稳定（按原索引升序保留两组），与 C 一致。
    pub fn drain_in_range(&mut self, y_min: f64, y_max: f64) -> Vec<OcrWord> {
        let mut taken = Vec::new();
        let mut kept = Vec::with_capacity(self.words.len());
        for w in self.words.drain(..) {
            let yc = w.y_center();
            if yc >= y_min && yc < y_max {
                taken.push(w);
            } else {
                kept.push(w);
            }
        }
        self.words = kept;
        taken
    }

    /// 把 `src` 中所有 words 追加到本桶（消费 `src`）。对应 C `ocrwords_concatenate`
    /// (`ocrwords.c:503-510`)：逐个 `ocrwords_add_word(dst, &src[i])`。
    pub fn concatenate(&mut self, src: Vec<OcrWord>) {
        self.words.extend(src);
    }

    /// 把 `src` 中所有 words 克隆并追加到本桶（不消费 `src`）。
    ///
    /// 对应 C `ocrwords_concatenate` 在 src 仍需保留时（C 通过 `ocrword_copy`
    /// 内部 dup `text/cpos/xbmp`，Rust 用 [`Clone`] 等价）。
    pub fn concatenate_clone(&mut self, src: &[OcrWord]) {
        self.words.extend_from_slice(src);
    }

    /// 按位置排序（行重叠 7% 容差，与 C [`ocrwords_sort_by_position`]
    /// (`ocrwords.c:513-589`) 等价）。
    ///
    /// 排序键：先按 word 矩形是否同一行（重叠 ≥ 7% 视为同行）；同行内按 x 升序；
    /// 不同行按 y_bottom 升序（C 按 `r` 即底部行号）。
    ///
    /// 详细算法委托给 [`k2ocr::mapping::sort_by_position`]，本方法仅薄包装。
    pub fn sort_by_position(&mut self) {
        sort_words_by_position(&mut self.words);
    }
}

impl Default for OcrStaging {
    fn default() -> Self {
        Self::new()
    }
}

/// 按位置排序 OCR words（Step 9.2 落地，对应 C `ocrwords_sort_by_position`）。
///
/// 与 C 版一致的比较语义：允许 7% 行重叠视为同一行；同行按 x 升序；
/// 不同行按 y_bottom (= C `r`) 升序。
///
/// **设计**：单独提供顶层函数（不仅是 [`OcrStaging::sort_by_position`] 方法），
/// 便于 `k2ocr::mapping` 在不依赖 [`OcrStaging`] 容器的情况下复用。
pub fn sort_words_by_position(words: &mut [OcrWord]) {
    // Rust stable sort vs C heapsort：等键时顺序不同，对结果不敏感（与 Step 6.3
    // ocrword sort_pair_desc 同源约定，Open Question 6.3.J）。
    words.sort_by(compare_position);
}

/// 1:1 复刻 C `ocrword_compare_position` (`ocrwords.c:569-589`)：
///
/// 7% 行重叠容差，行内 x 升序，跨行按 y_bottom 升序。返回 [`std::cmp::Ordering`]。
///
/// Rust `y_bottom` 等价 C `r`（底部行号），`h` 等价 C `h`（高度）。
fn compare_position(w1: &OcrWord, w2: &OcrWord) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let r1 = w1.y_bottom();
    let r2 = w2.y_bottom();
    let h1 = w1.h;
    let h2 = w2.h;
    // C: if (w1->r <= w2->r - w2->h) return -1;
    if r1 <= r2 - h2 {
        return Ordering::Less;
    }
    // C: if (w1->r - w1->h >= w2->r) return 1;
    if r1 - h1 >= r2 {
        return Ordering::Greater;
    }
    // 重叠计算
    let h = h1.min(h2).max(1.0);
    // C: ol = w1->r<w2->r ? w1->r-(w2->r-w2->h+1)+1 : w2->r-(w1->r-w1->h+1)+1
    // Rust 端用 f64，避免 C 的 +1/-1 整数边界（语义等价于"重叠像素数"）
    let ol = if r1 < r2 {
        r1 - (r2 - h2 + 1.0) + 1.0
    } else {
        r2 - (r1 - h1 + 1.0) + 1.0
    };
    let percentage_overlap = ol * 100.0 / h;
    if percentage_overlap < 7.0 {
        return if r1 < r2 {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    // 同行：按 x 排序；x 相同按 y_bottom 排序
    if (w1.x - w2.x).abs() < f64::EPSILON {
        return r1.partial_cmp(&r2).unwrap_or(Ordering::Equal);
    }
    w1.x.partial_cmp(&w2.x).unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn new_is_empty() {
        let s = OcrStaging::new();
        assert!(s.words.is_empty());
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn default_eq_new() {
        let a = OcrStaging::default();
        let b = OcrStaging::new();
        assert_eq!(a, b);
    }

    #[test]
    fn ocrword_construct_and_partial_eq() {
        let w1 = OcrWord {
            text: "hello".to_string(),
            x: 10.0,
            y: 20.0,
            w: 50.0,
            h: 12.0,
            confidence: 0.95,
        };
        let w2 = OcrWord {
            text: "hello".to_string(),
            x: 10.0,
            y: 20.0,
            w: 50.0,
            h: 12.0,
            confidence: 0.95,
        };
        assert_eq!(w1, w2);
    }

    #[test]
    fn ocrword_text_differs() {
        let w1 = OcrWord {
            text: "hello".to_string(),
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            confidence: 0.0,
        };
        let w2 = OcrWord {
            text: "world".to_string(),
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            confidence: 0.0,
        };
        assert_ne!(w1, w2);
    }

    #[test]
    fn words_writable() {
        let mut s = OcrStaging::new();
        s.words.push(OcrWord {
            text: "a".to_string(),
            x: 1.0,
            y: 2.0,
            w: 3.0,
            h: 4.0,
            confidence: 0.5,
        });
        assert_eq!(s.words.len(), 1);
        assert_eq!(s.words[0].text, "a");
        assert!(!s.is_empty());
        assert_eq!(s.len(), 1);
    }

    // ---- Step 9.2 offset_words ----

    #[test]
    fn offset_words_translates_all_words() {
        let mut s = OcrStaging::new();
        s.words.push(OcrWord::new("a", 10.0, 100.0, 30.0, 12.0));
        s.words.push(OcrWord::new("b", 50.0, 200.0, 20.0, 12.0));
        s.offset_words(5.0, 7.0);
        assert!((s.words[0].x - 15.0).abs() < 1e-9);
        assert!((s.words[0].y - 107.0).abs() < 1e-9);
        assert!((s.words[1].x - 55.0).abs() < 1e-9);
        assert!((s.words[1].y - 207.0).abs() < 1e-9);
    }

    #[test]
    fn offset_y_only_changes_y() {
        let mut s = OcrStaging::new();
        s.words.push(OcrWord::new("a", 10.0, 100.0, 30.0, 12.0));
        s.offset_y(-50.0);
        assert!((s.words[0].x - 10.0).abs() < 1e-9);
        assert!((s.words[0].y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn offset_words_on_empty_is_noop() {
        let mut s = OcrStaging::new();
        s.offset_words(100.0, 200.0);
        assert!(s.is_empty());
    }

    #[test]
    fn offset_words_preserves_width_and_height() {
        let mut s = OcrStaging::new();
        s.words.push(OcrWord::new("a", 10.0, 100.0, 30.0, 12.0));
        s.offset_words(100.0, -50.0);
        assert!((s.words[0].w - 30.0).abs() < 1e-9);
        assert!((s.words[0].h - 12.0).abs() < 1e-9);
    }

    // ---- Step 9.2 drain_in_range ----

    #[test]
    fn drain_in_range_takes_words_with_center_in_range() {
        // word.y_center = y + h/2
        // - a: y=10 h=10 → center=15 → 在 [0, 100) → 取
        // - b: y=80 h=10 → center=85 → 在 [0, 100) → 取
        // - c: y=100 h=10 → center=105 → 不在 → 留
        // - d: y=200 h=10 → center=205 → 不在 → 留
        let mut s = OcrStaging::new();
        s.words.push(OcrWord::new("a", 0.0, 10.0, 5.0, 10.0));
        s.words.push(OcrWord::new("b", 0.0, 80.0, 5.0, 10.0));
        s.words.push(OcrWord::new("c", 0.0, 100.0, 5.0, 10.0));
        s.words.push(OcrWord::new("d", 0.0, 200.0, 5.0, 10.0));
        let taken = s.drain_in_range(0.0, 100.0);
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0].text, "a");
        assert_eq!(taken[1].text, "b");
        assert_eq!(s.words.len(), 2);
        assert_eq!(s.words[0].text, "c");
        assert_eq!(s.words[1].text, "d");
    }

    #[test]
    fn drain_in_range_lower_bound_inclusive() {
        // 边界：y_center == y_min 视为在范围内
        let mut s = OcrStaging::new();
        s.words.push(OcrWord::new("x", 0.0, 95.0, 1.0, 10.0)); // center=100
        let taken = s.drain_in_range(100.0, 200.0);
        assert_eq!(taken.len(), 1);
        assert!(s.is_empty());
    }

    #[test]
    fn drain_in_range_upper_bound_exclusive() {
        // y_center == y_max 不视为在范围内
        let mut s = OcrStaging::new();
        s.words.push(OcrWord::new("x", 0.0, 95.0, 1.0, 10.0)); // center=100
        let taken = s.drain_in_range(0.0, 100.0);
        assert!(taken.is_empty());
        assert_eq!(s.words.len(), 1);
    }

    #[test]
    fn drain_in_range_empty_returns_empty() {
        let mut s = OcrStaging::new();
        let taken = s.drain_in_range(0.0, 1000.0);
        assert!(taken.is_empty());
    }

    #[test]
    fn drain_in_range_preserves_order() {
        // 一连串 words 部分提走，留下的应保持原相对顺序
        let mut s = OcrStaging::new();
        for (i, y) in [10.0, 30.0, 50.0, 200.0, 220.0, 70.0].iter().enumerate() {
            s.words
                .push(OcrWord::new(format!("w{i}"), 0.0, *y, 5.0, 5.0));
        }
        // y_center: 12.5, 32.5, 52.5, 202.5, 222.5, 72.5
        // 在 [0, 100) 的: w0/w1/w2/w5 (4 个) — 但 w5 在 y=70 后于 w3/w4
        let taken = s.drain_in_range(0.0, 100.0);
        assert_eq!(taken.len(), 4);
        assert_eq!(taken[0].text, "w0");
        assert_eq!(taken[1].text, "w1");
        assert_eq!(taken[2].text, "w2");
        assert_eq!(taken[3].text, "w5");
        // 留下：w3, w4
        assert_eq!(s.words.len(), 2);
        assert_eq!(s.words[0].text, "w3");
        assert_eq!(s.words[1].text, "w4");
    }

    // ---- Step 9.2 concatenate ----

    #[test]
    fn concatenate_appends_all_words() {
        let mut s = OcrStaging::new();
        s.words.push(OcrWord::new("a", 0.0, 0.0, 1.0, 1.0));
        let src = vec![
            OcrWord::new("b", 0.0, 0.0, 1.0, 1.0),
            OcrWord::new("c", 0.0, 0.0, 1.0, 1.0),
        ];
        s.concatenate(src);
        assert_eq!(s.words.len(), 3);
        assert_eq!(s.words[0].text, "a");
        assert_eq!(s.words[1].text, "b");
        assert_eq!(s.words[2].text, "c");
    }

    #[test]
    fn concatenate_clone_preserves_src() {
        let mut s = OcrStaging::new();
        let src = [
            OcrWord::new("a", 0.0, 0.0, 1.0, 1.0),
            OcrWord::new("b", 0.0, 0.0, 1.0, 1.0),
        ];
        s.concatenate_clone(&src);
        assert_eq!(s.words.len(), 2);
        // src 仍可用
        assert_eq!(src.len(), 2);
        assert_eq!(src[0].text, "a");
    }

    // ---- Step 9.2 clear ----

    #[test]
    fn clear_removes_all_words() {
        let mut s = OcrStaging::new();
        s.words.push(OcrWord::new("a", 0.0, 0.0, 1.0, 1.0));
        s.words.push(OcrWord::new("b", 0.0, 0.0, 1.0, 1.0));
        s.clear();
        assert!(s.is_empty());
    }

    // ---- Step 9.2 sort_by_position ----

    #[test]
    fn sort_by_position_orders_within_row_by_x() {
        let mut s = OcrStaging::new();
        // 三个 word 在同一行（y_bottom ≈ 20，h=10 重叠 ≥ 7%）
        s.words.push(OcrWord::new("c", 200.0, 10.0, 50.0, 10.0));
        s.words.push(OcrWord::new("a", 10.0, 10.0, 50.0, 10.0));
        s.words.push(OcrWord::new("b", 100.0, 10.0, 50.0, 10.0));
        s.sort_by_position();
        assert_eq!(s.words[0].text, "a");
        assert_eq!(s.words[1].text, "b");
        assert_eq!(s.words[2].text, "c");
    }

    #[test]
    fn sort_by_position_orders_across_rows_by_y() {
        let mut s = OcrStaging::new();
        // y_bottom 差距 > h（不重叠），按 y_bottom 升序
        s.words.push(OcrWord::new("row3", 0.0, 200.0, 10.0, 10.0)); // y_bot=210
        s.words.push(OcrWord::new("row1", 0.0, 0.0, 10.0, 10.0)); // y_bot=10
        s.words.push(OcrWord::new("row2", 0.0, 100.0, 10.0, 10.0)); // y_bot=110
        s.sort_by_position();
        assert_eq!(s.words[0].text, "row1");
        assert_eq!(s.words[1].text, "row2");
        assert_eq!(s.words[2].text, "row3");
    }

    #[test]
    fn sort_by_position_empty_is_noop() {
        let mut s = OcrStaging::new();
        s.sort_by_position();
        assert!(s.is_empty());
    }

    #[test]
    fn compare_position_disjoint_rows() {
        // w1 完全在 w2 上方
        let w1 = OcrWord::new("up", 0.0, 0.0, 5.0, 10.0); // y_bot=10
        let w2 = OcrWord::new("down", 0.0, 100.0, 5.0, 10.0); // y_bot=110
        assert_eq!(compare_position(&w1, &w2), std::cmp::Ordering::Less);
        assert_eq!(compare_position(&w2, &w1), std::cmp::Ordering::Greater);
    }

    #[test]
    fn compare_position_overlap_geq_7pct_treated_as_same_row() {
        // h=20，重叠 5 像素（25%）≥ 7%
        let w1 = OcrWord::new("a", 10.0, 80.0, 5.0, 20.0); // y_bot=100
        let w2 = OcrWord::new("b", 50.0, 95.0, 5.0, 20.0); // y_bot=115，区间 [95,115]
                                                           // w1 区间 [80,100]，与 w2 [95,115] 重叠 5 像素
        assert_eq!(compare_position(&w1, &w2), std::cmp::Ordering::Less); // x 升序
    }

    #[test]
    fn compare_position_overlap_lt_7pct_different_rows() {
        // h=100，重叠 5 像素 = 5% < 7%
        let w1 = OcrWord::new("a", 100.0, 0.0, 5.0, 100.0); // y_bot=100
        let w2 = OcrWord::new("b", 0.0, 95.0, 5.0, 100.0); // y_bot=195
                                                           // [0,100] vs [95,195]，overlap=5
        assert_eq!(compare_position(&w1, &w2), std::cmp::Ordering::Less); // 按 y_bot
    }
}
