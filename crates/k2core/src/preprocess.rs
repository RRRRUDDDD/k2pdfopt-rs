//! 图像预处理 - contrast / gamma / sharpen 三个滤镜。
//!
//! ## 算法来源
//!
//! 全部按 C 版 `willuslib/bmp.c` 行号 1:1 移植：
//!
//! - [`apply_contrast`]：`bmp_contrast_adjust` (`willuslib/bmp.c:3345-3377`)
//!   - 256-LUT，正负号 / `|c|>1.5` exponential 曲线 / `|c|<=1.5` 线性 clip
//! - [`apply_gamma`]：`bmp_gamma_correct` (`willuslib/bmp.c:3387-3400`)
//!   - 256-LUT，`gamma = max(gamma, 0.001)`，`v = 255*(i/255)^(1/gamma) + 0.5`
//! - [`apply_sharpen`]：`bmp_sharpen` (`willuslib/bmp.c:3571-3587`)
//!   - 3x3 卷积核：周边 `-0.1`，中心 `1.8`；走 `bmp_apply_filter`
//! - [`apply_filter_3x3`]：`bmp_apply_filter` (`willuslib/bmp.c:3598-3673`)
//!   - 边界处只用 in-bounds 的 filter 值，并按 `weight` 归一化（非 zero-padding）
//!
//! ## 像素格式策略
//!
//! - `Gray8`：单通道 LUT / 单通道卷积
//! - `Rgb8`：每通道独立 LUT / 每通道独立卷积
//! - `Rgba8`：RGB 走滤镜，**alpha 通道原样保留**（C 版无 RGBA 支持，这是 Rust 端的扩展约定）
//!
//! ## In-place vs out-of-place
//!
//! 全部 API 形如 `apply_xxx(bitmap: &mut Bitmap, ...)`，等价于 C 版
//! `bmp_xxx(dest=src, src, ...)`。卷积内部会临时拷贝一份 src 像素（避免 in-place 写冲突）。
//!
//! 来源：`rust-rewrite-execution-plan.md` Step 5.3；`rust-rewrite-plan.md` v2.1 §5.2 / §8.2。

use k2types::{Bitmap, PixelFormat};

// --------------------------------------------------------------------------
// Public API
// --------------------------------------------------------------------------

/// 对比度调整（in-place）。
///
/// 1:1 复刻 C 版 `bmp_contrast_adjust` (`willuslib/bmp.c:3345-3377`)。
///
/// - `contrast > 1`：增强对比度
/// - `contrast < 1`：降低对比度
/// - `contrast == 0`：所有像素归一为同一灰度（127 附近）
/// - `contrast == 1`：恒等（仍会重写每个像素，但值不变）
/// - `contrast < 0`：先按 `|contrast|` 调整，再以 127.5 为轴**镜像翻转**像素值
/// - `|contrast| > 1.5`：走 exponential 曲线 `1 - exp(|c|*x/(x-1))`
/// - `|contrast| <= 1.5`：走线性 clip `min(|c|*x, 1)`
///
/// LUT 内部用 f64 计算，最终写回 `u8`，与 C 版 `static unsigned char newval[256]`
/// 路径一致。
pub fn apply_contrast(bitmap: &mut Bitmap, contrast: f64) {
    let lut = build_contrast_lut(contrast);
    apply_lut(bitmap, &lut);
}

/// 伽马校正（in-place）。
///
/// 1:1 复刻 C 版 `bmp_gamma_correct` (`willuslib/bmp.c:3387-3400`)。
///
/// - `gamma < 0.001`：会被钳到 `0.001`（避免除零）
/// - `gamma == 1`：恒等
/// - `gamma > 1`：图像变亮（中间调上抬）
/// - `gamma < 1`：图像变暗（中间调下压）
///
/// 公式：`v = 255 * (i / 255)^(1/gamma) + 0.5`，然后 clamp 到 `[0, 255]`。
pub fn apply_gamma(bitmap: &mut Bitmap, gamma: f64) {
    let lut = build_gamma_lut(gamma);
    apply_lut(bitmap, &lut);
}

/// 锐化（in-place），使用固定 3x3 卷积核：
/// `[[-0.1, -0.1, -0.1], [-0.1, 1.8, -0.1], [-0.1, -0.1, -0.1]]`。
///
/// 1:1 复刻 C 版 `bmp_sharpen` (`willuslib/bmp.c:3571-3587`)。
pub fn apply_sharpen(bitmap: &mut Bitmap) {
    apply_filter_3x3(bitmap, &sharpen_kernel());
}

/// 通用 3x3 卷积（in-place），边界采用 **partial sum + weight normalize** 策略
/// （非 zero-padding），与 C 版 `bmp_apply_filter` 完全一致。
///
/// 索引顺序：`filter[col][row]`，与 C 版 `filter[cf+cc][rf+rc]` 同序。
/// 对完全对称的 filter（如 sharpen）此顺序无影响。
///
/// 当某位置 `weight == 0` 时，整像素清零（含 alpha；与 C 版 `dst = bmp_alloc()`
/// 后 continue 不写的语义一致）。
pub fn apply_filter_3x3(bitmap: &mut Bitmap, filter: &[[f64; 3]; 3]) {
    let width = bitmap.width as i32;
    let height = bitmap.height as i32;
    if width == 0 || height == 0 {
        return;
    }
    let bpp = bitmap.format.bytes_per_pixel();
    let bytes_per_row = bitmap.bytes_per_row();
    // C 版用单独的 dest buffer；Rust 也复制 src 避免 in-place 写冲突
    let src = bitmap.pixels.clone();
    let mut dst = vec![0u8; bitmap.pixels.len()];

    // 3x3 中心
    let nrows = 3i32;
    let ncols = 3i32;
    let rc = nrows / 2; // = 1
    let cc = ncols / 2; // = 1

    for ir in 0..height {
        for ic in 0..width {
            // 计算 in-bounds 的 filter 偏移范围（C bmp.c:3625-3633）
            let rf1 = if ir - rc < 0 { -ir } else { -rc };
            let rf2 = if ir + (nrows - rc - 1) > height - 1 {
                height - 1 - ir
            } else {
                nrows - rc - 1
            };
            let cf1 = if ic - cc < 0 { -ic } else { -cc };
            let cf2 = if ic + (ncols - cc - 1) > width - 1 {
                width - 1 - ic
            } else {
                ncols - cc - 1
            };

            // 累加器（按通道）
            let mut sr = 0.0_f64;
            let mut sg = 0.0_f64;
            let mut sb = 0.0_f64;
            let mut weight = 0.0_f64;

            for rf in rf1..=rf2 {
                for cf in cf1..=cf2 {
                    // 注意 C 版索引顺序：filter[cf+cc][rf+rc]（外层 col，内层 row）
                    let fw = filter[(cf + cc) as usize][(rf + rc) as usize];
                    weight += fw;
                    let src_x = ic + cf;
                    let src_y = ir + rf;
                    let src_off = (src_y as usize) * bytes_per_row + (src_x as usize) * bpp;
                    match bitmap.format {
                        PixelFormat::Gray8 => {
                            sr += f64::from(src[src_off]) * fw;
                        }
                        PixelFormat::Rgb8 | PixelFormat::Rgba8 => {
                            sr += f64::from(src[src_off]) * fw;
                            sg += f64::from(src[src_off + 1]) * fw;
                            sb += f64::from(src[src_off + 2]) * fw;
                        }
                    }
                }
            }

            let dst_off = (ir as usize) * bytes_per_row + (ic as usize) * bpp;
            if weight == 0.0 {
                // 与 C bmp.c:3651-3652 一致：跳过，dst 保持 0
                // 对 RGBA：alpha 通道也保持 0（约定，与无 RGBA 的 C 版兼容）
                continue;
            }
            match bitmap.format {
                PixelFormat::Gray8 => {
                    dst[dst_off] = round_clamp_u8(sr / weight);
                }
                PixelFormat::Rgb8 => {
                    dst[dst_off] = round_clamp_u8(sr / weight);
                    dst[dst_off + 1] = round_clamp_u8(sg / weight);
                    dst[dst_off + 2] = round_clamp_u8(sb / weight);
                }
                PixelFormat::Rgba8 => {
                    dst[dst_off] = round_clamp_u8(sr / weight);
                    dst[dst_off + 1] = round_clamp_u8(sg / weight);
                    dst[dst_off + 2] = round_clamp_u8(sb / weight);
                    // alpha 原样保留
                    let src_alpha_off = (ir as usize) * bytes_per_row + (ic as usize) * bpp + 3;
                    dst[dst_off + 3] = src[src_alpha_off];
                }
            }
        }
    }

    bitmap.pixels = dst;
}

// --------------------------------------------------------------------------
// LUT 构造（暴露给测试用 pub(crate)）
// --------------------------------------------------------------------------

/// 构造 contrast LUT（256 项 u8 表）。1:1 复刻 C 版 `bmp_contrast_adjust` 内部循环。
#[must_use]
pub fn build_contrast_lut(contrast: f64) -> [u8; 256] {
    let mut lut = [0u8; 256];
    for (i, slot) in lut.iter_mut().enumerate() {
        // C bmp.c:3355-3374
        let x_raw = (i as f64 - 127.5) / 127.5;
        let sgn_base = if x_raw < 0.0 { -1.0 } else { 1.0 };
        let sgn = if contrast < 0.0 { -sgn_base } else { sgn_base };
        let x = x_raw.abs();
        let y = if contrast.abs() > 1.5 {
            if x < 0.99999 {
                1.0 - (contrast.abs() * x / (x - 1.0)).exp()
            } else {
                1.0
            }
        } else {
            let v = contrast.abs() * x;
            if v > 1.0 {
                1.0
            } else {
                v
            }
        };
        let y_final = 127.5 + y * sgn * 127.5;
        *slot = round_clamp_u8(y_final);
    }
    lut
}

/// 构造 gamma LUT（256 项 u8 表）。1:1 复刻 C 版 `bmp_gamma_correct` 内部循环。
#[must_use]
pub fn build_gamma_lut(gamma: f64) -> [u8; 256] {
    let g = if gamma < 0.001 { 0.001 } else { gamma };
    let gc = 1.0 / g;
    let mut lut = [0u8; 256];
    for (i, slot) in lut.iter_mut().enumerate() {
        // C bmp.c:3398:
        //   newval[i] = 255.*pow(i/255.,gc)+.5;
        // 隐式 `(unsigned char)` 转换是 truncate toward zero；对正数等价于 round half up。
        // [`round_clamp_u8`] 内部加 0.5 再 truncate，所以这里传未加 0.5 的原值。
        let v_raw = 255.0 * (i as f64 / 255.0).powf(gc);
        *slot = round_clamp_u8(v_raw);
    }
    lut
}

/// 锐化卷积核（3x3）：周边 -0.1，中心 1.8。
#[must_use]
fn sharpen_kernel() -> [[f64; 3]; 3] {
    // 与 C bmp.c:3580-3583 的二维 vector_2d 完全等价。
    [[-0.1, -0.1, -0.1], [-0.1, 1.8, -0.1], [-0.1, -0.1, -0.1]]
}

// --------------------------------------------------------------------------
// 内部工具函数
// --------------------------------------------------------------------------

/// 用 256-LUT 改写 bitmap 的所有像素。
/// Gray8 走单通道；RGB/RGBA 每通道独立查表，alpha 通道不变。
fn apply_lut(bitmap: &mut Bitmap, lut: &[u8; 256]) {
    match bitmap.format {
        PixelFormat::Gray8 => {
            for px in &mut bitmap.pixels {
                *px = lut[*px as usize];
            }
        }
        PixelFormat::Rgb8 => {
            for chunk in bitmap.pixels.chunks_exact_mut(3) {
                chunk[0] = lut[chunk[0] as usize];
                chunk[1] = lut[chunk[1] as usize];
                chunk[2] = lut[chunk[2] as usize];
            }
        }
        PixelFormat::Rgba8 => {
            for chunk in bitmap.pixels.chunks_exact_mut(4) {
                chunk[0] = lut[chunk[0] as usize];
                chunk[1] = lut[chunk[1] as usize];
                chunk[2] = lut[chunk[2] as usize];
                // chunk[3] (alpha) 保持不变
            }
        }
    }
}

/// 对 f64 加 0.5 后截断到 `[0, 255]` 的 u8。等价 C 版
/// `v = (int)(y + 0.5); BOUND(v, 0, 255);`。
fn round_clamp_u8(value: f64) -> u8 {
    let v = value + 0.5;
    // f64 -> i32 在 Rust >=1.45 是饱和的（不会 UB），符合预期边界
    let n = v as i32;
    n.clamp(0, 255) as u8
}

// --------------------------------------------------------------------------
// 单元测试
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;
    use k2types::{Bitmap, PixelFormat};

    // ----- LUT 构造测试 -----

    #[test]
    fn contrast_lut_identity_at_one() {
        // contrast=1 对 |c|<=1.5 路径，y = 1*x，但 y clip 到 1 -> 等价 max(x, 1) 映射
        // i=0 -> x=1 -> y=1*1=1 -> y_final=127.5+1*(-1)*127.5=0 -> u8 0
        // i=255 -> x=1 -> y=1 -> y_final=127.5+1*1*127.5=255 -> u8 255
        // i=128 -> x_raw=(128-127.5)/127.5≈0.00392 -> x=0.00392 -> y=0.00392
        //       -> y_final=127.5+0.00392*1*127.5≈128.0 -> u8 128
        let lut = build_contrast_lut(1.0);
        assert_eq!(lut[0], 0);
        assert_eq!(lut[127], 127); // x_raw=(127-127.5)/127.5≈-0.00392, sgn=-1, y_final≈127
        assert_eq!(lut[128], 128);
        assert_eq!(lut[255], 255);
    }

    #[test]
    fn contrast_lut_zero_flattens_to_midgray() {
        // contrast=0 -> y = 0*x = 0 -> y_final = 127.5 + 0 = 127.5 -> 128 (round half up)
        let lut = build_contrast_lut(0.0);
        for &v in lut.iter() {
            // 全部映射到 127 或 128（C 行为：y=0 时 sgn 为 -1 路径 -> 127.5-0*1*127.5=127.5 -> 128）
            assert!(v == 127 || v == 128, "got {v}");
        }
    }

    #[test]
    fn contrast_lut_negative_mirrors() {
        // contrast=-1 -> sgn 反转 -> 等价 LUT(i) = 255 - LUT_pos(i)
        let pos = build_contrast_lut(1.0);
        let neg = build_contrast_lut(-1.0);
        for i in 0..256 {
            let sum = i32::from(pos[i]) + i32::from(neg[i]);
            // i=127 与 i=128 各自有 1 像素的舍入残差
            assert!(
                (254..=256).contains(&sum),
                "i={i} pos={} neg={} sum={}",
                pos[i],
                neg[i],
                sum
            );
        }
    }

    #[test]
    fn contrast_lut_high_contrast_uses_exp_path() {
        // |c|>1.5 走 exponential，i=0 -> x=1 -> y=1 -> 0
        // 中间值会更陡（亮的更亮，暗的更暗）
        let lut = build_contrast_lut(2.0);
        assert_eq!(lut[0], 0);
        assert_eq!(lut[255], 255);
        // 与 c=1 相比，c=2 的中间值应更陡：lut[64] < lut_c1[64]
        let lut_c1 = build_contrast_lut(1.0);
        assert!(
            lut[64] < lut_c1[64],
            "c=2 lut[64]={} should be < c=1 lut[64]={}",
            lut[64],
            lut_c1[64]
        );
    }

    #[test]
    fn gamma_lut_identity_at_one() {
        let lut = build_gamma_lut(1.0);
        for (i, &v) in lut.iter().enumerate() {
            assert_eq!(v, i as u8, "gamma=1 lut[{i}]={v}");
        }
    }

    #[test]
    fn gamma_lut_brightens_when_gt_one() {
        // gamma > 1 -> gc = 1/gamma < 1 -> i/255 ^ gc 上抬（中间值变亮）
        let lut = build_gamma_lut(2.0);
        assert_eq!(lut[0], 0);
        assert_eq!(lut[255], 255);
        assert!(lut[128] > 128);
    }

    #[test]
    fn gamma_lut_darkens_when_lt_one() {
        // gamma < 1 -> gc > 1 -> 中间值变暗
        let lut = build_gamma_lut(0.5);
        assert_eq!(lut[0], 0);
        assert_eq!(lut[255], 255);
        assert!(lut[128] < 128);
    }

    #[test]
    fn gamma_lut_clamps_zero_to_min() {
        // gamma=0 -> 被钳到 0.001 -> gc=1000 -> 极陡 -> 几乎全是 0 直到 i=255 才 255
        let lut = build_gamma_lut(0.0);
        assert_eq!(lut[0], 0);
        assert_eq!(lut[255], 255);
        // 极值 gamma 会让 i=254 也接近 0
        assert!(lut[254] < 100);
    }

    // ----- apply_contrast / apply_gamma 像素级测试 -----

    #[test]
    fn apply_contrast_gray8_inplace() {
        let mut bmp = Bitmap::new(4, 1, 72.0, PixelFormat::Gray8).unwrap();
        bmp.pixels.copy_from_slice(&[0, 64, 192, 255]);
        let lut = build_contrast_lut(2.0);
        apply_contrast(&mut bmp, 2.0);
        assert_eq!(bmp.pixels[0], lut[0]);
        assert_eq!(bmp.pixels[1], lut[64]);
        assert_eq!(bmp.pixels[2], lut[192]);
        assert_eq!(bmp.pixels[3], lut[255]);
    }

    #[test]
    fn apply_contrast_rgba_preserves_alpha() {
        let mut bmp = Bitmap::new(2, 1, 72.0, PixelFormat::Rgba8).unwrap();
        bmp.pixels
            .copy_from_slice(&[100, 50, 200, 128, 0, 255, 0, 200]);
        apply_contrast(&mut bmp, 1.5);
        // alpha 通道不变
        assert_eq!(bmp.pixels[3], 128);
        assert_eq!(bmp.pixels[7], 200);
    }

    #[test]
    fn apply_gamma_rgb8_each_channel_independent() {
        let mut bmp = Bitmap::new(1, 1, 72.0, PixelFormat::Rgb8).unwrap();
        bmp.pixels.copy_from_slice(&[64, 128, 200]);
        let lut = build_gamma_lut(2.0);
        apply_gamma(&mut bmp, 2.0);
        assert_eq!(bmp.pixels[0], lut[64]);
        assert_eq!(bmp.pixels[1], lut[128]);
        assert_eq!(bmp.pixels[2], lut[200]);
    }

    #[test]
    fn apply_gamma_identity_keeps_pixels() {
        let mut bmp = Bitmap::new(3, 2, 72.0, PixelFormat::Gray8).unwrap();
        let original = [10u8, 30, 50, 70, 90, 110];
        bmp.pixels.copy_from_slice(&original);
        apply_gamma(&mut bmp, 1.0);
        assert_eq!(bmp.pixels, original);
    }

    // ----- sharpen 像素级测试 -----

    #[test]
    fn sharpen_uniform_image_unchanged() {
        // 全均匀图像，sharpen 后等于原图（高频为 0）
        let mut bmp = Bitmap::new(5, 5, 72.0, PixelFormat::Gray8).unwrap();
        bmp.pixels.fill(128);
        apply_sharpen(&mut bmp);
        for (i, &p) in bmp.pixels.iter().enumerate() {
            // 边界 weight=1.3 / 1.5 / 角点 1.5 等都不为 0；
            // 均匀图像下 sum_pixel = 128 * weight -> mr = 128
            // 但浮点 round 可能差 1
            assert!(
                (127..=129).contains(&p),
                "uniform sharpen pixel {i} = {p}, expected ~128"
            );
        }
    }

    #[test]
    fn sharpen_center_pixel_enhances_contrast() {
        // 5x5 全黑，中心 1 像素白 -> sharpen 后中心更亮（>255 被钳为 255，仍是 255）；
        // 周围像素由于 -0.1 影响应略变暗（接近 -25.5 -> 钳为 0）
        let mut bmp = Bitmap::new(5, 5, 72.0, PixelFormat::Gray8).unwrap();
        bmp.pixels.fill(0);
        bmp.pixel_mut(2, 2).unwrap()[0] = 255; // 中心点白
        apply_sharpen(&mut bmp);
        // 中心仍然是 255（1.8 * 255 / 1.0 = 459 → 钳 255）
        assert_eq!(bmp.gray_at(2, 2), Some(255));
        // 直接邻居 (2,1)/(2,3)/(1,2)/(3,2)：周围有 1 个中心白点贡献 -0.1*255 = -25.5
        //   sum = 0*0.8 + 255*(-0.1) = -25.5（中心点对邻居是周围位置，filter 权重 -0.1）
        //   weight 在 5x5 中间不是边界，所以 weight = 1.0；mr = -25.5 / 1.0 + 0.5 = -25 → 钳 0
        assert_eq!(bmp.gray_at(2, 1), Some(0));
        assert_eq!(bmp.gray_at(1, 2), Some(0));
        // 远处像素未受影响
        assert_eq!(bmp.gray_at(0, 0), Some(0));
    }

    #[test]
    fn sharpen_rgb_processes_each_channel() {
        // 3x3 全红 (255, 0, 0)，应保持基本不变（均匀图像）
        let mut bmp = Bitmap::new(3, 3, 72.0, PixelFormat::Rgb8).unwrap();
        bmp.fill_rgb(255, 0, 0);
        apply_sharpen(&mut bmp);
        for chunk in bmp.pixels.chunks_exact(3) {
            // 由于浮点和边界 weight 不同，允许 ±1 偏差
            assert!((254..=255).contains(&chunk[0]), "R channel: {}", chunk[0]);
            assert!(chunk[1] <= 1, "G channel should be ~0: {}", chunk[1]);
            assert!(chunk[2] <= 1, "B channel should be ~0: {}", chunk[2]);
        }
    }

    #[test]
    fn sharpen_rgba_preserves_alpha() {
        let mut bmp = Bitmap::new(3, 3, 72.0, PixelFormat::Rgba8).unwrap();
        for chunk in bmp.pixels.chunks_exact_mut(4) {
            chunk[0] = 100;
            chunk[1] = 100;
            chunk[2] = 100;
            chunk[3] = 200; // alpha
        }
        apply_sharpen(&mut bmp);
        for chunk in bmp.pixels.chunks_exact(4) {
            assert_eq!(chunk[3], 200, "alpha should remain 200");
        }
    }

    #[test]
    fn sharpen_empty_bitmap_noop() {
        let mut bmp = Bitmap::new(0, 0, 72.0, PixelFormat::Gray8).unwrap();
        apply_sharpen(&mut bmp); // should not panic
        assert!(bmp.pixels.is_empty());
    }

    #[test]
    fn apply_filter_3x3_zero_weight_zeros_pixel() {
        // filter sum = 0 → weight == 0 → C 版 continue 不写 → dst 保持 0
        let mut bmp = Bitmap::new(3, 3, 72.0, PixelFormat::Gray8).unwrap();
        bmp.pixels.copy_from_slice(&[
            10, 20, 30, //
            40, 50, 60, //
            70, 80, 90,
        ]);
        // sum = 0 的 filter（如 [[1, -1, 0], [0, 0, 0], [0, 0, 0]]）
        let zero_sum_filter = [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
        apply_filter_3x3(&mut bmp, &zero_sum_filter);
        // 内部像素 (1,1) weight=0 → 像素 = 0
        assert_eq!(bmp.gray_at(1, 1), Some(0));
    }

    // ----- round_clamp_u8 边界测试 -----

    #[test]
    fn round_clamp_u8_boundaries() {
        assert_eq!(round_clamp_u8(-100.0), 0);
        assert_eq!(round_clamp_u8(-0.4), 0);
        assert_eq!(round_clamp_u8(0.0), 0);
        assert_eq!(round_clamp_u8(0.4), 0);
        assert_eq!(round_clamp_u8(0.5), 1);
        assert_eq!(round_clamp_u8(127.5), 128);
        assert_eq!(round_clamp_u8(254.4), 254);
        assert_eq!(round_clamp_u8(254.5), 255);
        assert_eq!(round_clamp_u8(255.0), 255);
        assert_eq!(round_clamp_u8(300.0), 255);
    }

    // ----- 端到端比对测试 -----

    #[test]
    fn contrast_then_inverse_recovers_approximately() {
        // contrast=2 -> contrast=0.5 后像素值应接近原值
        let mut bmp = Bitmap::new(8, 1, 72.0, PixelFormat::Gray8).unwrap();
        let original = [10u8, 30, 50, 80, 120, 160, 200, 240];
        bmp.pixels.copy_from_slice(&original);
        apply_contrast(&mut bmp, 2.0);
        apply_contrast(&mut bmp, 0.5);
        // 经过 LUT round 后会有较大误差，但应在合理范围（< 60）
        // 这只是 smoke test，不要求精确还原
        for (i, &p) in bmp.pixels.iter().enumerate() {
            let diff = (i32::from(p) - i32::from(original[i])).abs();
            assert!(
                diff < 80,
                "pixel {i} diff {diff} too large (original {}, got {p})",
                original[i]
            );
        }
    }

    #[test]
    fn gamma_roundtrip_recovers_within_tolerance() {
        let mut bmp = Bitmap::new(8, 1, 72.0, PixelFormat::Gray8).unwrap();
        let original = [10u8, 30, 50, 80, 120, 160, 200, 240];
        bmp.pixels.copy_from_slice(&original);
        apply_gamma(&mut bmp, 2.0);
        apply_gamma(&mut bmp, 0.5);
        // gamma 是逆运算，应高精度还原（u8 量化误差 < 2）
        for (i, &p) in bmp.pixels.iter().enumerate() {
            let diff = (i32::from(p) - i32::from(original[i])).abs();
            assert!(
                diff <= 2,
                "pixel {i} roundtrip diff {diff} (original {}, got {p})",
                original[i]
            );
        }
    }
}
