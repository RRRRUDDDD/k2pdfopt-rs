//! 矩形几何 `Rect` —— inclusive 语义，对应 C 版 `c1/r1/c2/r2`。
//!
//! ## 语义对齐
//!
//! C 版（[`k2pdfoptlib/bmpregion.c`] 等）的 `BMPREGION` 用 `c1/r1/c2/r2`
//! 表示"左上角列号、行号"和"右下角列号、行号"，**包含两端**（inclusive）。
//! 例如 `c1=0, c2=4` 表示宽度为 5 像素的区域。
//!
//! Rust 版 [`Rect`] 沿用 inclusive 语义：`x0`/`x1` 分别是 left/right 列号；
//! `y0`/`y1` 分别是 top/bottom 行号；宽 = `x1 - x0 + 1`，高 = `y1 - y0 + 1`。
//!
//! ## 为什么是 `i32` 而不是 `u32`
//!
//! C 版算法（如 trim margins / bbox shrink）会临时构造"空区域"用 `c2 < c1`
//! 表示，或返回 `-1` 作为越界哨兵。Rust 版保留 `i32` 与之等价；
//! [`Rect::is_empty`] / [`Rect::is_valid`] 提供显式状态检查。
//!
//! 来源：`rust-rewrite-plan.md` v2.1 §8.2。

/// 矩形（inclusive 语义）。
///
/// - `x0 <= x1`：水平范围 `[x0, x1]`
/// - `y0 <= y1`：垂直范围 `[y0, y1]`
/// - 当 `x0 > x1` 或 `y0 > y1` 时为"空矩形"（`is_empty() == true`）
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    /// 用 4 个 inclusive 端点构造矩形（不校验顺序）。
    #[must_use]
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// 用左上角坐标 `(x, y)` 和**正向**宽高构造（与 [`Rect::new`] 互补）。
    /// `width=0` 或 `height=0` 会得到 `is_empty()==true` 的矩形（如 `x1=x0-1`）。
    #[must_use]
    pub const fn from_xywh(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x0: x,
            y0: y,
            x1: x + (width as i32) - 1,
            y1: y + (height as i32) - 1,
        }
    }

    /// 宽度（inclusive 语义：`x1 - x0 + 1`）。空矩形返回 0。
    #[must_use]
    pub const fn width(self) -> u32 {
        if self.x1 < self.x0 {
            0
        } else {
            (self.x1 - self.x0 + 1) as u32
        }
    }

    /// 高度（inclusive 语义：`y1 - y0 + 1`）。空矩形返回 0。
    #[must_use]
    pub const fn height(self) -> u32 {
        if self.y1 < self.y0 {
            0
        } else {
            (self.y1 - self.y0 + 1) as u32
        }
    }

    /// 面积 = 宽 × 高，溢出走 `saturating_mul`（避免 panic）。
    #[must_use]
    pub const fn area(self) -> u64 {
        (self.width() as u64).saturating_mul(self.height() as u64)
    }

    /// 是否为空矩形（宽或高为 0）。
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.x1 < self.x0 || self.y1 < self.y0
    }

    /// 是否为有效非空矩形。
    #[must_use]
    pub const fn is_valid(self) -> bool {
        !self.is_empty()
    }

    /// 点 `(x, y)` 是否落在矩形内（inclusive）。
    #[must_use]
    pub const fn contains_point(self, x: i32, y: i32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }

    /// `other` 是否完全包含在 `self` 内。空矩形被认为"包含在任何矩形内"
    /// （`other.is_empty() == true` 时返回 `true`）。
    #[must_use]
    pub const fn contains_rect(self, other: Rect) -> bool {
        if other.is_empty() {
            return true;
        }
        if self.is_empty() {
            return false;
        }
        other.x0 >= self.x0 && other.x1 <= self.x1 && other.y0 >= self.y0 && other.y1 <= self.y1
    }

    /// 是否与 `other` 相交（共享至少一个像素）。
    #[must_use]
    pub const fn intersects(self, other: Rect) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        self.x0 <= other.x1 && self.x1 >= other.x0 && self.y0 <= other.y1 && self.y1 >= other.y0
    }

    /// 与 `other` 的交集。无重叠时返回 `is_empty()==true` 的矩形（`x1<x0`）。
    #[must_use]
    pub fn intersection(self, other: Rect) -> Rect {
        Rect {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
    }

    /// 与 `other` 的并集（最小包围矩形）。任一为空时返回另一个。
    #[must_use]
    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Rect {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    /// 将矩形整体平移 `(dx, dy)`。
    #[must_use]
    pub const fn translate(self, dx: i32, dy: i32) -> Rect {
        Rect {
            x0: self.x0 + dx,
            y0: self.y0 + dy,
            x1: self.x1 + dx,
            y1: self.y1 + dy,
        }
    }

    /// 将矩形钳制到 `bounds` 内。两矩形不相交时结果为空矩形。
    #[must_use]
    pub fn clamp_to(self, bounds: Rect) -> Rect {
        self.intersection(bounds)
    }
}

impl Default for Rect {
    /// 默认是一个 1x1 的零位置矩形（点 (0,0)），与 C 版 `bmp_init` 后未设置 region
    /// 的不确定状态隔离开。需要"空矩形"时显式用 `Rect::new(0,0,-1,-1)`。
    fn default() -> Self {
        Self::new(0, 0, 0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_inclusive_width_height() {
        let r = Rect::new(0, 0, 4, 2);
        assert_eq!(r.width(), 5);
        assert_eq!(r.height(), 3);
        assert_eq!(r.area(), 15);
        assert!(r.is_valid());
        assert!(!r.is_empty());
    }

    #[test]
    fn rect_from_xywh_round_trip() {
        let r = Rect::from_xywh(10, 20, 5, 7);
        assert_eq!(r.x0, 10);
        assert_eq!(r.y0, 20);
        assert_eq!(r.x1, 14);
        assert_eq!(r.y1, 26);
        assert_eq!(r.width(), 5);
        assert_eq!(r.height(), 7);
    }

    #[test]
    fn rect_from_xywh_zero_width_is_empty() {
        let r = Rect::from_xywh(5, 5, 0, 3);
        assert!(r.is_empty());
        assert_eq!(r.width(), 0);
    }

    #[test]
    fn rect_empty_sentinel() {
        // C 版习惯用 c2 < c1 表示空区域
        let empty = Rect::new(10, 10, 9, 12);
        assert!(empty.is_empty());
        assert_eq!(empty.width(), 0);
        assert_eq!(empty.area(), 0);
    }

    #[test]
    fn rect_contains_point() {
        let r = Rect::new(2, 2, 5, 5);
        assert!(r.contains_point(2, 2));
        assert!(r.contains_point(5, 5));
        assert!(r.contains_point(3, 4));
        assert!(!r.contains_point(1, 3));
        assert!(!r.contains_point(6, 3));
        assert!(!r.contains_point(3, 6));
    }

    #[test]
    fn rect_contains_rect_inclusive() {
        let outer = Rect::new(0, 0, 10, 10);
        let inner = Rect::new(2, 2, 8, 8);
        let edge = Rect::new(0, 0, 10, 10);
        let outside = Rect::new(5, 5, 11, 11);

        assert!(outer.contains_rect(inner));
        assert!(outer.contains_rect(edge));
        assert!(!outer.contains_rect(outside));
        // 空矩形被视为"包含在任何矩形内"
        assert!(outer.contains_rect(Rect::new(0, 0, -1, -1)));
    }

    #[test]
    fn rect_intersects_includes_edge() {
        let a = Rect::new(0, 0, 5, 5);
        let b = Rect::new(5, 5, 10, 10);
        // 端点重合也算相交（inclusive）
        assert!(a.intersects(b));
        let c = Rect::new(6, 6, 10, 10);
        assert!(!a.intersects(c));
    }

    #[test]
    fn rect_intersection_normal_and_empty() {
        let a = Rect::new(0, 0, 5, 5);
        let b = Rect::new(3, 3, 10, 10);
        let i = a.intersection(b);
        assert_eq!(i, Rect::new(3, 3, 5, 5));
        assert_eq!(i.area(), 9);

        let c = Rect::new(100, 100, 200, 200);
        let e = a.intersection(c);
        assert!(e.is_empty());
    }

    #[test]
    fn rect_union_includes_both() {
        let a = Rect::new(0, 0, 5, 5);
        let b = Rect::new(10, 10, 15, 15);
        let u = a.union(b);
        assert_eq!(u, Rect::new(0, 0, 15, 15));
    }

    #[test]
    fn rect_union_empty_passes_through() {
        let a = Rect::new(0, 0, 5, 5);
        let empty = Rect::new(0, 0, -1, -1);
        assert_eq!(a.union(empty), a);
        assert_eq!(empty.union(a), a);
    }

    #[test]
    fn rect_translate() {
        let r = Rect::new(1, 2, 3, 4);
        assert_eq!(r.translate(10, 20), Rect::new(11, 22, 13, 24));
        assert_eq!(r.translate(-1, -2), Rect::new(0, 0, 2, 2));
    }

    #[test]
    fn rect_clamp_to_bounds() {
        let r = Rect::new(-5, -5, 100, 100);
        let bounds = Rect::new(0, 0, 50, 50);
        assert_eq!(r.clamp_to(bounds), Rect::new(0, 0, 50, 50));

        // 完全在外
        let outside = Rect::new(60, 60, 70, 70);
        assert!(outside.clamp_to(bounds).is_empty());
    }
}
