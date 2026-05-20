//! `RegionView` —— 位图上的"矩形区域"视图，对应 C 版 `BMPREGION`。
//!
//! ## 语义对齐
//!
//! C 版 `BMPREGION`（`k2pdfopt.h:594-620`）字段：
//!
//! ```c
//! typedef struct {
//!     int r1,r2;          /* row position from top of bmp, inclusive */
//!     int c1,c2;          /* column positions, inclusive */
//!     int rotdeg;
//!     int dpi;
//!     unsigned char bgcolor; /* 0 - 255 */
//!     WILLUSBITMAP *bmp;
//!     WILLUSBITMAP *bmp8;
//!     ...
//! } BMPREGION;
//! ```
//!
//! Rust 版 [`RegionView`] 用**借用**（`&'a Bitmap`）替代 C 版的指针，且不区分
//! `bmp`/`bmp8`（C 版分别是彩色 / 8-bit 灰度的双缓冲；Rust 端 [`Bitmap`] 已含
//! `format` 字段、`gray_at()` 可直接读灰度，无须双指针）。
//!
//! ## 与 C 版的差异
//!
//! - 不持有 `colcount` / `rowcount` 内存（C 版用裸指针并按需 `willus_dmem_alloc`）。
//!   Rust 版改为：算法函数（[`crate::crop::calc_bbox`] 等）拿 `&RegionView` 算完
//!   返回新 `Vec<i32>`，由调用方决定生命周期，避免可变借用冲突
//! - 不携带 `textrows` / `wrectmaps` / `k2pagebreakmarks`：这些是上层
//!   layout/master 阶段的状态，本结构保持"纯几何视图"
//! - `rotdeg` 字段省略：旋转在更早的 `k2render::MutoolRenderer` 阶段已处理
//!
//! 来源：`rust-rewrite-execution-plan.md` Step 5.2 输出清单。

use k2core::Rect;
use k2types::Bitmap;

/// "白色阈值"默认值。对应 C 版 `k2bmp.c` 中 `bgcolor` 初值（详见
/// `k2pdfopt_settings_init` 的间接路径）。值 = 255 表示"严格 255 才算白"。
pub const DEFAULT_BGCOLOR: u8 = 255;

/// 位图区域视图（不可变借用）。
///
/// 对应 C 版 `BMPREGION` 的"几何 + 颜色阈值"子集，由算法函数消费、
/// 不持有可变状态。
#[derive(Copy, Clone, Debug)]
pub struct RegionView<'a> {
    /// 底层位图（不可变借用）
    pub bmp: &'a Bitmap,
    /// 区域裁切矩形（inclusive，对应 C `c1/c2/r1/r2`）
    pub rect: Rect,
    /// 区域 DPI（C 版 `region->dpi`）。常等于 `bmp.dpi`，但允许独立设置
    pub dpi: f32,
    /// "黑像素阈值"：灰度 `< bgcolor` 视为黑/前景（C `p[0] < region->bgcolor`）。
    /// 默认 255，等价于 C 版 `WHITETHRESH` 的硬阈值语义
    pub bgcolor: u8,
}

impl<'a> RegionView<'a> {
    /// 构造覆盖整张位图的视图，DPI 取自位图、bgcolor 默认 255。
    #[must_use]
    pub fn full(bmp: &'a Bitmap) -> Self {
        let rect = Rect::from_xywh(0, 0, bmp.width, bmp.height);
        Self {
            bmp,
            rect,
            dpi: bmp.dpi,
            bgcolor: DEFAULT_BGCOLOR,
        }
    }

    /// 用指定矩形构造视图，DPI 和 bgcolor 取默认。
    #[must_use]
    pub fn new(bmp: &'a Bitmap, rect: Rect) -> Self {
        Self {
            bmp,
            rect,
            dpi: bmp.dpi,
            bgcolor: DEFAULT_BGCOLOR,
        }
    }

    /// 构造 + 显式设置全部字段。
    #[must_use]
    pub fn with(bmp: &'a Bitmap, rect: Rect, dpi: f32, bgcolor: u8) -> Self {
        Self {
            bmp,
            rect,
            dpi,
            bgcolor,
        }
    }

    /// 返回区域宽度（inclusive：`x1 - x0 + 1`）。空矩形返回 0。
    #[must_use]
    pub fn width(&self) -> u32 {
        self.rect.width()
    }

    /// 返回区域高度。
    #[must_use]
    pub fn height(&self) -> u32 {
        self.rect.height()
    }

    /// 区域是否完全在位图范围内（含右下边界，inclusive）。
    ///
    /// 对应 C 版 `bmpregion_calc_bbox` 入口处的越界检测（行 484-494）。
    #[must_use]
    pub fn is_in_bounds(&self) -> bool {
        let bw = self.bmp.width as i32;
        let bh = self.bmp.height as i32;
        self.rect.x0 >= 0
            && self.rect.y0 >= 0
            && self.rect.x1 < bw
            && self.rect.y1 < bh
            && self.rect.x0 <= self.rect.x1
            && self.rect.y0 <= self.rect.y1
    }

    /// 返回一个新视图，更换裁切矩形但共享其它字段。
    #[must_use]
    pub fn with_rect(self, rect: Rect) -> Self {
        Self { rect, ..self }
    }

    /// 读 `(x, y)` 处的灰度（RGB/RGBA 用 BT.601 加权）。
    /// `(x, y)` 超出位图范围时返回 255（默认背景白）。
    #[must_use]
    pub fn gray_at(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 {
            return 255;
        }
        let bw = self.bmp.width as i32;
        let bh = self.bmp.height as i32;
        if x >= bw || y >= bh {
            return 255;
        }
        self.bmp.gray_at(x as u32, y as u32).unwrap_or(255)
    }

    /// 判断 `(x, y)` 是否为"黑像素"（前景）。
    /// 即 `gray_at(x, y) < bgcolor`，对应 C `p[0] < region->bgcolor`。
    #[must_use]
    pub fn is_dark(&self, x: i32, y: i32) -> bool {
        self.gray_at(x, y) < self.bgcolor
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use k2types::PixelFormat;

    fn make_white_bmp(w: u32, h: u32) -> Bitmap {
        let mut bmp = Bitmap::new(w, h, 150.0, PixelFormat::Gray8).unwrap();
        bmp.fill_byte(255);
        bmp
    }

    #[test]
    fn full_view_covers_entire_bitmap() {
        let bmp = make_white_bmp(10, 6);
        let view = RegionView::full(&bmp);
        assert_eq!(view.rect, Rect::new(0, 0, 9, 5));
        assert_eq!(view.width(), 10);
        assert_eq!(view.height(), 6);
        assert!(view.is_in_bounds());
        assert!((view.dpi - 150.0).abs() < 1e-6);
        assert_eq!(view.bgcolor, DEFAULT_BGCOLOR);
    }

    #[test]
    fn new_uses_custom_rect() {
        let bmp = make_white_bmp(20, 20);
        let view = RegionView::new(&bmp, Rect::new(2, 3, 11, 15));
        assert_eq!(view.width(), 10);
        assert_eq!(view.height(), 13);
        assert!(view.is_in_bounds());
    }

    #[test]
    fn out_of_bounds_detected() {
        let bmp = make_white_bmp(5, 5);
        let bad = RegionView::new(&bmp, Rect::new(0, 0, 5, 4));
        assert!(!bad.is_in_bounds(), "x1=5 越界 (width=5 -> max=4)");
        let neg = RegionView::new(&bmp, Rect::new(-1, 0, 4, 4));
        assert!(!neg.is_in_bounds(), "x0<0 越界");
        let inverted = RegionView::new(&bmp, Rect::new(3, 0, 2, 4));
        assert!(!inverted.is_in_bounds(), "x0>x1 空矩形");
    }

    #[test]
    fn with_rect_returns_new_view() {
        let bmp = make_white_bmp(10, 10);
        let v1 = RegionView::full(&bmp);
        let v2 = v1.with_rect(Rect::new(1, 1, 5, 5));
        assert_eq!(v1.rect, Rect::new(0, 0, 9, 9));
        assert_eq!(v2.rect, Rect::new(1, 1, 5, 5));
        assert_eq!(v2.bgcolor, v1.bgcolor);
        assert!((v2.dpi - v1.dpi).abs() < 1e-6);
    }

    #[test]
    fn gray_at_returns_white_for_out_of_bounds() {
        let bmp = make_white_bmp(3, 3);
        let view = RegionView::full(&bmp);
        assert_eq!(view.gray_at(-1, 0), 255);
        assert_eq!(view.gray_at(0, -1), 255);
        assert_eq!(view.gray_at(3, 0), 255);
        assert_eq!(view.gray_at(0, 3), 255);
        assert_eq!(view.gray_at(1, 1), 255);
    }

    #[test]
    fn is_dark_uses_bgcolor_threshold() {
        let mut bmp = make_white_bmp(3, 3);
        // 中心 (1,1) 设为黑
        let px = bmp.pixel_mut(1, 1).unwrap();
        px[0] = 0;
        let view = RegionView::full(&bmp);
        assert!(view.is_dark(1, 1));
        assert!(!view.is_dark(0, 0));
        // 阈值改为 0 → 没有"黑像素"
        let view2 = RegionView::with(&bmp, Rect::from_xywh(0, 0, 3, 3), 150.0, 0);
        assert!(!view2.is_dark(1, 1));
        // 阈值改为 128 → 0 < 128，仍是黑
        let view3 = RegionView::with(&bmp, Rect::from_xywh(0, 0, 3, 3), 150.0, 128);
        assert!(view3.is_dark(1, 1));
        assert!(!view3.is_dark(0, 0)); // 255 >= 128 不是黑
    }
}
