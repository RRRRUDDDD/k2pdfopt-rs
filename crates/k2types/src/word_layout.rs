//! WordLayout — 完整文本 reflow 后的 word 位置信息
//!
//! **Step 11.3 起 deprecated**（保留结构以便 Step 11.5 OCR 路径再评估）。
//! 自 Step 11.3 起 `ReflowOutcome::TextReflowed` 字段类型改为 `Vec<FlushedLine>`
//! 直接由 wrap_state 流水线吐出已对齐到 master canvas 的整行位图；word 级
//! 坐标信息保留在 `FlushedLine.wrectmaps`（C `WRECTMAPS` 同源）。
//!
//! 本结构仍保留：
//! - 便于 Step 11.5 OCR 不可见层 / PDF writer 评估是否仍需要"word → source"
//!   的强类型表达（与 `WRectMap` 字段集有部分重叠）
//! - 字段集设计已通过 Step 11.1 单测稳定；删除后再加回来代价不对等
//!
//! 如果 Step 11.5 评估后决定彻底退役本类型，将整体移除（含 `k2types::lib.rs`
//! 的 re-export 与本文件）；目前 deprecated 但保留构造能力。
//!
//! ## 坐标系
//!
//! - `source_*`：源 bitmap 坐标系（region 内 inclusive 像素坐标）
//! - `dest_*`：master canvas 目标坐标系（reflow 后的输出位置 inclusive 像素坐标）
//! - 语义沿用 `k2core::Rect`（inclusive，宽 = x1 − x0 + 1，高 = y1 − y0 + 1）
//!
//! ## 为什么不复用 `k2core::Rect`
//!
//! `k2core` 已依赖 `k2types`（基础类型层），如果 `k2types::WordLayout` 反向依赖
//! `k2core::Rect`，将形成 `k2types → k2core → k2types` 循环依赖。
//! 方案：本结构以四个独立 `i32` 字段表达 source/dest 矩形（语义等价 inclusive
//! Rect）。如果未来跨 crate 需要 `Rect` 共享，应整体把 `Rect` 提到 `k2types`，
//! 而不是在此引入反向依赖。详见 Step 11.1 Open Question 11.1.A。

/// 完整文本 reflow 后一个 word 的源/目标布局信息。
///
/// **Step 11.3 起 deprecated**（保留结构以便 Step 11.5 OCR 路径再评估）：
/// `ReflowOutcome::TextReflowed` 字段类型改为 `Vec<FlushedLine>`；word 级
/// 坐标信息保留在 `FlushedLine.wrectmaps`（[`super::WordLayout`] 与 `WRectMap`
/// 字段集有部分重叠，Step 11.5 评估去留）。
///
/// 字段语义与 `k2core::Rect` 一致 — `x0`/`y0` 是左上角，`x1`/`y1` 是右下角，
/// **均为 inclusive** 像素坐标；宽 = `x1 − x0 + 1`，高 = `y1 − y0 + 1`。
#[deprecated(
    since = "0.2.0",
    note = "Step 11.3 起 ReflowOutcome::TextReflowed 携带 Vec<FlushedLine>；word 级坐标走 wrectmap。Step 11.5 OCR 评估后可能整体移除"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordLayout {
    /// 源 bitmap 坐标系：左上角 x（inclusive）
    pub source_x0: i32,
    /// 源 bitmap 坐标系：左上角 y（inclusive）
    pub source_y0: i32,
    /// 源 bitmap 坐标系：右下角 x（inclusive）
    pub source_x1: i32,
    /// 源 bitmap 坐标系：右下角 y（inclusive）
    pub source_y1: i32,
    /// 目标 canvas 坐标系：左上角 x（inclusive）
    pub dest_x0: i32,
    /// 目标 canvas 坐标系：左上角 y（inclusive）
    pub dest_y0: i32,
    /// 目标 canvas 坐标系：右下角 x（inclusive）
    pub dest_x1: i32,
    /// 目标 canvas 坐标系：右下角 y（inclusive）
    pub dest_y1: i32,
    /// 字号（lowercase char height），用于 hyphen detection / 字号一致性裁剪
    pub lcheight: i32,
    /// 来源 region 索引（debug / 反查用）
    pub region_idx: usize,
}

#[allow(deprecated)]
impl WordLayout {
    /// 源宽度（inclusive 语义：x1 − x0 + 1，空区域返回 0）
    #[inline]
    #[must_use]
    pub fn source_width(&self) -> i32 {
        if self.source_x1 < self.source_x0 {
            0
        } else {
            self.source_x1 - self.source_x0 + 1
        }
    }

    /// 源高度（inclusive 语义：y1 − y0 + 1，空区域返回 0）
    #[inline]
    #[must_use]
    pub fn source_height(&self) -> i32 {
        if self.source_y1 < self.source_y0 {
            0
        } else {
            self.source_y1 - self.source_y0 + 1
        }
    }

    /// 目标宽度（inclusive 语义：x1 − x0 + 1，空区域返回 0）
    #[inline]
    #[must_use]
    pub fn dest_width(&self) -> i32 {
        if self.dest_x1 < self.dest_x0 {
            0
        } else {
            self.dest_x1 - self.dest_x0 + 1
        }
    }

    /// 目标高度（inclusive 语义：y1 − y0 + 1，空区域返回 0）
    #[inline]
    #[must_use]
    pub fn dest_height(&self) -> i32 {
        if self.dest_y1 < self.dest_y0 {
            0
        } else {
            self.dest_y1 - self.dest_y0 + 1
        }
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    fn sample() -> WordLayout {
        WordLayout {
            source_x0: 10,
            source_y0: 20,
            source_x1: 49,
            source_y1: 39,
            dest_x0: 100,
            dest_y0: 200,
            dest_x1: 139,
            dest_y1: 219,
            lcheight: 16,
            region_idx: 3,
        }
    }

    #[test]
    fn dimensions_inclusive() {
        let w = sample();
        assert_eq!(w.source_width(), 40);
        assert_eq!(w.source_height(), 20);
        assert_eq!(w.dest_width(), 40);
        assert_eq!(w.dest_height(), 20);
    }

    #[test]
    fn empty_when_x1_less_than_x0() {
        let mut w = sample();
        w.source_x1 = w.source_x0 - 1;
        w.source_y1 = w.source_y0 - 1;
        w.dest_x1 = w.dest_x0 - 1;
        w.dest_y1 = w.dest_y0 - 1;
        assert_eq!(w.source_width(), 0);
        assert_eq!(w.source_height(), 0);
        assert_eq!(w.dest_width(), 0);
        assert_eq!(w.dest_height(), 0);
    }

    #[test]
    fn single_pixel_word() {
        let w = WordLayout {
            source_x0: 5,
            source_y0: 5,
            source_x1: 5,
            source_y1: 5,
            dest_x0: 0,
            dest_y0: 0,
            dest_x1: 0,
            dest_y1: 0,
            lcheight: 1,
            region_idx: 0,
        };
        assert_eq!(w.source_width(), 1);
        assert_eq!(w.source_height(), 1);
        assert_eq!(w.dest_width(), 1);
        assert_eq!(w.dest_height(), 1);
    }

    #[test]
    fn clone_and_equality() {
        let a = sample();
        let b = a.clone();
        assert_eq!(a, b);
    }
}
