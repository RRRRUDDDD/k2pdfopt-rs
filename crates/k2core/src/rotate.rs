//! 直角旋转与像素反转 helper（Step 8.4 / M6 收尾）。
//!
//! 1:1 复刻 C `willuslib/bmp.c::bmp_rotate_right_angle` (行 1569-1589) +
//! `bmp_rotate_90` (1592-1624) + `bmp_rotate_270` (1627-1659) +
//! `bmp_invert` (3223-3244)。
//!
//! # 与 [`crate::deskew::rotate_fast`] 的区别
//!
//! - [`crate::deskew::rotate_fast`]：任意角度双线性插值旋转（带 `expand` 选项），
//!   用于 deskew 自动纠偏。整角度时仍走 bilinear，引入 0.5 像素偏移与
//!   round-trip 误差。
//! - [`rotate_right_angle`]：仅支持 0/90/180/-90/270 直角旋转，**整数运算
//!   转置 + 翻转**，无插值误差，适合 figure 旋转主路径。
//!
//! # C 旋转方向约定（重要）
//!
//! C `bmp_rotate_right_angle(bmp, degrees)` 内部：
//! - `degrees=0`：no-op
//! - `degrees=90`：调 `bmp_rotate_90` → **逆时针 90°**（CCW）
//! - `degrees=180`：水平翻转 + 垂直翻转
//! - `degrees=-90` 或 `degrees=270`：调 `bmp_rotate_270` → **顺时针 90°**（CW）
//!
//! Rust 端保持 C 同源语义：`degrees=90` 触发 CCW 90°，`degrees=-90` 触发 CW 90°。
//! 调用方（如 `figure.rs::FigureRotation::to_deg`）的命名约定由其负责，本模块
//! 不做语义重映射。
//!
//! # PixelFormat 支持
//!
//! 全部 3 路径覆盖（Gray8 / Rgb8 / Rgba8）。`Rgba8` 旋转保留所有通道（含 alpha），
//! `invert` 仅反转 RGB 通道（alpha 原样保留，与 C 同源约定一致：C 无 RGBA 路径）。

use k2types::{Bitmap, PixelFormat};

// ---------------------------------------------------------------------------
// invert - 对应 C bmp_invert (willuslib/bmp.c:3223-3244)
// ---------------------------------------------------------------------------

/// 像素反转（in-place）：`v ← 255 - v`。
///
/// 1:1 复刻 C `bmp_invert` (`willuslib/bmp.c:3223-3244`)。
///
/// # PixelFormat 行为
///
/// - `Gray8`：单通道全反转。
/// - `Rgb8`：三通道（R/G/B）全反转。
/// - `Rgba8`：仅 RGB 三通道反转，**Alpha 通道原样保留**（C 版无 RGBA 路径，
///   Rust 端约定 alpha 保留是 figure negative 模式语义所需）。
///
/// # 来源
///
/// `k2pdfoptlib/k2proc.c:1448`（`if (is_figure && k2settings->dst_negative==1)
/// bmp_invert(bmp);`）。
pub fn invert(bmp: &mut Bitmap) {
    match bmp.format {
        PixelFormat::Gray8 | PixelFormat::Rgb8 => {
            for p in &mut bmp.pixels {
                *p = 255 - *p;
            }
        }
        PixelFormat::Rgba8 => {
            // 每像素 4 字节：[R, G, B, A]，仅 R/G/B 反转，A 保留
            for chunk in bmp.pixels.chunks_exact_mut(4) {
                chunk[0] = 255 - chunk[0];
                chunk[1] = 255 - chunk[1];
                chunk[2] = 255 - chunk[2];
                // chunk[3] 是 alpha，保留
            }
        }
    }
}

// ---------------------------------------------------------------------------
// rotate_right_angle - 对应 C bmp_rotate_right_angle (willuslib/bmp.c:1569)
// ---------------------------------------------------------------------------

/// 把 `degrees` 标准化为 `[0, 360)` 内最近的 90° 倍数索引（0/1/2/3）。
///
/// 1:1 复刻 C 行 1574-1577：
/// ```c
/// d = degrees % 360;
/// if (d < 0) d += 360;
/// d = (d + 45) / 90;
/// ```
fn normalize_quadrant(degrees: i32) -> i32 {
    let mut d = degrees % 360;
    if d < 0 {
        d += 360;
    }
    // (d + 45) / 90 用 C 整数截断：45 → 1, 134 → 1, 135 → 2 ...
    let q = (d + 45) / 90;
    // C 在 d=360 时 q=4，下面分支只看 1/2/3，d==0 与 d==4 等价 no-op
    q % 4
}

/// 直角旋转（in-place），仅支持 0 / +90 / 180 / -90 / 270 等 90° 倍数。
///
/// 1:1 复刻 C `bmp_rotate_right_angle` (`willuslib/bmp.c:1569-1589`)。
///
/// # 旋转语义（与 C 同源，**非命名直觉**）
///
/// - `degrees=0`：no-op
/// - `degrees=90`：**逆时针 90°**（CCW）—— 调 `bmp_rotate_90` 路径
/// - `degrees=180`：上下颠倒 + 左右翻转
/// - `degrees=-90` 或 `degrees=270`：**顺时针 90°**（CW）—— 调 `bmp_rotate_270` 路径
/// - 其他角度：先模 360，再四舍五入到最近的 90° 倍数（与 C 行 1577 一致）
///
/// # 维度变化
///
/// 90°/-90°/270° 后宽高互换（`dst.width = src.height`，`dst.height = src.width`）；
/// 180° 保持原维度。PixelFormat 不变。
///
/// # 来源
///
/// `k2pdfoptlib/k2proc.c:1496` 在 figure rotate 决策为非 0 时调用。
pub fn rotate_right_angle(bmp: &mut Bitmap, degrees: i32) {
    if bmp.width == 0 || bmp.height == 0 {
        return;
    }
    let q = normalize_quadrant(degrees);
    match q {
        0 => {} // no-op
        1 => rotate_90_ccw(bmp),
        2 => rotate_180(bmp),
        3 => rotate_90_cw(bmp),
        _ => unreachable!("normalize_quadrant returned {q} (mod 4)"),
    }
}

// ---------------------------------------------------------------------------
// 内部 helper：rotate_90_ccw / rotate_90_cw / rotate_180
// ---------------------------------------------------------------------------

/// 逆时针 90° 旋转（in-place）。C `bmp_rotate_90` (`willuslib/bmp.c:1592-1624`)。
///
/// 公式：`dst(i, j) = src(j, src.width - 1 - i)`，其中 `i ∈ [0, dst.height)`，
/// `j ∈ [0, dst.width)`，`dst.width = src.height`，`dst.height = src.width`。
fn rotate_90_ccw(bmp: &mut Bitmap) {
    let bpp = bmp.format.bytes_per_pixel();
    let sw = bmp.width as usize;
    let sh = bmp.height as usize;
    let src_bpr = sw * bpp;
    // dst 维度：(sh, sw) 互换 → (dst_w, dst_h) = (sh, sw)
    let dst_w = sh;
    let dst_h = sw;
    let dst_bpr = dst_w * bpp;
    let mut dst = vec![0u8; dst_bpr * dst_h];
    // src(sr, sc) → dst(dst_h-1-sc, sr) = dst(sw-1-sc, sr)
    for sr in 0..sh {
        let src_row_off = sr * src_bpr;
        for sc in 0..sw {
            let src_off = src_row_off + sc * bpp;
            let dst_row = sw - 1 - sc;
            let dst_col = sr;
            let dst_off = dst_row * dst_bpr + dst_col * bpp;
            dst[dst_off..dst_off + bpp].copy_from_slice(&bmp.pixels[src_off..src_off + bpp]);
        }
    }
    bmp.pixels = dst;
    bmp.width = dst_w as u32;
    bmp.height = dst_h as u32;
}

/// 顺时针 90° 旋转（in-place）。C `bmp_rotate_270` (`willuslib/bmp.c:1627-1659`)。
///
/// 公式：`dst(i, j) = src(src.height - 1 - j, i)`，其中 `i ∈ [0, dst.height)`，
/// `j ∈ [0, dst.width)`，`dst.width = src.height`，`dst.height = src.width`。
fn rotate_90_cw(bmp: &mut Bitmap) {
    let bpp = bmp.format.bytes_per_pixel();
    let sw = bmp.width as usize;
    let sh = bmp.height as usize;
    let src_bpr = sw * bpp;
    let dst_w = sh;
    let dst_h = sw;
    let dst_bpr = dst_w * bpp;
    let mut dst = vec![0u8; dst_bpr * dst_h];
    // src(sr, sc) → dst(sc, sh-1-sr)
    for sr in 0..sh {
        let src_row_off = sr * src_bpr;
        for sc in 0..sw {
            let src_off = src_row_off + sc * bpp;
            let dst_row = sc;
            let dst_col = sh - 1 - sr;
            let dst_off = dst_row * dst_bpr + dst_col * bpp;
            dst[dst_off..dst_off + bpp].copy_from_slice(&bmp.pixels[src_off..src_off + bpp]);
        }
    }
    bmp.pixels = dst;
    bmp.width = dst_w as u32;
    bmp.height = dst_h as u32;
}

/// 180° 旋转 = 上下颠倒 + 左右翻转。等价 C 行 1581-1585。
///
/// 维度不变。
fn rotate_180(bmp: &mut Bitmap) {
    let bpp = bmp.format.bytes_per_pixel();
    let w = bmp.width as usize;
    let h = bmp.height as usize;
    let bpr = w * bpp;
    let mut dst = vec![0u8; bpr * h];
    // dst(i, j) = src(h-1-i, w-1-j)
    for sr in 0..h {
        let src_row_off = sr * bpr;
        let dst_row = h - 1 - sr;
        let dst_row_off = dst_row * bpr;
        for sc in 0..w {
            let src_off = src_row_off + sc * bpp;
            let dst_col = w - 1 - sc;
            let dst_off = dst_row_off + dst_col * bpp;
            dst[dst_off..dst_off + bpp].copy_from_slice(&bmp.pixels[src_off..src_off + bpp]);
        }
    }
    bmp.pixels = dst;
    // width / height 不变
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use k2types::{Bitmap, PixelFormat};

    fn mk(width: u32, height: u32, fmt: PixelFormat, pixels: Vec<u8>) -> Bitmap {
        Bitmap::from_raw(width, height, 1.0, fmt, pixels).unwrap()
    }

    // ---------------- invert ----------------

    #[test]
    fn invert_gray8_flips_all_pixels() {
        let mut b = mk(2, 1, PixelFormat::Gray8, vec![0, 200]);
        invert(&mut b);
        assert_eq!(b.pixels, vec![255, 55]);
    }

    #[test]
    fn invert_rgb8_flips_three_channels() {
        let mut b = mk(1, 1, PixelFormat::Rgb8, vec![10, 20, 30]);
        invert(&mut b);
        assert_eq!(b.pixels, vec![245, 235, 225]);
    }

    #[test]
    fn invert_rgba8_preserves_alpha() {
        let mut b = mk(
            2,
            1,
            PixelFormat::Rgba8,
            vec![10, 20, 30, 100, 40, 50, 60, 200],
        );
        invert(&mut b);
        // RGB 反转，A 不变
        assert_eq!(b.pixels, vec![245, 235, 225, 100, 215, 205, 195, 200]);
    }

    #[test]
    fn invert_empty_no_op() {
        let mut b = Bitmap::new(0, 0, 1.0, PixelFormat::Gray8).unwrap();
        invert(&mut b);
        assert!(b.pixels.is_empty());
    }

    // ---------------- normalize_quadrant ----------------

    #[test]
    fn normalize_quadrant_table() {
        // d=0 → q=0
        assert_eq!(normalize_quadrant(0), 0);
        // d=44 → q=0; d=45 → q=1
        assert_eq!(normalize_quadrant(44), 0);
        assert_eq!(normalize_quadrant(45), 1);
        // d=90 → q=1
        assert_eq!(normalize_quadrant(90), 1);
        // d=134 → q=1; d=135 → q=2
        assert_eq!(normalize_quadrant(134), 1);
        assert_eq!(normalize_quadrant(135), 2);
        // d=180 → q=2
        assert_eq!(normalize_quadrant(180), 2);
        // d=270 → q=3
        assert_eq!(normalize_quadrant(270), 3);
        // d=-90 → +90 后 d=270 → q=3
        assert_eq!(normalize_quadrant(-90), 3);
        // d=360 → mod 360 = 0 → q=0
        assert_eq!(normalize_quadrant(360), 0);
        // d=-270 → +90 后 d=90 → q=1
        assert_eq!(normalize_quadrant(-270), 1);
    }

    // ---------------- rotate_right_angle 0° ----------------

    #[test]
    fn rotate_0_is_noop() {
        let mut b = mk(2, 3, PixelFormat::Gray8, vec![1, 2, 3, 4, 5, 6]);
        rotate_right_angle(&mut b, 0);
        assert_eq!(b.width, 2);
        assert_eq!(b.height, 3);
        assert_eq!(b.pixels, vec![1, 2, 3, 4, 5, 6]);
    }

    // ---------------- rotate_right_angle 90° (CCW) ----------------

    #[test]
    fn rotate_90_ccw_gray8_2x3() {
        // src:
        //   A B
        //   C D
        //   E F
        // CCW 90°（dst.width = 3, dst.height = 2）:
        //   B D F
        //   A C E
        let mut b = mk(
            2,
            3,
            PixelFormat::Gray8,
            vec![b'A', b'B', b'C', b'D', b'E', b'F'],
        );
        rotate_right_angle(&mut b, 90);
        assert_eq!(b.width, 3);
        assert_eq!(b.height, 2);
        assert_eq!(
            b.pixels,
            vec![b'B', b'D', b'F', b'A', b'C', b'E'],
            "CCW 90° 应把右上角 'B' 转到左上角"
        );
    }

    #[test]
    fn rotate_90_ccw_rgb8_2x2() {
        // src（每像素 3 字节）:
        //   (1,2,3) (4,5,6)
        //   (7,8,9) (10,11,12)
        // CCW 90° (2x2 → 2x2):
        //   (4,5,6) (10,11,12)
        //   (1,2,3) (7,8,9)
        let mut b = mk(
            2,
            2,
            PixelFormat::Rgb8,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        );
        rotate_right_angle(&mut b, 90);
        assert_eq!(b.width, 2);
        assert_eq!(b.height, 2);
        assert_eq!(b.pixels, vec![4, 5, 6, 10, 11, 12, 1, 2, 3, 7, 8, 9]);
    }

    #[test]
    fn rotate_90_ccw_rgba8_1x2() {
        // src:
        //   (10,20,30,40)
        //   (50,60,70,80)
        // CCW 90° (1x2 → 2x1):
        //   (10,20,30,40) (50,60,70,80)
        let mut b = mk(
            1,
            2,
            PixelFormat::Rgba8,
            vec![10, 20, 30, 40, 50, 60, 70, 80],
        );
        rotate_right_angle(&mut b, 90);
        assert_eq!(b.width, 2);
        assert_eq!(b.height, 1);
        assert_eq!(b.pixels, vec![10, 20, 30, 40, 50, 60, 70, 80]);
    }

    // ---------------- rotate_right_angle 180° ----------------

    #[test]
    fn rotate_180_gray8_2x3() {
        // src:
        //   A B
        //   C D
        //   E F
        // 180° (2x3 不变):
        //   F E
        //   D C
        //   B A
        let mut b = mk(
            2,
            3,
            PixelFormat::Gray8,
            vec![b'A', b'B', b'C', b'D', b'E', b'F'],
        );
        rotate_right_angle(&mut b, 180);
        assert_eq!(b.width, 2);
        assert_eq!(b.height, 3);
        assert_eq!(b.pixels, vec![b'F', b'E', b'D', b'C', b'B', b'A']);
    }

    // ---------------- rotate_right_angle -90° (CW) ----------------

    #[test]
    fn rotate_minus_90_cw_gray8_2x3() {
        // src:
        //   A B
        //   C D
        //   E F
        // CW 90° (dst.width = 3, dst.height = 2):
        //   E C A
        //   F D B
        let mut b = mk(
            2,
            3,
            PixelFormat::Gray8,
            vec![b'A', b'B', b'C', b'D', b'E', b'F'],
        );
        rotate_right_angle(&mut b, -90);
        assert_eq!(b.width, 3);
        assert_eq!(b.height, 2);
        assert_eq!(
            b.pixels,
            vec![b'E', b'C', b'A', b'F', b'D', b'B'],
            "CW 90° 应把左上角 'A' 转到右上角"
        );
    }

    #[test]
    fn rotate_270_equals_minus_90() {
        // CW 90° via degrees=270 应与 degrees=-90 等价
        let mut a = mk(
            2,
            3,
            PixelFormat::Gray8,
            vec![b'A', b'B', b'C', b'D', b'E', b'F'],
        );
        let mut b = mk(
            2,
            3,
            PixelFormat::Gray8,
            vec![b'A', b'B', b'C', b'D', b'E', b'F'],
        );
        rotate_right_angle(&mut a, 270);
        rotate_right_angle(&mut b, -90);
        assert_eq!(a.pixels, b.pixels);
        assert_eq!(a.width, b.width);
        assert_eq!(a.height, b.height);
    }

    // ---------------- 旋转 round-trip ----------------

    #[test]
    fn rotate_90_ccw_then_cw_round_trip() {
        let orig = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9];
        let mut b = mk(3, 3, PixelFormat::Gray8, orig.clone());
        rotate_right_angle(&mut b, 90);
        rotate_right_angle(&mut b, -90);
        assert_eq!(b.width, 3);
        assert_eq!(b.height, 3);
        assert_eq!(b.pixels, orig);
    }

    #[test]
    fn rotate_180_twice_round_trip() {
        let orig = vec![1u8, 2, 3, 4, 5, 6];
        let mut b = mk(2, 3, PixelFormat::Gray8, orig.clone());
        rotate_right_angle(&mut b, 180);
        rotate_right_angle(&mut b, 180);
        assert_eq!(b.pixels, orig);
    }

    #[test]
    fn rotate_90_four_times_round_trip() {
        let orig = vec![1u8, 2, 3, 4];
        let mut b = mk(2, 2, PixelFormat::Gray8, orig.clone());
        for _ in 0..4 {
            rotate_right_angle(&mut b, 90);
        }
        assert_eq!(b.width, 2);
        assert_eq!(b.height, 2);
        assert_eq!(b.pixels, orig);
    }

    // ---------------- 非 90° 倍数四舍五入 ----------------

    #[test]
    fn rotate_46_rounds_to_90() {
        let mut a = mk(2, 3, PixelFormat::Gray8, vec![1, 2, 3, 4, 5, 6]);
        let mut b = mk(2, 3, PixelFormat::Gray8, vec![1, 2, 3, 4, 5, 6]);
        rotate_right_angle(&mut a, 46);
        rotate_right_angle(&mut b, 90);
        assert_eq!(a.pixels, b.pixels);
        assert_eq!(a.width, b.width);
        assert_eq!(a.height, b.height);
    }

    #[test]
    fn rotate_44_rounds_to_zero() {
        let mut b = mk(2, 3, PixelFormat::Gray8, vec![1, 2, 3, 4, 5, 6]);
        rotate_right_angle(&mut b, 44);
        assert_eq!(b.pixels, vec![1, 2, 3, 4, 5, 6]);
    }

    // ---------------- 边界：空 bitmap ----------------

    #[test]
    fn rotate_empty_no_op() {
        let mut b = Bitmap::new(0, 0, 1.0, PixelFormat::Gray8).unwrap();
        rotate_right_angle(&mut b, 90);
        assert_eq!(b.width, 0);
        assert_eq!(b.height, 0);
        assert!(b.pixels.is_empty());
    }
}
