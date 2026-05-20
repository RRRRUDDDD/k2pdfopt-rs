//! `BitmapPage` / `Bitmap` / `PixelFormat` —— DocumentRenderer 输出的栅格化页面数据。
//!
//! 设计来源：
//! - `rust-rewrite-plan.md` v2.1 §8.2（Bitmap、PixelFormat、Rect、Region 结构）
//! - `rust-rewrite-plan.md` v2.1 §9.2（DocumentRenderer trait + BitmapPage）
//! - `rust-rewrite-execution-plan.md` Step 4.1（本步骤）
//! - C 对照：`willuslib/willus.h` `WILLUSBITMAP` 结构
//!
//! 关于 `Bitmap` 归属：v2.1 §5.2 表格把 Bitmap 算法层归 `k2core`（Step 5.1）；
//! 但 §9.2 的 `BitmapPage` 字段类型直接是 `Bitmap`，且 `k2types` 是叶子 crate
//! （不应依赖 k2core）。为同时满足两端，本 crate 承载 **Bitmap 的纯数据结构定义**，
//! Step 5.1 在 `k2core` 通过自由函数 / `BitmapExt` trait 挂载算法，互不冲突。

use std::fmt;

/// 单像素的字节布局。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// 1 字节/像素，灰度
    Gray8,
    /// 3 字节/像素，RGB 顺序
    Rgb8,
    /// 4 字节/像素，RGBA 顺序（mutool PAM `TUPLTYPE RGB_ALPHA` 默认输出）
    Rgba8,
}

impl PixelFormat {
    /// 每像素字节数
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::Gray8 => 1,
            PixelFormat::Rgb8 => 3,
            PixelFormat::Rgba8 => 4,
        }
    }
}

/// 栅格位图（纯数据，无算法）。
///
/// 对应 C 版 `WILLUSBITMAP`（`willuslib/willus.h`）。算法（trim / bbox / preprocess /
/// deskew / autocrop）在 Step 5.1 起由 `k2core` 经 free fn 或 trait 挂载，避免本 crate
/// 引入算法循环依赖。
#[derive(Clone)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub dpi: f32,
    pub format: PixelFormat,
    pub pixels: Vec<u8>,
}

impl Bitmap {
    /// 创建一张全 0 的空白位图；宽高乘积溢出 `usize` 时返回 `BitmapError::SizeOverflow`。
    pub fn new(
        width: u32,
        height: u32,
        dpi: f32,
        format: PixelFormat,
    ) -> Result<Self, BitmapError> {
        let size = expected_pixel_len(width, height, format)?;
        Ok(Self {
            width,
            height,
            dpi,
            format,
            pixels: vec![0; size],
        })
    }

    /// 用已知像素 `Vec<u8>` 构造（不拷贝），并严格校验长度与宽高/格式匹配。
    pub fn from_raw(
        width: u32,
        height: u32,
        dpi: f32,
        format: PixelFormat,
        pixels: Vec<u8>,
    ) -> Result<Self, BitmapError> {
        let expected = expected_pixel_len(width, height, format)?;
        if pixels.len() != expected {
            return Err(BitmapError::PixelLenMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            dpi,
            format,
            pixels,
        })
    }

    /// 每行像素的字节数 = `width * bytes_per_pixel`。
    #[must_use]
    pub fn bytes_per_row(&self) -> usize {
        (self.width as usize) * self.format.bytes_per_pixel()
    }

    /// 总像素字节数（width * height * bpp）。
    #[must_use]
    pub fn total_bytes(&self) -> usize {
        self.bytes_per_row() * (self.height as usize)
    }

    /// 取第 `y` 行的只读像素切片。`y=0` 为最顶行，对应 C 版
    /// `bmp_rowptr_from_top(bmp, y)`（`willuslib/bmp.c:1416`，native 模式 top→bottom）。
    /// `y >= height` 时返回 `None`。
    #[must_use]
    pub fn row(&self, y: u32) -> Option<&[u8]> {
        if y >= self.height {
            return None;
        }
        let bpr = self.bytes_per_row();
        let start = (y as usize) * bpr;
        Some(&self.pixels[start..start + bpr])
    }

    /// 取第 `y` 行的可变像素切片，语义同 [`Bitmap::row`]。
    pub fn row_mut(&mut self, y: u32) -> Option<&mut [u8]> {
        if y >= self.height {
            return None;
        }
        let bpr = self.bytes_per_row();
        let start = (y as usize) * bpr;
        Some(&mut self.pixels[start..start + bpr])
    }

    /// 取 `(x, y)` 处的单个像素 byte slice（长度 = `bytes_per_pixel`）。
    /// 越界返回 `None`。
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<&[u8]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let bpp = self.format.bytes_per_pixel();
        let start = (y as usize) * self.bytes_per_row() + (x as usize) * bpp;
        Some(&self.pixels[start..start + bpp])
    }

    /// 取 `(x, y)` 处的单个像素 byte slice（可变）。
    pub fn pixel_mut(&mut self, x: u32, y: u32) -> Option<&mut [u8]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let bpp = self.format.bytes_per_pixel();
        let bpr = self.bytes_per_row();
        let start = (y as usize) * bpr + (x as usize) * bpp;
        Some(&mut self.pixels[start..start + bpp])
    }

    /// 把整个 bitmap 用单字节 `value` 填充。
    /// 对应 C 版 `bmp_fill(bmp, r, r, r)`（`willuslib/bmp.c:335`）的灰度路径。
    pub fn fill_byte(&mut self, value: u8) {
        self.pixels.fill(value);
    }

    /// 用 RGB 三元组填充整张位图。对应 C 版 `bmp_fill(bmp, r, g, b)`。
    /// `Gray8` 走 `0.299 R + 0.587 G + 0.114 B` 近似（与 C 版调色板设置等价的灰度回退）。
    pub fn fill_rgb(&mut self, r: u8, g: u8, b: u8) {
        match self.format {
            PixelFormat::Gray8 => {
                // 0.299 / 0.587 / 0.114 - 标准亮度系数，与 C 版 bmp_color_xform8 一致
                let lum = (0.299_f32 * f32::from(r)
                    + 0.587_f32 * f32::from(g)
                    + 0.114_f32 * f32::from(b))
                .round()
                .clamp(0.0, 255.0) as u8;
                self.pixels.fill(lum);
            }
            PixelFormat::Rgb8 => {
                for chunk in self.pixels.chunks_exact_mut(3) {
                    chunk[0] = r;
                    chunk[1] = g;
                    chunk[2] = b;
                }
            }
            PixelFormat::Rgba8 => {
                for chunk in self.pixels.chunks_exact_mut(4) {
                    chunk[0] = r;
                    chunk[1] = g;
                    chunk[2] = b;
                    chunk[3] = 255;
                }
            }
        }
    }

    /// 判断是否为"灰度"位图。
    ///
    /// - `Gray8`：恒为 `true`。
    /// - `Rgb8` / `Rgba8`：所有像素 R==G==B 时为 `true`。
    ///
    /// 对应 C 版 `bmp_is_grayscale`（`willuslib/bmp.c:3454`），但 C 版基于
    /// 调色板比较；Rust 版无调色板，直接扫描像素。
    #[must_use]
    pub fn is_grayscale(&self) -> bool {
        match self.format {
            PixelFormat::Gray8 => true,
            PixelFormat::Rgb8 => self
                .pixels
                .chunks_exact(3)
                .all(|c| c[0] == c[1] && c[1] == c[2]),
            PixelFormat::Rgba8 => self
                .pixels
                .chunks_exact(4)
                .all(|c| c[0] == c[1] && c[1] == c[2]),
        }
    }

    /// 读取 `(x, y)` 处的灰度值（0-255）。
    /// - `Gray8`：直接读 1 字节
    /// - `Rgb8` / `Rgba8`：按亮度公式 `0.299R + 0.587G + 0.114B` 计算
    ///
    /// 对应 C 版 `bmp_grey_pix_vali`（`willuslib/bmp.c:2367` 附近的整数版本）。
    /// 越界返回 `None`。
    #[must_use]
    pub fn gray_at(&self, x: u32, y: u32) -> Option<u8> {
        let px = self.pixel(x, y)?;
        Some(match self.format {
            PixelFormat::Gray8 => px[0],
            PixelFormat::Rgb8 | PixelFormat::Rgba8 => {
                ((0.299_f32 * f32::from(px[0])
                    + 0.587_f32 * f32::from(px[1])
                    + 0.114_f32 * f32::from(px[2]))
                .round()
                .clamp(0.0, 255.0)) as u8
            }
        })
    }
}

impl fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bitmap")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("dpi", &self.dpi)
            .field("format", &self.format)
            .field("pixels_len", &self.pixels.len())
            .finish()
    }
}

/// 一页栅格化数据 + 元信息。由 `DocumentRenderer::render_page` 产出，
/// 后续 layout pipeline（Step 5.x）消费。
#[derive(Debug, Clone)]
pub struct BitmapPage {
    /// 0-based 页号
    pub page_index: usize,
    /// 像素数据
    pub bitmap: Bitmap,
    /// 渲染使用的 DPI（典型 150~300）
    pub source_dpi: f32,
    /// 原始页面物理尺寸 `(width_pt, height_pt)`，1 pt = 1/72 inch
    pub source_size_pt: (f32, f32),
    /// 旋转角度（度），0 / 90 / 180 / 270 之一
    pub rotation: f32,
}

/// Bitmap 构造期的可恢复错误。
#[derive(Debug, thiserror::Error)]
pub enum BitmapError {
    /// 宽 × 高 × 每像素字节数 超出 `usize` 范围。
    #[error("bitmap size overflow: {width}x{height} fmt={format:?}")]
    SizeOverflow {
        width: u32,
        height: u32,
        format: PixelFormat,
    },
    /// 传入的 `pixels.len()` 与 `width * height * bpp` 不匹配。
    #[error("pixel length mismatch: expected {expected}, got {actual}")]
    PixelLenMismatch { expected: usize, actual: usize },
}

fn expected_pixel_len(width: u32, height: u32, format: PixelFormat) -> Result<usize, BitmapError> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(format.bytes_per_pixel()))
        .ok_or(BitmapError::SizeOverflow {
            width,
            height,
            format,
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn pixel_format_bytes_per_pixel() {
        assert_eq!(PixelFormat::Gray8.bytes_per_pixel(), 1);
        assert_eq!(PixelFormat::Rgb8.bytes_per_pixel(), 3);
        assert_eq!(PixelFormat::Rgba8.bytes_per_pixel(), 4);
    }

    #[test]
    fn bitmap_new_zero_filled() {
        let b = Bitmap::new(4, 3, 300.0, PixelFormat::Gray8).unwrap();
        assert_eq!(b.width, 4);
        assert_eq!(b.height, 3);
        assert_eq!(b.pixels.len(), 12);
        assert!(b.pixels.iter().all(|&p| p == 0));
        assert_eq!(b.bytes_per_row(), 4);
        assert_eq!(b.total_bytes(), 12);
    }

    #[test]
    fn bitmap_new_rgba_sizing() {
        let b = Bitmap::new(10, 10, 150.0, PixelFormat::Rgba8).unwrap();
        assert_eq!(b.pixels.len(), 400);
        assert_eq!(b.bytes_per_row(), 40);
    }

    #[test]
    fn bitmap_from_raw_strict_length() {
        let good = vec![1u8; 9];
        let b = Bitmap::from_raw(3, 3, 300.0, PixelFormat::Gray8, good).unwrap();
        assert_eq!(b.pixels.len(), 9);

        let too_short = vec![1u8; 8];
        let err = Bitmap::from_raw(3, 3, 300.0, PixelFormat::Gray8, too_short).unwrap_err();
        assert!(matches!(
            err,
            BitmapError::PixelLenMismatch {
                expected: 9,
                actual: 8
            }
        ));
    }

    #[test]
    fn bitmap_size_overflow_detected() {
        // u32::MAX * u32::MAX 在 64-bit usize 内合法（≈ 1.84e19，未超 usize::MAX）。
        // 必须再乘 4（Rgba8）才能让 checked_mul 触发 SizeOverflow。
        let err = Bitmap::new(u32::MAX, u32::MAX, 1.0, PixelFormat::Rgba8).unwrap_err();
        assert!(matches!(err, BitmapError::SizeOverflow { .. }));

        // from_raw 同样路径触发 SizeOverflow（不会真正分配 Vec）。
        let err =
            Bitmap::from_raw(u32::MAX, u32::MAX, 1.0, PixelFormat::Rgba8, Vec::new()).unwrap_err();
        assert!(matches!(err, BitmapError::SizeOverflow { .. }));
    }

    #[test]
    fn bitmap_debug_does_not_dump_pixels() {
        let b = Bitmap::new(2, 2, 300.0, PixelFormat::Gray8).unwrap();
        let s = format!("{b:?}");
        assert!(s.contains("pixels_len: 4"));
        assert!(!s.contains("[0, 0, 0, 0]"));
    }

    #[test]
    fn bitmap_page_basic_construction() {
        let bitmap = Bitmap::new(100, 50, 200.0, PixelFormat::Rgb8).unwrap();
        let page = BitmapPage {
            page_index: 7,
            bitmap,
            source_dpi: 200.0,
            source_size_pt: (595.0, 842.0),
            rotation: 0.0,
        };
        assert_eq!(page.page_index, 7);
        assert_eq!(page.bitmap.width, 100);
        assert_eq!(page.source_size_pt, (595.0, 842.0));
    }

    #[test]
    fn bitmap_row_top_to_bottom() {
        // 5x3 Gray8, 行 0..2 共 3 行
        let mut b = Bitmap::new(5, 3, 100.0, PixelFormat::Gray8).unwrap();
        for y in 0..3u32 {
            let row = b.row_mut(y).unwrap();
            for cell in row.iter_mut() {
                *cell = (y + 1) as u8;
            }
        }
        // 顶行 y=0 -> 全 1
        assert_eq!(b.row(0).unwrap(), &[1, 1, 1, 1, 1]);
        // 中行 y=1 -> 全 2
        assert_eq!(b.row(1).unwrap(), &[2, 2, 2, 2, 2]);
        // 底行 y=2 -> 全 3
        assert_eq!(b.row(2).unwrap(), &[3, 3, 3, 3, 3]);
        // 越界
        assert!(b.row(3).is_none());
        assert!(b.row_mut(99).is_none());
    }

    #[test]
    fn bitmap_pixel_access() {
        let mut b = Bitmap::new(4, 2, 72.0, PixelFormat::Rgb8).unwrap();
        b.pixel_mut(1, 0).unwrap().copy_from_slice(&[10, 20, 30]);
        b.pixel_mut(3, 1).unwrap().copy_from_slice(&[40, 50, 60]);
        assert_eq!(b.pixel(1, 0).unwrap(), &[10, 20, 30]);
        assert_eq!(b.pixel(3, 1).unwrap(), &[40, 50, 60]);
        assert_eq!(b.pixel(0, 0).unwrap(), &[0, 0, 0]);
        // 越界
        assert!(b.pixel(4, 0).is_none());
        assert!(b.pixel(0, 2).is_none());
    }

    #[test]
    fn bitmap_fill_byte_uniform() {
        let mut b = Bitmap::new(3, 3, 72.0, PixelFormat::Gray8).unwrap();
        b.fill_byte(0xAB);
        assert!(b.pixels.iter().all(|&p| p == 0xAB));
    }

    #[test]
    fn bitmap_fill_rgb_rgb8() {
        let mut b = Bitmap::new(2, 2, 72.0, PixelFormat::Rgb8).unwrap();
        b.fill_rgb(255, 0, 0);
        for chunk in b.pixels.chunks_exact(3) {
            assert_eq!(chunk, &[255, 0, 0]);
        }
    }

    #[test]
    fn bitmap_fill_rgb_rgba_sets_alpha_255() {
        let mut b = Bitmap::new(2, 2, 72.0, PixelFormat::Rgba8).unwrap();
        b.fill_rgb(11, 22, 33);
        for chunk in b.pixels.chunks_exact(4) {
            assert_eq!(chunk, &[11, 22, 33, 255]);
        }
    }

    #[test]
    fn bitmap_fill_rgb_gray8_luminance_blend() {
        let mut b = Bitmap::new(2, 1, 72.0, PixelFormat::Gray8).unwrap();
        // 纯红：0.299 * 255 ≈ 76
        b.fill_rgb(255, 0, 0);
        assert_eq!(b.pixels, vec![76, 76]);
        // 白色：所有通道 255 → 255
        b.fill_rgb(255, 255, 255);
        assert_eq!(b.pixels, vec![255, 255]);
        // 黑色 → 0
        b.fill_rgb(0, 0, 0);
        assert_eq!(b.pixels, vec![0, 0]);
    }

    #[test]
    fn bitmap_is_grayscale_detection() {
        // Gray8 恒为 true
        let g = Bitmap::new(2, 2, 72.0, PixelFormat::Gray8).unwrap();
        assert!(g.is_grayscale());
        // Rgb8 R==G==B → true
        let mut rgb = Bitmap::new(2, 1, 72.0, PixelFormat::Rgb8).unwrap();
        rgb.fill_rgb(128, 128, 128);
        assert!(rgb.is_grayscale());
        // Rgb8 有彩色像素 → false
        rgb.pixel_mut(0, 0).unwrap().copy_from_slice(&[255, 0, 0]);
        assert!(!rgb.is_grayscale());
    }

    #[test]
    fn bitmap_gray_at_works_for_all_formats() {
        let mut g = Bitmap::new(2, 1, 72.0, PixelFormat::Gray8).unwrap();
        g.pixel_mut(0, 0).unwrap()[0] = 123;
        assert_eq!(g.gray_at(0, 0), Some(123));

        let mut rgb = Bitmap::new(2, 1, 72.0, PixelFormat::Rgb8).unwrap();
        rgb.pixel_mut(0, 0).unwrap().copy_from_slice(&[255, 0, 0]);
        // 0.299 * 255 ≈ 76.245 → 76
        assert_eq!(rgb.gray_at(0, 0), Some(76));

        let mut rgba = Bitmap::new(2, 1, 72.0, PixelFormat::Rgba8).unwrap();
        rgba.pixel_mut(0, 0)
            .unwrap()
            .copy_from_slice(&[0, 255, 0, 255]);
        // 0.587 * 255 ≈ 149.685 → 150
        assert_eq!(rgba.gray_at(0, 0), Some(150));

        // 越界
        assert_eq!(rgb.gray_at(5, 0), None);
    }
}
