//! 直方图 / 投影 —— layout 模块（行 / 列检测）的基础数据。
//!
//! ## 为什么用 `u64` 而不是 `u32`
//!
//! C 版（[`willuslib/bmp.c`] / [`k2pdfoptlib/bmpregion.c`]）用 `int` 存累计值，
//! 30000×30000 像素 × 255 = 2.3e11 远超 `i32::MAX` (2.1e9)。codex 复核要求
//! Rust 版用 `u64` 防溢出（v2.1 §8.2 + Step 5.1 操作清单第 4 条）。
//!
//! ## 提供两类投影
//!
//! - **亮度投影**：把每行 / 每列的灰度值累加（用于找连续暗色块 ≈ 文本行）
//! - **暗像素计数**：每行 / 每列内 "暗像素" (gray < threshold) 的个数
//!   （对应 C 版 `bmpregion_one_row_find_textrows` 等的 `row_count`/`col_count`）
//!
//! 算法层（找峰值 / 阈值化 / 平滑）放在 Step 5.2+ 的 `k2layout`，本步只做基础结构。

use k2types::{Bitmap, PixelFormat};

use crate::rect::Rect;

/// 直方图：固定长度的 `u64` 桶序列。
///
/// 典型用法：
/// ```
/// use k2core::histogram::Histogram;
/// let mut h = Histogram::new(5);
/// h.set(0, 10);
/// h.add(0, 5);
/// assert_eq!(h.get(0), 15);
/// assert_eq!(h.len(), 5);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Histogram {
    buckets: Vec<u64>,
}

impl Histogram {
    /// 创建长度为 `len` 的全 0 直方图。
    #[must_use]
    pub fn new(len: usize) -> Self {
        Self {
            buckets: vec![0; len],
        }
    }

    /// 桶数（长度）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    /// 是否为空（长度 0）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    /// 读取第 `i` 个桶。越界 panic（用户责任）。
    #[must_use]
    pub fn get(&self, i: usize) -> u64 {
        self.buckets[i]
    }

    /// 设置第 `i` 个桶。越界 panic（用户责任）。
    pub fn set(&mut self, i: usize, v: u64) {
        self.buckets[i] = v;
    }

    /// 第 `i` 个桶累加 `v`，使用 `saturating_add` 防止溢出。
    pub fn add(&mut self, i: usize, v: u64) {
        self.buckets[i] = self.buckets[i].saturating_add(v);
    }

    /// 整个直方图各桶累加 1（在简单 count 场景下用）。
    pub fn inc(&mut self, i: usize) {
        self.add(i, 1);
    }

    /// 返回 `(峰值索引, 峰值)`。直方图空时返回 `None`。
    /// 多个相同最大值时返回**第一个**遇到的索引（与 C 版相同）。
    #[must_use]
    pub fn argmax(&self) -> Option<(usize, u64)> {
        let mut iter = self.buckets.iter().copied().enumerate();
        let mut best = iter.next()?;
        for (i, v) in iter {
            // 严格大于才更新，保证相等时保留更小的索引
            if v > best.1 {
                best = (i, v);
            }
        }
        Some(best)
    }

    /// 返回最大值。直方图空时返回 0。
    #[must_use]
    pub fn max(&self) -> u64 {
        self.buckets.iter().copied().max().unwrap_or(0)
    }

    /// 返回最小值。直方图空时返回 0。
    #[must_use]
    pub fn min(&self) -> u64 {
        self.buckets.iter().copied().min().unwrap_or(0)
    }

    /// 所有桶之和（saturating）。
    #[must_use]
    pub fn sum(&self) -> u64 {
        self.buckets
            .iter()
            .copied()
            .fold(0u64, |a, b| a.saturating_add(b))
    }

    /// 平均值（仅整数部分）。空时返回 0。
    #[must_use]
    pub fn mean(&self) -> u64 {
        if self.buckets.is_empty() {
            return 0;
        }
        self.sum() / (self.buckets.len() as u64)
    }

    /// 取桶切片（只读）。
    #[must_use]
    pub fn buckets(&self) -> &[u64] {
        &self.buckets
    }

    /// 取桶切片（可变）。
    pub fn buckets_mut(&mut self) -> &mut [u64] {
        &mut self.buckets
    }
}

/// 矩形交集裁剪：把 `rect` 钳制到 `[0, width-1] x [0, height-1]` 内。
/// 全部越界或负数会得到 `is_empty()==true`。
fn clip_rect_to_bitmap(rect: Rect, width: u32, height: u32) -> Rect {
    let canvas = Rect::new(0, 0, (width as i32) - 1, (height as i32) - 1);
    rect.clamp_to(canvas)
}

/// 横向投影：沿 X 方向求和，结果长度 = 矩形高度。
///
/// `buckets[k]` 是 `rect` 的第 `k` 行（自顶向下）灰度值之和。
/// 对应 C 版 `bmpregion_one_row_find_textrows` 中的 `row_count`（按行积分）。
///
/// `rect` 越出 bitmap 边界时自动钳制；钳制后为空则返回空直方图。
#[must_use]
pub fn horizontal_projection(bitmap: &Bitmap, rect: Rect) -> Histogram {
    let clipped = clip_rect_to_bitmap(rect, bitmap.width, bitmap.height);
    let h = clipped.height();
    let mut hist = Histogram::new(h as usize);
    if clipped.is_empty() {
        return hist;
    }
    let bpp = bitmap.format.bytes_per_pixel();
    let bpr = bitmap.bytes_per_row();
    let x0 = clipped.x0 as usize;
    let x1 = clipped.x1 as usize;
    for k in 0..h as usize {
        let y = (clipped.y0 as usize) + k;
        let row_start = y * bpr;
        let mut sum: u64 = 0;
        match bitmap.format {
            PixelFormat::Gray8 => {
                for x in x0..=x1 {
                    sum = sum.saturating_add(u64::from(bitmap.pixels[row_start + x]));
                }
            }
            PixelFormat::Rgb8 | PixelFormat::Rgba8 => {
                for x in x0..=x1 {
                    let off = row_start + x * bpp;
                    let r = u32::from(bitmap.pixels[off]);
                    let g = u32::from(bitmap.pixels[off + 1]);
                    let b = u32::from(bitmap.pixels[off + 2]);
                    // 0.299 / 0.587 / 0.114 标准亮度（与 Bitmap::gray_at 一致）
                    let lum = (299 * r + 587 * g + 114 * b + 500) / 1000;
                    sum = sum.saturating_add(u64::from(lum));
                }
            }
        }
        hist.set(k, sum);
    }
    hist
}

/// 纵向投影：沿 Y 方向求和，结果长度 = 矩形宽度。
///
/// `buckets[k]` 是 `rect` 的第 `k` 列（自左向右）灰度值之和。
/// 对应 C 版 `pageregions_find_columns` 中的列直方图。
#[must_use]
pub fn vertical_projection(bitmap: &Bitmap, rect: Rect) -> Histogram {
    let clipped = clip_rect_to_bitmap(rect, bitmap.width, bitmap.height);
    let w = clipped.width();
    let mut hist = Histogram::new(w as usize);
    if clipped.is_empty() {
        return hist;
    }
    let bpp = bitmap.format.bytes_per_pixel();
    let bpr = bitmap.bytes_per_row();
    let y0 = clipped.y0 as usize;
    let y1 = clipped.y1 as usize;
    for k in 0..w as usize {
        let x = (clipped.x0 as usize) + k;
        let mut sum: u64 = 0;
        match bitmap.format {
            PixelFormat::Gray8 => {
                for y in y0..=y1 {
                    sum = sum.saturating_add(u64::from(bitmap.pixels[y * bpr + x]));
                }
            }
            PixelFormat::Rgb8 | PixelFormat::Rgba8 => {
                for y in y0..=y1 {
                    let off = y * bpr + x * bpp;
                    let r = u32::from(bitmap.pixels[off]);
                    let g = u32::from(bitmap.pixels[off + 1]);
                    let b = u32::from(bitmap.pixels[off + 2]);
                    let lum = (299 * r + 587 * g + 114 * b + 500) / 1000;
                    sum = sum.saturating_add(u64::from(lum));
                }
            }
        }
        hist.set(k, sum);
    }
    hist
}

/// 横向"暗像素计数"投影：对每行计 `gray < threshold` 的像素数。
///
/// 对应 C 版 `bmpregion_one_row_find_textrows` 中按 `whitethresh` 计的 row_count
/// （行内黑像素个数，用于判断"是否有文字"）。
#[must_use]
pub fn horizontal_dark_count(bitmap: &Bitmap, rect: Rect, threshold: u8) -> Histogram {
    let clipped = clip_rect_to_bitmap(rect, bitmap.width, bitmap.height);
    let h = clipped.height();
    let mut hist = Histogram::new(h as usize);
    if clipped.is_empty() {
        return hist;
    }
    let bpp = bitmap.format.bytes_per_pixel();
    let bpr = bitmap.bytes_per_row();
    let x0 = clipped.x0 as usize;
    let x1 = clipped.x1 as usize;
    for k in 0..h as usize {
        let y = (clipped.y0 as usize) + k;
        let row_start = y * bpr;
        let mut count: u64 = 0;
        match bitmap.format {
            PixelFormat::Gray8 => {
                for x in x0..=x1 {
                    if bitmap.pixels[row_start + x] < threshold {
                        count += 1;
                    }
                }
            }
            PixelFormat::Rgb8 | PixelFormat::Rgba8 => {
                for x in x0..=x1 {
                    let off = row_start + x * bpp;
                    let r = u32::from(bitmap.pixels[off]);
                    let g = u32::from(bitmap.pixels[off + 1]);
                    let b = u32::from(bitmap.pixels[off + 2]);
                    let lum = ((299 * r + 587 * g + 114 * b + 500) / 1000) as u8;
                    if lum < threshold {
                        count += 1;
                    }
                }
            }
        }
        hist.set(k, count);
    }
    hist
}

/// 纵向"暗像素计数"投影：对每列计 `gray < threshold` 的像素数。
#[must_use]
pub fn vertical_dark_count(bitmap: &Bitmap, rect: Rect, threshold: u8) -> Histogram {
    let clipped = clip_rect_to_bitmap(rect, bitmap.width, bitmap.height);
    let w = clipped.width();
    let mut hist = Histogram::new(w as usize);
    if clipped.is_empty() {
        return hist;
    }
    let bpp = bitmap.format.bytes_per_pixel();
    let bpr = bitmap.bytes_per_row();
    let y0 = clipped.y0 as usize;
    let y1 = clipped.y1 as usize;
    for k in 0..w as usize {
        let x = (clipped.x0 as usize) + k;
        let mut count: u64 = 0;
        match bitmap.format {
            PixelFormat::Gray8 => {
                for y in y0..=y1 {
                    if bitmap.pixels[y * bpr + x] < threshold {
                        count += 1;
                    }
                }
            }
            PixelFormat::Rgb8 | PixelFormat::Rgba8 => {
                for y in y0..=y1 {
                    let off = y * bpr + x * bpp;
                    let r = u32::from(bitmap.pixels[off]);
                    let g = u32::from(bitmap.pixels[off + 1]);
                    let b = u32::from(bitmap.pixels[off + 2]);
                    let lum = ((299 * r + 587 * g + 114 * b + 500) / 1000) as u8;
                    if lum < threshold {
                        count += 1;
                    }
                }
            }
        }
        hist.set(k, count);
    }
    hist
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use k2types::{Bitmap, PixelFormat};

    #[test]
    fn histogram_new_zero_init() {
        let h = Histogram::new(5);
        assert_eq!(h.len(), 5);
        assert_eq!(h.buckets(), &[0, 0, 0, 0, 0]);
        assert!(!h.is_empty());
        assert_eq!(h.sum(), 0);
    }

    #[test]
    fn histogram_empty_metadata() {
        let h = Histogram::new(0);
        assert!(h.is_empty());
        assert_eq!(h.max(), 0);
        assert_eq!(h.min(), 0);
        assert_eq!(h.sum(), 0);
        assert_eq!(h.mean(), 0);
        assert!(h.argmax().is_none());
    }

    #[test]
    fn histogram_get_set_add_inc() {
        let mut h = Histogram::new(3);
        h.set(0, 10);
        h.set(2, 5);
        h.add(0, 5);
        h.inc(1);
        h.inc(1);
        assert_eq!(h.buckets(), &[15, 2, 5]);
    }

    #[test]
    fn histogram_saturating_add() {
        let mut h = Histogram::new(1);
        h.set(0, u64::MAX - 10);
        h.add(0, 100);
        assert_eq!(h.get(0), u64::MAX); // saturating, 不 overflow
    }

    #[test]
    fn histogram_argmax_first_match() {
        let mut h = Histogram::new(5);
        h.set(0, 3);
        h.set(1, 7);
        h.set(2, 7); // 第二个 7
        h.set(3, 4);
        h.set(4, 7); // 第三个 7
        let (idx, val) = h.argmax().unwrap();
        assert_eq!(idx, 1); // 第一个最大值的索引
        assert_eq!(val, 7);
    }

    #[test]
    fn histogram_stats_basic() {
        let mut h = Histogram::new(4);
        h.set(0, 1);
        h.set(1, 2);
        h.set(2, 3);
        h.set(3, 4);
        assert_eq!(h.sum(), 10);
        assert_eq!(h.max(), 4);
        assert_eq!(h.min(), 1);
        assert_eq!(h.mean(), 2); // 10/4 = 2 (整数除法)
    }

    fn build_gray_bitmap(width: u32, height: u32, pattern: &[u8]) -> Bitmap {
        // pattern.len() 必须等于 width*height
        assert_eq!(pattern.len(), (width * height) as usize);
        Bitmap::from_raw(width, height, 100.0, PixelFormat::Gray8, pattern.to_vec()).unwrap()
    }

    #[test]
    fn horizontal_projection_gray8() {
        // 3x2 Gray8: 行0=[10,20,30], 行1=[5,5,5]
        let bmp = build_gray_bitmap(3, 2, &[10, 20, 30, 5, 5, 5]);
        let h = horizontal_projection(&bmp, Rect::new(0, 0, 2, 1));
        assert_eq!(h.len(), 2);
        assert_eq!(h.get(0), 60); // 10+20+30
        assert_eq!(h.get(1), 15); // 5+5+5
    }

    #[test]
    fn vertical_projection_gray8() {
        // 3x2 Gray8: col0=[10,5], col1=[20,5], col2=[30,5]
        let bmp = build_gray_bitmap(3, 2, &[10, 20, 30, 5, 5, 5]);
        let v = vertical_projection(&bmp, Rect::new(0, 0, 2, 1));
        assert_eq!(v.len(), 3);
        assert_eq!(v.get(0), 15);
        assert_eq!(v.get(1), 25);
        assert_eq!(v.get(2), 35);
    }

    #[test]
    fn projection_sub_rect() {
        let bmp = build_gray_bitmap(4, 3, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        // 取中间 2x1: x=[1,2], y=1 -> 6, 7
        let h = horizontal_projection(&bmp, Rect::new(1, 1, 2, 1));
        assert_eq!(h.len(), 1);
        assert_eq!(h.get(0), 13);
        let v = vertical_projection(&bmp, Rect::new(1, 1, 2, 1));
        assert_eq!(v.len(), 2);
        assert_eq!(v.get(0), 6);
        assert_eq!(v.get(1), 7);
    }

    #[test]
    fn projection_clamps_negative_rect() {
        let bmp = build_gray_bitmap(2, 2, &[1, 2, 3, 4]);
        // 矩形左/上超出，自动钳制到 (0,0)-(1,1)
        let h = horizontal_projection(&bmp, Rect::new(-5, -5, 10, 10));
        assert_eq!(h.len(), 2);
        assert_eq!(h.get(0), 3); // 1+2
        assert_eq!(h.get(1), 7); // 3+4
    }

    #[test]
    fn projection_empty_when_disjoint() {
        let bmp = build_gray_bitmap(2, 2, &[1, 2, 3, 4]);
        let h = horizontal_projection(&bmp, Rect::new(100, 100, 200, 200));
        assert!(h.is_empty());
    }

    #[test]
    fn horizontal_dark_count_threshold() {
        // 4x1 Gray8: [10, 100, 200, 250]
        let bmp = build_gray_bitmap(4, 1, &[10, 100, 200, 250]);
        // threshold=128 -> 暗像素是 < 128: 10, 100 → 2 个
        let h = horizontal_dark_count(&bmp, Rect::new(0, 0, 3, 0), 128);
        assert_eq!(h.len(), 1);
        assert_eq!(h.get(0), 2);
    }

    #[test]
    fn vertical_dark_count_threshold() {
        // 1x4 Gray8: [10, 100, 200, 250]  (一列 4 行)
        let bmp = build_gray_bitmap(1, 4, &[10, 100, 200, 250]);
        let v = vertical_dark_count(&bmp, Rect::new(0, 0, 0, 3), 128);
        assert_eq!(v.len(), 1);
        assert_eq!(v.get(0), 2);
    }

    #[test]
    fn projection_rgb_luminance() {
        // 1x1 Rgb8 像素 [255, 0, 0] 红色：亮度 ≈ 76
        let bmp = Bitmap::from_raw(1, 1, 100.0, PixelFormat::Rgb8, vec![255, 0, 0]).unwrap();
        let h = horizontal_projection(&bmp, Rect::new(0, 0, 0, 0));
        // (299*255 + 0 + 0 + 500) / 1000 = (76245+500)/1000 = 76 (truncating div)
        assert_eq!(h.get(0), 76);
    }
}
