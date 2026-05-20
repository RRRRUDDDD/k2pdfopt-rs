//! `master_canvas` - 输出 master bitmap + row cursor 桶。
//!
//! 见 [`crate::master`] 模块文档与 `docs/masterinfo-design.md` §2 第 2 行。
//!
//! # C 字段对应
//!
//! 来源：`k2pdfoptlib/k2pdfopt.h:674-742`（MASTERINFO struct 的 canvas 字段）
//!
//! | C 字段 | Rust 字段 | C 行号 |
//! |--------|-----------|--------|
//! | `bmp` (WILLUSBITMAP) | [`MasterCanvas::bmp`] | 681 |
//! | `rows` | [`MasterCanvas::rows`] | 702 |
//! | `preview_bitmap *` | [`MasterCanvas::preview_bitmap`] | 686 |
//! | `preview_captured` | [`MasterCanvas::preview_captured`] | 688 |
//! | `cover_image` (v2.34) | [`MasterCanvas::cover_image`] | 696 |
//!
//! # Step 7.3 落地
//!
//! 本步骤落地 `ensure_height` / `fill_gap` / `blit` 三个核心算法（Gray8 / Rgb8 / Rgba8 全支持，
//! C 对照见 `k2master.c:867-915` + `bmp_more_rows` `willuslib/bmp.c`）。
//! `split_off_top` 是 Step 7.3 新增方法（C 等价 `masterinfo_remove_top_rows`,
//! `k2master.c:1196-1280`），把 master 顶部 rowcount 行切出来做下一页输出，并把剩余行往上挪。

use crate::master::RegionType;
use k2types::{Bitmap, PixelFormat};

/// 输出 master bitmap 画布。累积所有写入的行，达到一页阈值后 flush 出页。
///
/// 算法部分（resize / fill_gap / blit / split_off_top）在 Step 7.3（M5）落地。
#[derive(Debug)]
pub struct MasterCanvas {
    /// 输出 bitmap（来自 [`k2types::Bitmap`]，Step 5.1 落地的统一类型）。
    /// 对应 C `bmp` (WILLUSBITMAP)。`None` 表示尚未初始化。
    pub bmp: Option<Bitmap>,
    /// Master bitmap 当前已填充的行数（cursor）。对应 C `rows`。
    pub rows: u32,
    /// canvas 的宽度（pixel）。对应 C `bmp.width`。
    /// 单独保留以便在 `bmp == None` 时仍可查询。
    pub width: u32,
    /// canvas 的容量高度（pixel）。对应 C `bmp.height`。
    pub height: u32,
    /// 预览 bitmap 指针（用于 GUI 即时预览）。对应 C `preview_bitmap *`。
    pub preview_bitmap: Option<Bitmap>,
    /// 是否已捕获到预览 bitmap。对应 C `preview_captured`。
    pub preview_captured: bool,
    /// 封面 bitmap（v2.34 引入，用于 native PDF 输出的首页 cover）。对应 C `cover_image`。
    pub cover_image: Option<Bitmap>,
}

impl MasterCanvas {
    /// 构造默认空 MasterCanvas（无 bitmap，rows=0）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            bmp: None,
            rows: 0,
            width: 0,
            height: 0,
            preview_bitmap: None,
            preview_captured: false,
            cover_image: None,
        }
    }

    /// 用给定的 width + format 初始化 canvas（rows=0, 容量按需 grow）。
    ///
    /// 对应 C `bmp_alloc(&mi->bmp, width, height)`（在 `masterinfo_init` /
    /// `masterinfo_new_source_page_init` 中）。本实现把 height 初值设为 0；
    /// 实际容量由后续 [`MasterCanvas::ensure_height`] 按 1.4x 增长策略分配。
    pub fn init(&mut self, width: u32, format: PixelFormat) {
        self.width = width;
        self.height = 0;
        self.rows = 0;
        // 创建零容量 bitmap；ensure_height 会扩到首页所需高度
        self.bmp = Bitmap::new(width, 0, 1.0, format).ok();
    }

    /// 确保 canvas 内 bitmap 至少有 `needed` 行容量；不足时按 1.4x 增长。
    ///
    /// 对应 C `bmp_more_rows(&mi->bmp, 1.4, 255)`（`k2master.c:867-868`）。
    /// 新增行用背景色 255（白）填充。
    ///
    /// # 行为
    ///
    /// - 如果 `bmp` 为 `None`，按 `(width, needed)` 分配（用全白）
    /// - 如果 `bmp` 现有 height < needed，按 `max(needed, height*1.4)` 扩
    /// - 否则 no-op
    pub fn ensure_height(&mut self, needed: u32) {
        if needed == 0 {
            return;
        }
        if self.bmp.is_none() {
            debug_assert!(self.width > 0, "ensure_height 前必须先调 init()");
            // 用全白零容量 bitmap 起步；Bitmap::new 仅在 size 溢出时失败，
            // 这里 width * needed * 1 byte 不会超 usize（实际由调用方限制）
            let format = PixelFormat::Gray8;
            let bytes = (needed as usize)
                .saturating_mul(self.width as usize)
                .saturating_mul(format.bytes_per_pixel());
            let pixels = vec![255u8; bytes];
            // 用 from_raw 构造可绕过 expect_used（错误路径直接 fallback）
            if let Ok(b) = Bitmap::from_raw(self.width, needed, 1.0, format, pixels) {
                self.height = needed;
                self.bmp = Some(b);
            }
            return;
        }
        let cur_height = self.height;
        if needed <= cur_height {
            return;
        }
        // 按 1.4x 增长（C 行 868）；至少满足 needed
        let grown = ((cur_height as f64) * 1.4).ceil() as u32;
        let new_height = needed.max(grown).max(needed);
        if let Some(bmp) = self.bmp.as_mut() {
            let bytes_per_pixel = bmp.format.bytes_per_pixel();
            let bytes_per_row = (bmp.width as usize).saturating_mul(bytes_per_pixel);
            let new_total = (new_height as usize).saturating_mul(bytes_per_row);
            let old_len = bmp.pixels.len();
            if new_total > old_len {
                bmp.pixels.resize(new_total, 255);
            }
            bmp.height = new_height;
            self.height = new_height;
        }
    }

    /// 在 canvas 当前 rows 位置填充长度为 `gap` 行的空白（背景色 255）。
    ///
    /// 对应 C `if (gap_start>0) { ... memset(pdst,255, bmp_bytewidth*gap_start);
    /// rows += gap_start; }`（`k2master.c:892-899`）。
    ///
    /// # 行为
    ///
    /// - `gap=0`：no-op
    /// - 否则先 `ensure_height(rows + gap)` 扩容，再 memset 255 一段，rows += gap
    /// - `region_type` 仅作为元信息（C 版用于 fg/bg color 等高级特性，当前忽略）
    pub fn fill_gap(&mut self, gap: u32, _region_type: RegionType) {
        if gap == 0 {
            return;
        }
        let new_rows = self.rows.saturating_add(gap);
        self.ensure_height(new_rows);
        let bmp = match self.bmp.as_mut() {
            Some(b) => b,
            None => return, // ensure_height 后仍 None 说明 width=0，no-op
        };
        let bytes_per_row = bmp.bytes_per_row();
        let start = (self.rows as usize).saturating_mul(bytes_per_row);
        let end = (new_rows as usize).saturating_mul(bytes_per_row);
        if end <= bmp.pixels.len() {
            for b in &mut bmp.pixels[start..end] {
                *b = 255;
            }
        }
        self.rows = new_rows;
    }

    /// 把源 bitmap 数据复制到 canvas 当前 rows 位置，按 justification 决定水平对齐。
    ///
    /// 对应 C `for (i=0;i<tmp->height;i++) { memset(pdst,255,dw); memcpy(pdst+dw,
    /// psrc, srcbytewidth); memset(pdst+dw+srcbytewidth,255,dw2); rows++; }`
    /// （`k2master.c:904-915`）。
    ///
    /// # 参数
    ///
    /// - `src_pixels`：源 bitmap 像素（与 src_width × src_height × canvas.format 匹配）
    /// - `src_width` / `src_height`：源 bitmap 尺寸
    /// - `justification_flags`：水平对齐。`0` = 左对齐（C `just=0`），`1` = 居中
    ///   （C `just=1`），`2` = 右对齐（C `just=2`）。其他值按左对齐处理
    ///
    /// # 行为
    ///
    /// - 若 src_width > canvas.width，左对齐裁切（不重排，与 C 行 729 `dw<0` 后 `dw=0` 一致）
    /// - 自动调 `ensure_height(rows + src_height)`
    /// - 左右空白用 255 填白
    pub fn blit(&mut self, src: &[u8], src_width: u32, src_height: u32, justification_flags: i32) {
        if src_width == 0 || src_height == 0 {
            return;
        }
        let new_rows = self.rows.saturating_add(src_height);
        self.ensure_height(new_rows);
        let bmp = match self.bmp.as_mut() {
            Some(b) => b,
            None => return,
        };
        let bpp = bmp.format.bytes_per_pixel();
        let dst_w = bmp.width;
        let dst_bytes_per_row = (dst_w as usize).saturating_mul(bpp);
        let src_bytes_per_row = (src_width as usize).saturating_mul(bpp);
        // 水平偏移 dw（pixels，左对齐时为左边距）
        let dw = if src_width >= dst_w {
            0u32
        } else {
            match justification_flags & 0x3 {
                1 => (dst_w - src_width) / 2, // 居中
                2 => dst_w - src_width,       // 右对齐
                _ => 0,                       // 左对齐（含 0 / 3 / 未知）
            }
        };
        let dw_bytes = (dw as usize).saturating_mul(bpp);
        // 实际可用 src 字节数（src_width 超 dst_w 时裁切到 dst_w）
        let effective_src_w = src_width.min(dst_w);
        let effective_src_bytes = (effective_src_w as usize).saturating_mul(bpp);
        // 右侧空白字节数
        let dw2_bytes = dst_bytes_per_row.saturating_sub(dw_bytes + effective_src_bytes);

        for row in 0..src_height {
            let dst_row = self.rows.saturating_add(row);
            let dst_start = (dst_row as usize).saturating_mul(dst_bytes_per_row);
            let dst_end = dst_start.saturating_add(dst_bytes_per_row);
            if dst_end > bmp.pixels.len() {
                break; // ensure_height 已分配，这是防御性兜底
            }
            let src_start = (row as usize).saturating_mul(src_bytes_per_row);
            if src_start.saturating_add(effective_src_bytes) > src.len() {
                break;
            }
            // 左白
            for b in &mut bmp.pixels[dst_start..dst_start + dw_bytes] {
                *b = 255;
            }
            // src 拷贝
            let copy_to = dst_start + dw_bytes;
            bmp.pixels[copy_to..copy_to + effective_src_bytes]
                .copy_from_slice(&src[src_start..src_start + effective_src_bytes]);
            // 右白
            let pad_to = copy_to + effective_src_bytes;
            for b in &mut bmp.pixels[pad_to..pad_to + dw2_bytes] {
                *b = 255;
            }
        }
        self.rows = new_rows;
    }

    /// 把 canvas 顶部 `rowcount` 行切出来（用于 flush 出页），剩余行往上挪。
    ///
    /// 对应 C `masterinfo_remove_top_rows`（`k2master.c:1196-1280`）的简化版本。
    /// 返回切出的 (width, height, pixels) 三元组（pixel layout 与 canvas 相同）。
    ///
    /// # 行为
    ///
    /// - `rowcount >= rows`：返回整段 rows，canvas 清空（rows=0）
    /// - 否则切出 [0, rowcount) 行，剩余 [rowcount, rows) 行 memmove 到 [0, rows-rowcount)
    /// - canvas.rows -= rowcount
    /// - canvas.bmp 容量保持不变（不缩容）
    #[must_use]
    pub fn split_off_top(&mut self, rowcount: u32) -> Option<(u32, u32, Vec<u8>)> {
        if rowcount == 0 || self.rows == 0 {
            return None;
        }
        let take = rowcount.min(self.rows);
        let bmp = self.bmp.as_mut()?;
        let bytes_per_row = bmp.bytes_per_row();
        let take_bytes = (take as usize).saturating_mul(bytes_per_row);
        if take_bytes > bmp.pixels.len() {
            return None;
        }
        let mut split = vec![0u8; take_bytes];
        split.copy_from_slice(&bmp.pixels[..take_bytes]);
        // 剩余行往上挪（memmove 等价）
        let remaining = self.rows - take;
        if remaining > 0 {
            let remaining_bytes = (remaining as usize).saturating_mul(bytes_per_row);
            bmp.pixels
                .copy_within(take_bytes..take_bytes + remaining_bytes, 0);
            // 把腾空的尾部填白（避免下一次 blit 看到上一页残留）
            let tail_start = remaining_bytes;
            let tail_end = (self.rows as usize).saturating_mul(bytes_per_row);
            for b in &mut bmp.pixels[tail_start..tail_end] {
                *b = 255;
            }
        } else {
            // 全清空：把 [0, rows*bpr) 填白
            let zero_end = (self.rows as usize).saturating_mul(bytes_per_row);
            for b in &mut bmp.pixels[..zero_end] {
                *b = 255;
            }
        }
        self.rows = remaining;
        Some((bmp.width, take, split))
    }
}

impl Default for MasterCanvas {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::master::RegionType;

    #[test]
    fn new_defaults_to_empty_state() {
        let c = MasterCanvas::new();
        assert!(c.bmp.is_none());
        assert_eq!(c.rows, 0);
        assert_eq!(c.width, 0);
        assert_eq!(c.height, 0);
        assert!(c.preview_bitmap.is_none());
        assert!(!c.preview_captured);
        assert!(c.cover_image.is_none());
    }

    #[test]
    fn default_eq_new() {
        let a = MasterCanvas::default();
        let b = MasterCanvas::new();
        assert_eq!(a.rows, b.rows);
        assert_eq!(a.width, b.width);
        assert_eq!(a.height, b.height);
        assert_eq!(a.preview_captured, b.preview_captured);
    }

    #[test]
    fn fields_writable() {
        let mut c = MasterCanvas::new();
        c.rows = 100;
        c.width = 800;
        c.height = 2000;
        c.preview_captured = true;
        assert_eq!(c.rows, 100);
        assert_eq!(c.width, 800);
        assert_eq!(c.height, 2000);
        assert!(c.preview_captured);
    }

    #[test]
    fn init_allocates_zero_height_gray8() {
        let mut c = MasterCanvas::new();
        c.init(100, PixelFormat::Gray8);
        assert_eq!(c.width, 100);
        assert_eq!(c.height, 0);
        assert_eq!(c.rows, 0);
        assert!(c.bmp.is_some());
        assert_eq!(c.bmp.as_ref().unwrap().width, 100);
        assert_eq!(c.bmp.as_ref().unwrap().height, 0);
        assert_eq!(c.bmp.as_ref().unwrap().format, PixelFormat::Gray8);
    }

    #[test]
    fn ensure_height_grows_from_zero() {
        let mut c = MasterCanvas::new();
        c.init(50, PixelFormat::Gray8);
        c.ensure_height(100);
        assert_eq!(c.height, 100);
        assert_eq!(c.bmp.as_ref().unwrap().height, 100);
        assert_eq!(c.bmp.as_ref().unwrap().pixels.len(), 50 * 100);
        // 全白
        assert!(c.bmp.as_ref().unwrap().pixels.iter().all(|&b| b == 255));
    }

    #[test]
    fn ensure_height_growth_factor_140pct() {
        let mut c = MasterCanvas::new();
        c.init(50, PixelFormat::Gray8);
        c.ensure_height(100);
        // 再 ensure_height(101)：会按 100*1.4=140 增长
        c.ensure_height(101);
        assert!(c.height >= 101);
        // 1.4x 计算：(100*1.4).ceil()=140
        assert_eq!(c.height, 140);
    }

    #[test]
    fn ensure_height_idempotent_when_sufficient() {
        let mut c = MasterCanvas::new();
        c.init(50, PixelFormat::Gray8);
        c.ensure_height(100);
        let before = c.bmp.as_ref().unwrap().pixels.len();
        c.ensure_height(50);
        assert_eq!(c.height, 100); // 不缩
        assert_eq!(c.bmp.as_ref().unwrap().pixels.len(), before);
    }

    #[test]
    fn ensure_height_zero_is_noop() {
        let mut c = MasterCanvas::new();
        c.init(50, PixelFormat::Gray8);
        c.ensure_height(0);
        assert_eq!(c.height, 0);
    }

    #[test]
    fn fill_gap_appends_white_rows() {
        let mut c = MasterCanvas::new();
        c.init(10, PixelFormat::Gray8);
        c.fill_gap(5, RegionType::Text);
        assert_eq!(c.rows, 5);
        assert!(c.height >= 5);
        assert!(c.bmp.as_ref().unwrap().pixels[..50]
            .iter()
            .all(|&b| b == 255));
    }

    #[test]
    fn fill_gap_zero_is_noop() {
        let mut c = MasterCanvas::new();
        c.init(10, PixelFormat::Gray8);
        c.fill_gap(0, RegionType::Text);
        assert_eq!(c.rows, 0);
    }

    #[test]
    fn blit_left_aligned_copies_src() {
        let mut c = MasterCanvas::new();
        c.init(10, PixelFormat::Gray8);
        // 写一个 4x3 的"黑色"小块 (值 0)
        let src: Vec<u8> = vec![0; 4 * 3];
        c.blit(&src, 4, 3, 0);
        assert_eq!(c.rows, 3);
        // 第一行：[0,0,0,0, 255,255,255,255,255,255]
        let pixels = &c.bmp.as_ref().unwrap().pixels;
        assert_eq!(&pixels[0..4], &[0, 0, 0, 0]);
        assert!(&pixels[4..10].iter().all(|&b| b == 255));
        assert_eq!(&pixels[10..14], &[0, 0, 0, 0]); // 第二行
        assert!(&pixels[14..20].iter().all(|&b| b == 255));
    }

    #[test]
    fn blit_center_aligned() {
        let mut c = MasterCanvas::new();
        c.init(10, PixelFormat::Gray8);
        let src: Vec<u8> = vec![0; 4 * 2];
        c.blit(&src, 4, 2, 1); // 居中
        let pixels = &c.bmp.as_ref().unwrap().pixels;
        // dw = (10-4)/2 = 3
        assert!(pixels[0..3].iter().all(|&b| b == 255));
        assert_eq!(&pixels[3..7], &[0, 0, 0, 0]);
        assert!(pixels[7..10].iter().all(|&b| b == 255));
    }

    #[test]
    fn blit_right_aligned() {
        let mut c = MasterCanvas::new();
        c.init(10, PixelFormat::Gray8);
        let src: Vec<u8> = vec![0; 4 * 2];
        c.blit(&src, 4, 2, 2); // 右对齐
        let pixels = &c.bmp.as_ref().unwrap().pixels;
        // dw = 10-4 = 6
        assert!(pixels[0..6].iter().all(|&b| b == 255));
        assert_eq!(&pixels[6..10], &[0, 0, 0, 0]);
    }

    #[test]
    fn blit_oversized_src_truncates_left_align() {
        let mut c = MasterCanvas::new();
        c.init(5, PixelFormat::Gray8);
        let src: Vec<u8> = vec![1; 10 * 2]; // 比 canvas 宽
        c.blit(&src, 10, 2, 0);
        // 只拷贝前 5 像素
        let pixels = &c.bmp.as_ref().unwrap().pixels;
        assert_eq!(&pixels[0..5], &[1; 5]);
        assert_eq!(&pixels[5..10], &[1; 5]);
    }

    #[test]
    fn blit_rgb8_preserves_3_bytes_per_pixel() {
        let mut c = MasterCanvas::new();
        c.init(2, PixelFormat::Rgb8);
        let src: Vec<u8> = vec![10, 20, 30, 40, 50, 60]; // 2x1 RGB
        c.blit(&src, 2, 1, 0);
        assert_eq!(
            &c.bmp.as_ref().unwrap().pixels[..6],
            &[10, 20, 30, 40, 50, 60]
        );
    }

    #[test]
    fn split_off_top_returns_correct_data() {
        let mut c = MasterCanvas::new();
        c.init(4, PixelFormat::Gray8);
        // 写 6 行：前 3 行 = 100，后 3 行 = 200
        let src1: Vec<u8> = vec![100; 4 * 3];
        let src2: Vec<u8> = vec![200; 4 * 3];
        c.blit(&src1, 4, 3, 0);
        c.blit(&src2, 4, 3, 0);
        assert_eq!(c.rows, 6);

        let (w, h, data) = c.split_off_top(3).unwrap();
        assert_eq!(w, 4);
        assert_eq!(h, 3);
        assert_eq!(&data, &vec![100; 12]);
        assert_eq!(c.rows, 3); // 剩余 3 行
                               // 剩余应是原后 3 行（200）
        assert_eq!(&c.bmp.as_ref().unwrap().pixels[..12], &vec![200; 12]);
    }

    #[test]
    fn split_off_top_full_clears_canvas() {
        let mut c = MasterCanvas::new();
        c.init(4, PixelFormat::Gray8);
        let src: Vec<u8> = vec![100; 4 * 3];
        c.blit(&src, 4, 3, 0);
        let (_w, h, _data) = c.split_off_top(10).unwrap(); // 10 > 3
        assert_eq!(h, 3);
        assert_eq!(c.rows, 0);
    }

    #[test]
    fn split_off_top_zero_returns_none() {
        let mut c = MasterCanvas::new();
        c.init(4, PixelFormat::Gray8);
        let src: Vec<u8> = vec![100; 4];
        c.blit(&src, 4, 1, 0);
        assert!(c.split_off_top(0).is_none());
    }

    #[test]
    fn split_off_top_empty_canvas_returns_none() {
        let mut c = MasterCanvas::new();
        c.init(4, PixelFormat::Gray8);
        assert!(c.split_off_top(5).is_none());
    }
}
