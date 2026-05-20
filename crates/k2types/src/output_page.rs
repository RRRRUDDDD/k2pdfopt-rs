//! `OutputPage` —— PDF writer 接收的单页输出契约。
//!
//! 设计来源：
//! - `rust-rewrite-plan.md` v2.1 §9.4（PdfWriter trait 签名 `add_page(&OutputPage)`）
//! - `rust-rewrite-execution-plan.md` Step 7.2（本步骤）
//! - C 对照：`willuslib/pdfwrite.c` `pdffile_add_bitmap` / `pdffile_add_bitmap_with_ocrwords`
//!   的入参集合（`WILLUSBITMAP *bmp, double dpi, int quality, int halfsize`）
//!
//! # 与 [`crate::BitmapPage`] 的区别
//!
//! | 类型 | 用途 | 来源 | 消费者 |
//! |------|------|------|--------|
//! | [`BitmapPage`] | 源 PDF 渲染输出 | `DocumentRenderer::render_page` | layout 流水线 |
//! | [`OutputPage`] | 经版面分析、reflow 后**待写入输出 PDF** 的页 | `k2layout::flush_page` | `PdfWriter::add_page` |
//!
//! `OutputPage` 携带 PDF writer 需要的全部信息：bitmap 数据 + 输出 DPI（决定页面物理尺寸）
//! + JPEG 质量 + halfsize 位深（C 版四种 1/2/4/8 bits-per-component 选项）。
//!
//! # 与 `k2layout::PaginatorPage` 的区别
//!
//! `k2layout::PaginatorPage`（Step 7.1 落地）是 master canvas 内部 flush 队列的临时形态，
//! 不含 DPI / JPEG 控制字段。Step 7.3 在 `ConvertContext::flush_page` 串联时把
//! `PaginatorPage` 与 settings 合并为 `OutputPage` 喂给 `PdfWriter`。

use crate::Bitmap;

/// 输出 PDF 的一页：bitmap 数据 + 写入参数。
///
/// 字段对应 C 版 `pdffile_add_bitmap_with_ocrwords` 的入参集合
/// （`willuslib/pdfwrite.c:292-296`）。
#[derive(Debug, Clone)]
pub struct OutputPage {
    /// 输出页索引（0-based）。
    /// 对应 C `pdf->imc`（`willuslib/pdfwrite.c:330`，自增 page counter）。
    pub page_index: u32,

    /// 源 PDF 页号（0-based）。`-1` 表示来源不可追溯（cover / 合并页）。
    /// 用于 outline 映射时把 src_page → dst_page。
    pub srcpageno: i32,

    /// 像素数据（含 width / height / format / pixels / dpi）。
    pub bitmap: Bitmap,

    /// 输出 PDF 的物理 DPI。决定 PDF 页面尺寸：
    /// `width_pt = bitmap.width * 72 / output_dpi`，`height_pt = bitmap.height * 72 / output_dpi`。
    /// 对应 C `pdffile_add_bitmap` 的 `double dpi` 参数（`pdfwrite.c:266`）。
    pub output_dpi: f32,

    /// 旋转角度（度），0 / 90 / 180 / 270 之一。当前仅作为元信息，
    /// PDF writer 在 Step 7.2 暂不应用旋转矩阵（写入前应已由 layout 旋转好像素）。
    pub rotation: f32,

    /// JPEG 编码质量（0~100），或 `-1` = 用 Flate (deflate) 无损压缩。
    /// 对应 C `pdffile_add_bitmap` 的 `int quality` 参数（`pdfwrite.c:266`，注释见 274）：
    /// > "Use quality=-1 for PNG ... If quality < 0, the deflate (PNG-style) method is used."
    pub jpeg_quality: i32,

    /// 像素位深控制（C 版 halfsize 参数，`pdfwrite.c:266`，注释见 278-281）：
    /// - `0` = 8 bits per component（全保真）
    /// - `1` = 4 bits per component（半精度）
    /// - `2` = 2 bits per component
    /// - `3` = 1 bit per component
    ///
    /// Step 7.2 主路径仅实现 halfsize=0（全保真）；其余值由 Open Question 推迟。
    pub halfsize: u8,
}

impl OutputPage {
    /// 用最常见的配置构造一页：JPEG 质量 85、halfsize=0、rotation=0、srcpageno=-1。
    /// 等价于 C 版默认的 `pdffile_add_bitmap(pdf, bmp, dpi, 85, 0)`。
    #[must_use]
    pub fn from_bitmap(page_index: u32, bitmap: Bitmap, output_dpi: f32) -> Self {
        Self {
            page_index,
            srcpageno: -1,
            bitmap,
            output_dpi,
            rotation: 0.0,
            jpeg_quality: 85,
            halfsize: 0,
        }
    }

    /// 输出页物理宽度（PDF point，1 pt = 1/72 inch）。
    /// 公式：`width_pt = bitmap.width * 72 / output_dpi`。
    #[must_use]
    pub fn width_pt(&self) -> f32 {
        (self.bitmap.width as f32) * 72.0 / self.output_dpi
    }

    /// 输出页物理高度（PDF point）。
    #[must_use]
    pub fn height_pt(&self) -> f32 {
        (self.bitmap.height as f32) * 72.0 / self.output_dpi
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::PixelFormat;

    #[test]
    fn from_bitmap_defaults() {
        let bmp = Bitmap::new(100, 50, 200.0, PixelFormat::Gray8).unwrap();
        let page = OutputPage::from_bitmap(7, bmp, 200.0);
        assert_eq!(page.page_index, 7);
        assert_eq!(page.srcpageno, -1);
        assert_eq!(page.bitmap.width, 100);
        assert_eq!(page.bitmap.height, 50);
        assert!((page.output_dpi - 200.0).abs() < 1e-6);
        assert!((page.rotation - 0.0).abs() < 1e-6);
        assert_eq!(page.jpeg_quality, 85);
        assert_eq!(page.halfsize, 0);
    }

    #[test]
    fn width_pt_height_pt_math() {
        // 200x100 px @ 100 DPI → 144 x 72 pt
        let bmp = Bitmap::new(200, 100, 100.0, PixelFormat::Rgb8).unwrap();
        let page = OutputPage::from_bitmap(0, bmp, 100.0);
        assert!((page.width_pt() - 144.0).abs() < 1e-3);
        assert!((page.height_pt() - 72.0).abs() < 1e-3);
    }

    #[test]
    fn width_pt_height_pt_at_300dpi() {
        // 一张 A4 等价图：2480 x 3508 px @ 300 DPI → 595.2 x 841.92 pt
        let bmp = Bitmap::new(2480, 3508, 300.0, PixelFormat::Gray8).unwrap();
        let page = OutputPage::from_bitmap(0, bmp, 300.0);
        assert!((page.width_pt() - 595.2).abs() < 0.1);
        assert!((page.height_pt() - 841.92).abs() < 0.1);
    }

    #[test]
    fn custom_quality_and_halfsize() {
        let bmp = Bitmap::new(10, 10, 72.0, PixelFormat::Gray8).unwrap();
        let mut page = OutputPage::from_bitmap(0, bmp, 72.0);
        page.jpeg_quality = -1; // PNG (Flate) 模式
        page.halfsize = 1;
        assert_eq!(page.jpeg_quality, -1);
        assert_eq!(page.halfsize, 1);
    }

    #[test]
    fn srcpageno_can_be_set() {
        let bmp = Bitmap::new(10, 10, 72.0, PixelFormat::Gray8).unwrap();
        let mut page = OutputPage::from_bitmap(0, bmp, 72.0);
        page.srcpageno = 12;
        assert_eq!(page.srcpageno, 12);
    }

    #[test]
    fn clone_preserves_all_fields() {
        let bmp = Bitmap::new(4, 4, 72.0, PixelFormat::Rgba8).unwrap();
        let mut page = OutputPage::from_bitmap(3, bmp, 150.0);
        page.srcpageno = 5;
        page.rotation = 90.0;
        page.jpeg_quality = 70;
        page.halfsize = 2;

        let cloned = page.clone();
        assert_eq!(cloned.page_index, 3);
        assert_eq!(cloned.srcpageno, 5);
        assert!((cloned.rotation - 90.0).abs() < 1e-6);
        assert_eq!(cloned.jpeg_quality, 70);
        assert_eq!(cloned.halfsize, 2);
    }
}
