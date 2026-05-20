//! `mapping` —— OCR word 坐标映射工具（Step 9.2）。
//!
//! 把 source bitmap 坐标系下的 [`OcrWord`] 列表映射到 master canvas 坐标系，
//! 进而映射到 output PDF 坐标系。本模块是纯函数库，不持有状态，所有 API 取
//! `&mut [OcrWord]` 原地修改或返回新 [`Vec<OcrWord>`]。
//!
//! # C 对照
//!
//! 来源：`willuslib/ocrwords.c` 的几何变换函数族：
//!
//! | C 函数 | Rust 等价 | 备注 |
//! |--------|-----------|------|
//! | `ocrwords_offset(words, dx, dy)` (`423`) | [`offset`] | 平移 |
//! | `ocrwords_scale(words, srat)` (`439`) | [`scale`] | 浮点缩放（位置 + 尺寸 + lcheight/maxheight）|
//! | `ocrwords_int_scale(words, ndiv)` (`481`) | [`int_scale`] | 整数除法缩放 |
//! | `ocrwords_concatenate(dst, src)` (`503`) | [`concatenate`] | 拼接 |
//! | `ocrwords_sort_by_position(words)` (`513`) | [`sort_by_position`] | 7% 重叠容差行排序 |
//! | `ocrwords_rot90(words, bmpw)` (`461`) | _未实现_ | Open Question 9.2.B（Rust OcrWord 无 `rot` 字段）|
//!
//! # 坐标系约定
//!
//! 与 [`k2types::OcrWord`] 一致：
//! - `(x, y)` = word 矩形**左上角**像素坐标（image top-left 原点，y 向下增）
//! - 不同于 C `(c, r)`（C `r` 是底部行号）
//! - 几何变换公式与 C 一致（按矩形位置 + 尺寸操作）但作用于 Rust 的 top-left 表示
//!
//! # 使用场景（C 版调用路径）
//!
//! | 场景 | C 行号 | Rust 等价调用 |
//! |------|-------|--------------|
//! | OCR 倍率渲染后缩回原图 | `k2master.c:707` | [`int_scale`] |
//! | region 局部 → master 全局 | `k2master.c:744` | [`offset`] |
//! | append region words 到 master | `k2master.c:763` | [`OcrStaging::concatenate`](../../../k2layout/master/ocr_staging/struct.OcrStaging.html#method.concatenate) |
//! | flush 后剩余 words 上移 | `k2master.c:1243` | [`offset`] (dx=0) 或 [`OcrStaging::offset_y`](../../../k2layout/master/ocr_staging/struct.OcrStaging.html#method.offset_y) |
//! | flush 时按行选词 | `k2master.c:1535-1544` | [`OcrStaging::drain_in_range`](../../../k2layout/master/ocr_staging/struct.OcrStaging.html#method.drain_in_range) |
//! | flush 后页内居中 offset | `k2master.c:1576` | [`offset`] (dy=0) |
//! | 输出前按位置排序 | `k2master.c:1582` | [`sort_by_position`] |

use k2types::OcrWord;

/// 给每个 word 的 `(x, y)` 加 `(dx, dy)`。
///
/// 对应 C `ocrwords_offset` (`ocrwords.c:423-433`)。
///
/// 注意：C 操作的是 `(c, r)` = `(x, y_bottom)`，但因 `r += dy` 与 `y_top += dy`
/// 等价（矩形整体平移），Rust 端直接对 `(x, y)` 加偏移即可。
pub fn offset(words: &mut [OcrWord], dx: f64, dy: f64) {
    for w in words {
        w.x += dx;
        w.y += dy;
    }
}

/// 浮点缩放（位置 + 尺寸）。`srat == 1.0` 时 no-op。
///
/// 对应 C `ocrwords_scale` (`ocrwords.c:439-458`)：
///
/// ```text
/// c2 = (c + w - 1) * srat
/// r2 = (r + h - 1) * srat
/// c  = c * srat
/// r  = r * srat
/// maxheight = maxheight * srat
/// lcheight  = lcheight  * srat
/// w  = c2 - c + 1
/// h  = r2 - r + 1
/// bmpscale *= srat
/// ```
///
/// Rust 端 `(x, y)` 是 top-left（与 C `(c, r)` 都是 word 矩形参考点），公式一致；
/// `maxheight` / `lcheight` / `bmpscale` 字段 Rust 端 [`OcrWord`] 未保留（Open
/// Question 9.2.A，目前 Rust 仅 6 字段），所以本函数只缩放 `(x, y, w, h)`。
pub fn scale(words: &mut [OcrWord], srat: f64) {
    for w in words {
        // 与 C 同源公式：先算右下角，再算左上角，最后用差值求 w/h
        let x_right = (w.x + w.w - 1.0) * srat;
        let y_bottom = (w.y + w.h - 1.0) * srat;
        w.x *= srat;
        w.y *= srat;
        w.w = x_right - w.x + 1.0;
        w.h = y_bottom - w.y + 1.0;
    }
}

/// 整数除法缩放（与 C `ocrwords_int_scale` (`ocrwords.c:481-500`) 一致）。
///
/// 当源 bitmap 用 `ndiv` 倍渲染做 OCR（提高小字精度），输出阶段需用此函数把
/// word 坐标 / 尺寸缩回到原始 source bitmap 坐标系。
///
/// `ndiv == 0` 时不做修改（防 div0）。
///
/// **整数语义**：与 C 完全一致地用整数除法，舍弃小数部分（不是 round 也不是
/// trunc 浮点）。
pub fn int_scale(words: &mut [OcrWord], ndiv: i32) {
    if ndiv == 0 {
        return;
    }
    let n = i64::from(ndiv);
    for w in words {
        // 转 i64 做整数除法（保持 C 语义）
        let c = w.x as i64;
        let r = w.y as i64;
        let wi = w.w as i64;
        let hi = w.h as i64;
        let c2 = (c + wi - 1) / n;
        let r2 = (r + hi - 1) / n;
        let new_x = c / n;
        let new_y = r / n;
        w.x = new_x as f64;
        w.y = new_y as f64;
        w.w = (c2 - new_x + 1) as f64;
        w.h = (r2 - new_y + 1) as f64;
    }
}

/// 把 `src` 追加到 `dst`（消费 `src`）。
///
/// 对应 C `ocrwords_concatenate` (`ocrwords.c:503-510`)：逐个 `ocrwords_add_word`，
/// Rust 用 [`Vec::extend`] 等价。
pub fn concatenate(dst: &mut Vec<OcrWord>, src: Vec<OcrWord>) {
    dst.extend(src);
}

/// 按位置排序（与 C `ocrwords_sort_by_position` 等价）。
///
/// 7% 行重叠容差视为同行；同行按 `x` 升序；跨行按 `y_bottom` 升序。
///
/// 顶层 free function，不依赖 [`k2layout`] 容器，避免 k2ocr → k2layout 循环依赖；
/// [`k2layout::master::ocr_staging::OcrStaging::sort_by_position`] 内部有功能等价的
/// 私有实现（共享同一 C 源算法）。
pub fn sort_by_position(words: &mut [OcrWord]) {
    // Rust stable sort vs C heapsort：等键时顺序不同（与 Step 6.3 sort_pair 同源
    // 约定 Open Question 6.3.J）
    words.sort_by(compare_position);
}

/// 1:1 复刻 C `ocrword_compare_position` (`willuslib/ocrwords.c:569-589`)：
///
/// 7% 行重叠容差，行内 x 升序，跨行按 y_bottom 升序。返回 [`std::cmp::Ordering`]。
fn compare_position(w1: &OcrWord, w2: &OcrWord) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let r1 = w1.y_bottom();
    let r2 = w2.y_bottom();
    let h1 = w1.h;
    let h2 = w2.h;
    if r1 <= r2 - h2 {
        return Ordering::Less;
    }
    if r1 - h1 >= r2 {
        return Ordering::Greater;
    }
    let h = h1.min(h2).max(1.0);
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
    if (w1.x - w2.x).abs() < f64::EPSILON {
        return r1.partial_cmp(&r2).unwrap_or(Ordering::Equal);
    }
    w1.x.partial_cmp(&w2.x).unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn w(text: &str, x: f64, y: f64, w: f64, h: f64) -> OcrWord {
        OcrWord::new(text, x, y, w, h)
    }

    // ---- offset ----

    #[test]
    fn offset_zero_is_noop() {
        let mut words = vec![w("a", 10.0, 20.0, 30.0, 40.0)];
        offset(&mut words, 0.0, 0.0);
        assert!((words[0].x - 10.0).abs() < 1e-9);
        assert!((words[0].y - 20.0).abs() < 1e-9);
    }

    #[test]
    fn offset_translates_position() {
        let mut words = vec![
            w("a", 10.0, 20.0, 30.0, 40.0),
            w("b", 50.0, 60.0, 30.0, 40.0),
        ];
        offset(&mut words, 100.0, -10.0);
        assert!((words[0].x - 110.0).abs() < 1e-9);
        assert!((words[0].y - 10.0).abs() < 1e-9);
        assert!((words[1].x - 150.0).abs() < 1e-9);
        assert!((words[1].y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn offset_preserves_size() {
        let mut words = vec![w("a", 10.0, 20.0, 30.0, 40.0)];
        offset(&mut words, 100.0, 200.0);
        assert!((words[0].w - 30.0).abs() < 1e-9);
        assert!((words[0].h - 40.0).abs() < 1e-9);
    }

    #[test]
    fn offset_empty_slice_is_safe() {
        let mut words: Vec<OcrWord> = vec![];
        offset(&mut words, 100.0, 200.0);
        assert!(words.is_empty());
    }

    // ---- scale ----

    #[test]
    fn scale_identity_returns_same() {
        let mut words = vec![w("a", 10.0, 20.0, 30.0, 40.0)];
        scale(&mut words, 1.0);
        assert!((words[0].x - 10.0).abs() < 1e-9);
        assert!((words[0].y - 20.0).abs() < 1e-9);
        assert!((words[0].w - 30.0).abs() < 1e-9);
        assert!((words[0].h - 40.0).abs() < 1e-9);
    }

    #[test]
    fn scale_double_scales_position_and_size() {
        // C 公式：c2=(10+30-1)*2=78, r2=(20+40-1)*2=118, c=20, r=40, w=78-20+1=59, h=118-40+1=79
        let mut words = vec![w("a", 10.0, 20.0, 30.0, 40.0)];
        scale(&mut words, 2.0);
        assert!((words[0].x - 20.0).abs() < 1e-9);
        assert!((words[0].y - 40.0).abs() < 1e-9);
        assert!((words[0].w - 59.0).abs() < 1e-9);
        assert!((words[0].h - 79.0).abs() < 1e-9);
    }

    #[test]
    fn scale_half_shrinks() {
        // c2=(10+30-1)*0.5=19.5, r2=(20+40-1)*0.5=29.5, c=5, r=10, w=15.5, h=20.5
        let mut words = vec![w("a", 10.0, 20.0, 30.0, 40.0)];
        scale(&mut words, 0.5);
        assert!((words[0].x - 5.0).abs() < 1e-9);
        assert!((words[0].y - 10.0).abs() < 1e-9);
        assert!((words[0].w - 15.5).abs() < 1e-9);
        assert!((words[0].h - 20.5).abs() < 1e-9);
    }

    // ---- int_scale ----

    #[test]
    fn int_scale_zero_divisor_is_noop() {
        let mut words = vec![w("a", 10.0, 20.0, 30.0, 40.0)];
        int_scale(&mut words, 0);
        assert!((words[0].x - 10.0).abs() < 1e-9);
        assert!((words[0].y - 20.0).abs() < 1e-9);
    }

    #[test]
    fn int_scale_divisor_one_is_noop() {
        let mut words = vec![w("a", 10.0, 20.0, 30.0, 40.0)];
        int_scale(&mut words, 1);
        assert!((words[0].x - 10.0).abs() < 1e-9);
        assert!((words[0].y - 20.0).abs() < 1e-9);
        assert!((words[0].w - 30.0).abs() < 1e-9);
        assert!((words[0].h - 40.0).abs() < 1e-9);
    }

    #[test]
    fn int_scale_divisor_two_matches_c_semantics() {
        // C: c=10, w=30 → c2=(10+30-1)/2=19, new_c=10/2=5, new_w=19-5+1=15
        //    r=20, h=40 → r2=(20+40-1)/2=29, new_r=20/2=10, new_h=29-10+1=20
        let mut words = vec![w("a", 10.0, 20.0, 30.0, 40.0)];
        int_scale(&mut words, 2);
        assert!((words[0].x - 5.0).abs() < 1e-9);
        assert!((words[0].y - 10.0).abs() < 1e-9);
        assert!((words[0].w - 15.0).abs() < 1e-9);
        assert!((words[0].h - 20.0).abs() < 1e-9);
    }

    #[test]
    fn int_scale_truncates_toward_zero() {
        // C 整数除法：奇数会被截掉小数
        // c=11, w=33 → c2=(11+33-1)/2=43/2=21, new_c=11/2=5, new_w=21-5+1=17
        let mut words = vec![w("a", 11.0, 0.0, 33.0, 10.0)];
        int_scale(&mut words, 2);
        assert!((words[0].x - 5.0).abs() < 1e-9);
        assert!((words[0].w - 17.0).abs() < 1e-9);
    }

    // ---- concatenate ----

    #[test]
    fn concatenate_appends_all() {
        let mut dst = vec![w("a", 0.0, 0.0, 1.0, 1.0)];
        let src = vec![w("b", 0.0, 0.0, 1.0, 1.0), w("c", 0.0, 0.0, 1.0, 1.0)];
        concatenate(&mut dst, src);
        assert_eq!(dst.len(), 3);
        assert_eq!(dst[1].text, "b");
        assert_eq!(dst[2].text, "c");
    }

    #[test]
    fn concatenate_empty_src_is_noop() {
        let mut dst = vec![w("a", 0.0, 0.0, 1.0, 1.0)];
        concatenate(&mut dst, vec![]);
        assert_eq!(dst.len(), 1);
    }

    // ---- sort_by_position ----

    #[test]
    fn sort_by_position_within_row_by_x() {
        let mut words = vec![
            w("c", 200.0, 10.0, 50.0, 10.0),
            w("a", 10.0, 10.0, 50.0, 10.0),
            w("b", 100.0, 10.0, 50.0, 10.0),
        ];
        sort_by_position(&mut words);
        assert_eq!(words[0].text, "a");
        assert_eq!(words[1].text, "b");
        assert_eq!(words[2].text, "c");
    }

    #[test]
    fn sort_by_position_across_rows_by_y_bottom() {
        let mut words = vec![
            w("row3", 0.0, 200.0, 10.0, 10.0),
            w("row1", 0.0, 0.0, 10.0, 10.0),
            w("row2", 0.0, 100.0, 10.0, 10.0),
        ];
        sort_by_position(&mut words);
        assert_eq!(words[0].text, "row1");
        assert_eq!(words[1].text, "row2");
        assert_eq!(words[2].text, "row3");
    }

    // ---- 端到端流水线（C k2master.c:707/744/763/1243 等价调用顺序）----

    #[test]
    fn pipeline_int_scale_then_offset_then_sort() {
        // 模拟 region OCR 倍率缩放 + 平移 + 多 region 拼接 + 排序
        let mut region_words = vec![
            w("hello", 20.0, 40.0, 60.0, 20.0),
            w("world", 100.0, 40.0, 60.0, 20.0),
        ];
        // ocrwords_int_scale(words, nocr=2)
        int_scale(&mut region_words, 2);
        // 验证 hello: c2=(20+60-1)/2=39, new_c=10, new_w=30; r2=(40+20-1)/2=29, new_r=20, new_h=10
        assert!((region_words[0].x - 10.0).abs() < 1e-9);
        assert!((region_words[0].y - 20.0).abs() < 1e-9);

        // ocrwords_offset(words, dw=50, masterinfo.rows+gap_start=100)
        offset(&mut region_words, 50.0, 100.0);
        assert!((region_words[0].x - 60.0).abs() < 1e-9);
        assert!((region_words[0].y - 120.0).abs() < 1e-9);

        // 拼接到 master
        let mut master = vec![w("prev", 0.0, 0.0, 5.0, 5.0)];
        concatenate(&mut master, region_words);
        assert_eq!(master.len(), 3);

        // 排序
        sort_by_position(&mut master);
        assert_eq!(master[0].text, "prev"); // y_bot=5 最上
    }
}
