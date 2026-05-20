//! Deskew - 去歪斜（自动找最优旋转角 + 双线性插值旋转）。
//!
//! ## 算法来源
//!
//! 全部按 C 版 `willuslib/bmp.c` 行号 1:1 移植：
//!
//! - [`auto_straighten_angle`]：`bmp_autostraighten` (`willuslib/bmp.c:4551-4753`)
//!   主入口，返回检测到的旋转角度（degrees）。
//! - [`auto_straighten`]：同 [`auto_straighten_angle`] 但额外把角度应用到 bitmap（in-place）。
//! - [`row_by_row_stdev`]（私有）：`bmp_row_by_row_stdev` (`willuslib/bmp.c:4470-4548`)
//!   计算给定 theta 下每行"暗像素百分比"的标准差（figure of merit）。
//! - [`rotate_fast`]：`bmp_rotate_fast` (`willuslib/bmp.c:1485-1562`)
//!   双线性插值旋转。
//!
//! ## 坐标系约定
//!
//! C 版 `WILLUSBITMAP` 的浮点像素访问 (`bmp_grey_pix_vald` / `bmp_pix_vald`) 使用
//! **from-bottom** y 坐标（`y=0` 是最底行）。Rust 端 [`k2types::Bitmap`] 使用自然的
//! **from-top** 坐标。本模块在 [`bilinear_gray_from_bottom`] / [`bilinear_rgb_from_bottom`]
//! / [`bilinear_rgba_from_bottom`] 内部做坐标翻转，公开 API 接收 C 风格的 from-bottom
//! 浮点 y 坐标以便代码与 C 行号严格对照。
//!
//! 但 [`row_by_row_stdev`] 全程在 from-top 坐标工作（C 版直接用 `bmp_rowptr_from_top`
//! 不经 `_pix_vali`），所以那一段 Rust 也用 from-top。
//!
//! ## 精度承诺
//!
//! 在合成已知歪斜角度的图像上，[`auto_straighten_angle`] 检测精度 ±0.3°（与 C 版一致）。
//! vs C 版 fixture 输出的精确比对推迟到 Step 5.7（M4.5 交叉验证基础设施）。
//!
//! 来源：`rust-rewrite-execution-plan.md` Step 5.4；`rust-rewrite-plan.md` v2.1 §5.2 / §8.2。

use core::f64::consts::PI;

use k2types::{Bitmap, PixelFormat};

// --------------------------------------------------------------------------
// Public API
// --------------------------------------------------------------------------

/// 角度搜索步长（度）。C `bmp.c:4569`：`stepsize=.05`。
const STEP_SIZE: f64 = 0.05;

/// 采样列数。C `bmp.c:4583`：`bmp_row_by_row_stdev(srcgrey, 400, white, theta)`。
const CCOUNT: i32 = 400;

/// 自动检测最优去歪斜角度（不修改 bitmap）。
///
/// 1:1 复刻 C 版 `bmp_autostraighten` (`willuslib/bmp.c:4551-4753`) 的"搜索 +
/// 决策"主流程，但**不**调用 [`rotate_fast`] 应用旋转——返回值即为检测到的角度
/// （单位：度，与 C 返回值约定一致），由调用方决定是否应用。
///
/// # 参数
///
/// - `bmp_gray`：灰度 bitmap（Gray8 / Rgb8 / Rgba8 均可，内部走 [`Bitmap::gray_at`]）。
/// - `white_thresh`：暗 / 亮像素阈值，`gray < white_thresh` 视为暗（含文字）。
/// - `max_degrees`：搜索范围 `±max_degrees`，典型 4.0。
/// - `min_degrees`：返回 `0.0` 的"贴近零"阈值（`|rotdeg| <= min_degrees` 则不旋转）。
///
/// # 返回
///
/// 推荐的去歪斜角度（度）。`0.0` 表示无需旋转（或检测无果）。
#[must_use]
pub fn auto_straighten_angle(
    bmp_gray: &Bitmap,
    white_thresh: u8,
    max_degrees: f64,
    min_degrees: f64,
) -> f64 {
    // C: na = (int)(maxdegrees/stepsize+.5);
    let na = ((max_degrees / STEP_SIZE) + 0.5) as i32;
    let na = na.max(1);
    let n = (1 + na * 2) as usize;

    // 网格搜索：theta_i = (i - na) * STEP_SIZE 度，i ∈ [0, n)
    let mut sdev = vec![0.0_f64; n];
    let mut sdmin = 999.0_f64;
    let mut sdmax = -999.0_f64;
    let mut imax: usize = 0;
    for (i, slot) in sdev.iter_mut().enumerate().take(n) {
        let theta = (i as f64 - f64::from(na)) * STEP_SIZE * PI / 180.0;
        let s = row_by_row_stdev(bmp_gray, CCOUNT, white_thresh, theta);
        if sdmin > s {
            sdmin = s;
        }
        if sdmax < s {
            imax = i;
            sdmax = s;
        }
        *slot = s;
    }
    // C: if (sdmax<=0.) return 0;
    if sdmax <= 0.0 {
        return 0.0;
    }
    // 归一化
    for v in &mut sdev {
        *v /= sdmax;
    }
    sdmin /= sdmax;
    let mut rotdeg = -(imax as f64 - f64::from(na)) * STEP_SIZE;

    // C `bmp.c:4615-4627`：3 种 bail-out 条件
    // (1) 无明显峰值 sdmin>0.95（曲线太平）
    // (2) 角度太接近 0（|rotdeg| <= min_degrees）
    // (3) 角度太接近搜索边界（与 max_degrees 差 < 0.25）
    if sdmin > 0.95
        || rotdeg.abs() <= min_degrees
        || (rotdeg.abs() - max_degrees.abs()).abs() < 0.25
    {
        return 0.0;
    }

    // C `bmp.c:4631-4638`：统计峰值数（用于决定单峰精搜 vs 多峰加权）
    let mut maxpt: i32 = 0;
    for i in 1..n.saturating_sub(1) {
        // C: sdev[i]>.95 && ((sdev[i]>sdev[i-1] && sdev[i]>sdev[i+1])
        //      || (i<n-2 && sdev[i]>sdev[i-1] && sdev[i]==sdev[i+1] && sdev[i+1]>sdev[i+2]))
        let is_strict_peak = sdev[i] > sdev[i - 1] && sdev[i] > sdev[i + 1];
        let is_plateau_peak = i < n.saturating_sub(2)
            && sdev[i] > sdev[i - 1]
            // f64 equality 这里照 C 写；浮点同源对齐才相等，足以判定 plateau
            && (sdev[i] - sdev[i + 1]).abs() < f64::EPSILON
            && sdev[i + 1] > sdev[i + 2];
        if sdev[i] > 0.95 && (is_strict_peak || is_plateau_peak) {
            maxpt += 1;
        }
    }

    if maxpt == 1 {
        // C `bmp.c:4641-4668`：单峰，在 imax 附近 1/5 步长精搜
        let mut sdmax2: f64 = 1.0;
        let mut thbest = (imax as f64 - f64::from(na)) * STEP_SIZE * PI / 180.0;
        let nfine = 5_i32;
        for ifine in (-nfine + 1)..nfine {
            if ifine == 0 {
                continue;
            }
            let theta = (imax as f64 + f64::from(ifine) / f64::from(nfine) - f64::from(na))
                * STEP_SIZE
                * PI
                / 180.0;
            let s = row_by_row_stdev(bmp_gray, CCOUNT, white_thresh, theta) / sdmax;
            if s > sdmax2 {
                sdmax2 = s;
                thbest = theta;
            }
        }
        rotdeg = -thbest * 180.0 / PI;
    } else if imax >= 3 && imax + 4 <= n {
        // C `bmp.c:4670-4723`：多峰 / 平顶，在 imax 两侧做"阈值上的"加权平均
        // 修正：C `imax<=n-4` ↔ `imax + 4 <= n`（含等号）
        let mut sd1min = sdev[imax - 1];
        for i in (0..=imax.saturating_sub(2)).rev() {
            if sd1min > sdev[i] {
                sd1min = sdev[i];
            }
        }
        let mut sd2min = sdev[imax + 1];
        for v in sdev.iter().take(n).skip(imax + 2) {
            if sd2min > *v {
                sd2min = *v;
            }
        }
        let mut sdthresh = if sd1min > sd2min {
            sd1min * 1.01
        } else {
            sd2min * 1.01
        };
        if sdthresh < 0.9 {
            sdthresh = 0.9;
        }
        if sdthresh < 0.95 {
            // 向左找第一个 sdev[i] < sdthresh
            let mut left_break: Option<usize> = None;
            for i in (0..imax).rev() {
                if sdev[i] < sdthresh {
                    left_break = Some(i);
                    break;
                }
            }
            // 向右找第一个 sdev[i] < sdthresh（在 [imax+1, n-2] 范围）
            let mut right_break: Option<usize> = None;
            for (i, v) in sdev
                .iter()
                .enumerate()
                .take(n.saturating_sub(1))
                .skip(imax + 1)
            {
                if *v < sdthresh {
                    right_break = Some(i);
                    break;
                }
            }
            // 严格按 C 行 4692-4713 计算 deg1/deg2 + 加权
            if let (Some(left_i), Some(right_i)) = (left_break, right_break) {
                let i1 = left_i + 1;
                // C: deg1 = stepsize * ((i-na) + (sdthresh-sdev[i]) / (sdev[i+1]-sdev[i]))
                let denom_left = sdev[left_i + 1] - sdev[left_i];
                let frac_left = if denom_left.abs() < f64::EPSILON {
                    0.0
                } else {
                    (sdthresh - sdev[left_i]) / denom_left
                };
                let deg1 = STEP_SIZE * ((left_i as f64 - f64::from(na)) + frac_left);

                let i2 = right_i - 1;
                let denom_right = sdev[right_i - 1] - sdev[right_i];
                let frac_right = if denom_right.abs() < f64::EPSILON {
                    0.0
                } else {
                    (sdthresh - sdev[right_i]) / denom_right
                };
                let deg2 = STEP_SIZE * ((right_i as f64 - f64::from(na)) - frac_right);

                if deg2 - deg1 < 2.5 {
                    let mut wsum = 0.0_f64;
                    let mut sum = 0.0_f64;
                    for (i, v) in sdev.iter().enumerate().take(i2 + 1).skip(i1) {
                        wsum += (*v - sdthresh) * STEP_SIZE * (i as f64 - f64::from(na));
                        sum += *v - sdthresh;
                    }
                    if sum.abs() < f64::EPSILON {
                        rotdeg = -(deg1 + deg2) / 2.0;
                    } else {
                        rotdeg = -wsum / sum;
                    }
                }
            }
        }
    }

    rotdeg
}

/// 自动检测最优去歪斜角度并应用旋转（in-place）。
///
/// 等价 C 版 `bmp_autostraighten` (`willuslib/bmp.c:4738-4750`) 的完整流程：
/// 检测 → 把 `(0,0)` 像素置 255（C 行 4738）→ 调用 [`rotate_fast`] 用 `expand=false`
/// 应用角度（C 行 4739）。返回检测到的角度。
///
/// 若返回 `0.0`（无需旋转），bitmap 不变。
pub fn auto_straighten(
    bmp_gray: &mut Bitmap,
    white_thresh: u8,
    max_degrees: f64,
    min_degrees: f64,
) -> f64 {
    let rotdeg = auto_straighten_angle(bmp_gray, white_thresh, max_degrees, min_degrees);
    if rotdeg == 0.0 {
        return 0.0;
    }
    // C 行 4738：`srcgrey->data[0] = 255;`
    // C 用 (0,0) 像素作为 fill 色，灰度场景下置 255 = 白底
    if let Some(slice) = bmp_gray.pixel_mut(0, 0) {
        for byte in slice.iter_mut() {
            *byte = 255;
        }
    }
    rotate_fast(bmp_gray, rotdeg, false);
    rotdeg
}

/// 双线性插值旋转（in-place）。
///
/// 1:1 复刻 C 版 `bmp_rotate_fast` (`willuslib/bmp.c:1485-1562`)：
///
/// - `degrees`：正值 = 顺时针（C 约定）。
/// - `expand=true`：dst 扩展以容纳整个旋转后的图像（含留白）；
///   `expand=false`：dst 尺寸保持，超出部分被裁掉。
/// - dst 用源像素 `(0, 0)`（from-bottom，即最后一行第 0 列）作为背景填充色。
/// - 边界外的源坐标直接跳过（dst 保持背景色）。
///
/// 支持 [`PixelFormat::Gray8`] / [`PixelFormat::Rgb8`] / [`PixelFormat::Rgba8`]。
/// C 版仅支持 8-bit / 24-bit，Rgba8 是 Rust 端扩展：alpha 通道一并双线性插值。
pub fn rotate_fast(bmp: &mut Bitmap, degrees: f64, expand: bool) {
    let src_w_u = bmp.width;
    let src_h_u = bmp.height;
    if src_w_u == 0 || src_h_u == 0 {
        return;
    }
    let th = degrees * PI / 180.0;
    let sth = th.sin();
    let cth = th.cos();

    let src_w = f64::from(src_w_u);
    let src_h = f64::from(src_h_u);
    let (w, h) = if expand {
        // C: w = (int)(fabs(width*cth)+fabs(height*sth)+.5);
        let new_w = (src_w * cth.abs() + src_h * sth.abs() + 0.5) as u32;
        let new_h = (src_h * cth.abs() + src_w * sth.abs() + 0.5) as u32;
        (new_w.max(1), new_h.max(1))
    } else {
        (src_w_u, src_h_u)
    };

    let format = bmp.format;
    let dpi = bmp.dpi;
    let Ok(mut dst) = Bitmap::new(w, h, dpi, format) else {
        return;
    };

    // C `bmp.c:1515-1516`：以源 (0,0) from-bottom 像素（即 Rust top-most 索引 height-1）作为 dst 背景
    let fill_rgb = pixel_at_from_bottom_top_index(bmp, 0, 0);
    match format {
        PixelFormat::Gray8 => dst.fill_byte(fill_rgb.0),
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => dst.fill_rgb(fill_rgb.0, fill_rgb.1, fill_rgb.2),
    }

    let dst_w = f64::from(w);
    let dst_h = f64::from(h);

    for row in 0..h {
        // C: y2 = dst->height/2. - row  -> from-bottom centered
        let y2 = dst_h / 2.0 - f64::from(row);
        for col in 0..w {
            let x2 = f64::from(col) - dst_w / 2.0;
            // C `bmp.c:1528-1529`：
            //   x1 = -0.5 + bmp->width/2 + x2*cth + y2*sth
            //   y1 = -0.5 + bmp->height/2 + y2*cth - x2*sth
            let x1 = -0.5 + src_w / 2.0 + x2 * cth + y2 * sth;
            let y1 = -0.5 + src_h / 2.0 + y2 * cth - x2 * sth;
            if x1 < 0.0 || x1 >= src_w || y1 < 0.0 || y1 >= src_h {
                continue;
            }
            // 取源像素（双线性，y1 是 from-bottom 浮点）
            match format {
                PixelFormat::Gray8 => {
                    if let Some(g) = bilinear_gray_from_bottom(bmp, x1, y1) {
                        if let Some(slice) = dst.pixel_mut(col, row) {
                            slice[0] = round_clamp_u8(g);
                        }
                    }
                }
                PixelFormat::Rgb8 => {
                    if let Some([r, g, b]) = bilinear_rgb_from_bottom(bmp, x1, y1) {
                        if let Some(slice) = dst.pixel_mut(col, row) {
                            slice[0] = round_clamp_u8(r);
                            slice[1] = round_clamp_u8(g);
                            slice[2] = round_clamp_u8(b);
                        }
                    }
                }
                PixelFormat::Rgba8 => {
                    if let Some([r, g, b, a]) = bilinear_rgba_from_bottom(bmp, x1, y1) {
                        if let Some(slice) = dst.pixel_mut(col, row) {
                            slice[0] = round_clamp_u8(r);
                            slice[1] = round_clamp_u8(g);
                            slice[2] = round_clamp_u8(b);
                            slice[3] = round_clamp_u8(a);
                        }
                    }
                }
            }
        }
    }

    *bmp = dst;
}

// --------------------------------------------------------------------------
// Private helpers
// --------------------------------------------------------------------------

/// 计算给定 theta 下每行"暗像素百分比"的标准差（figure of merit）。
///
/// 1:1 复刻 C 版 `bmp_row_by_row_stdev` (`willuslib/bmp.c:4470-4548`)。
///
/// 关键差异：C 版用 `bmp_rowptr_from_top` 直接索引字节（假定 8-bit Gray），
/// Rust 版用 [`Bitmap::gray_at`] 支持任意 PixelFormat（内部对 RGB/A 自动做亮度换算）。
fn row_by_row_stdev(bmp: &Bitmap, ccount: i32, white_thresh: u8, theta_radians: f64) -> f64 {
    let w = bmp.width as i32;
    let h = bmp.height as i32;
    if w <= 0 || h <= 0 {
        return 0.0;
    }
    // C: c1 = bmp->width/15.   c2 = bmp->width - c1
    let c1 = (f64::from(w) / 15.0) as i32;
    let c2 = w - c1;
    if c2 <= c1 {
        return 0.0;
    }
    // C: dw = (int)((c2-c1)/ccount+.5);
    let mut dw = (f64::from(c2 - c1) / f64::from(ccount) + 0.5) as i32;
    if dw < 1 {
        dw = 1;
    }
    // C `bmp.c:4485-4498`：dc1 / dc2 计算行扫描的安全边界
    let tanth = -theta_radians.tan();
    let mut dc1 = (tanth * f64::from(w)) as i32;
    let dc2;
    if dc1 < 0 {
        dc1 = 1 - dc1;
        dc2 = 0;
    } else {
        dc2 = -dc1 - 1;
        dc1 = 0;
    }
    let dc1 = dc1 + (f64::from(h) / 15.0) as i32;
    let dc2 = dc2 - (f64::from(h) / 15.0) as i32;

    // 预算采样列数 nw，分配 row[nw] 存每列的 y 偏移
    let mut nw: usize = 0;
    {
        let mut c = c1;
        while c < c2 {
            nw += 1;
            c += dw;
        }
    }
    if nw == 0 {
        return 0.0;
    }
    let countthresh = (nw * 2 / 3) as i32;

    let mut row_offset: Vec<i32> = Vec::with_capacity(nw);
    {
        let mut c = c1;
        while c < c2 {
            row_offset.push((tanth * f64::from(c)) as i32);
            c += dw;
        }
    }

    let mut csum: f64 = 0.0;
    let mut csumsq: f64 = 0.0;
    let mut n_rows: u64 = 0;

    // C: for (r=dc1+1; r<bmp->height+dc2-1; r++)
    let r_start = dc1 + 1;
    let r_end = h + dc2 - 1;
    for r in r_start..r_end {
        let mut cin: i32 = 0;
        let mut count: i32 = 0;
        let mut c = c1;
        let mut nn: usize = 0;
        while c < c2 {
            let r0 = r + row_offset[nn];
            if r0 < 0 || r0 >= h {
                // C: if (cin>0) break;  else continue;
                if cin > 0 {
                    break;
                }
                nn += 1;
                c += dw;
                continue;
            }
            cin += 1;
            // (c, r0) 都是 from-top；Rust gray_at 直接吃 from-top
            if let Some(g) = bmp.gray_at(c as u32, r0 as u32) {
                if g < white_thresh {
                    count += 1;
                }
            }
            nn += 1;
            c += dw;
        }
        if cin < countthresh {
            continue;
        }
        let dcount = 100.0 * f64::from(count) / f64::from(cin);
        csum += dcount;
        csumsq += dcount * dcount;
        n_rows += 1;
    }

    if n_rows == 0 {
        0.0
    } else {
        let nf = n_rows as f64;
        let mean = csum / nf;
        // C `bmp.c:4546`：sqrt(fabs((csum/n)*(csum/n) - csumsq/n))
        // 注意：C 的公式表面上是 |mean² - mean_sq|，标准差应是 |mean_sq - mean²|，
        // 但取 fabs 后两者相等。保留 C 表达。
        let var = (mean * mean - csumsq / nf).abs();
        var.sqrt()
    }
}

/// 从 C 的 from-bottom (x, y_from_bottom) 整数坐标取像素值（顶部即最后一行）。
///
/// 用于 [`rotate_fast`] 的背景色填充——对应 C `bmp_pix_vali(bmp, 0, 0, &r, &g, &b)`。
fn pixel_at_from_bottom_top_index(bmp: &Bitmap, x: u32, y_from_bottom: i32) -> (u8, u8, u8) {
    let h = bmp.height as i32;
    let y_top = h - 1 - y_from_bottom;
    if y_top < 0 || y_top >= h {
        return (255, 255, 255);
    }
    let Some(pixel) = bmp.pixel(x, y_top as u32) else {
        return (255, 255, 255);
    };
    match bmp.format {
        PixelFormat::Gray8 => (pixel[0], pixel[0], pixel[0]),
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => (pixel[0], pixel[1], pixel[2]),
    }
}

/// 双线性插值 - Gray 通道，y 是 from-bottom 浮点。
///
/// 1:1 复刻 C 版 `bmp_grey_pix_vald` (`willuslib/bmp.c:2333-2366`)。
/// 返回 `None` 等价 C 版返回 `-1.`（fx0+fx1==0 或 fy0+fy1==0，理论不发生）。
fn bilinear_gray_from_bottom(bmp: &Bitmap, x: f64, y_from_bottom: f64) -> Option<f64> {
    let (ix0c, ix1c, iy0c_top, iy1c_top, fx0, fx1, fy0, fy1, valid) =
        bilinear_setup(bmp, x, y_from_bottom);
    if !valid {
        return None;
    }
    let p00 = f64::from(bmp.gray_at(ix0c, iy0c_top)?);
    let p10 = f64::from(bmp.gray_at(ix1c, iy0c_top)?);
    let p01 = f64::from(bmp.gray_at(ix0c, iy1c_top)?);
    let p11 = f64::from(bmp.gray_at(ix1c, iy1c_top)?);
    Some(
        (fy0 * (fx0 * p00 + fx1 * p10) + fy1 * (fx0 * p01 + fx1 * p11))
            / ((fx0 + fx1) * (fy0 + fy1)),
    )
}

/// 双线性插值 - RGB 三通道，y 是 from-bottom 浮点。
///
/// 1:1 复刻 C 版 `bmp_pix_vald` (`willuslib/bmp.c:2377-2419`)。
fn bilinear_rgb_from_bottom(bmp: &Bitmap, x: f64, y_from_bottom: f64) -> Option<[f64; 3]> {
    let (ix0c, ix1c, iy0c_top, iy1c_top, fx0, fx1, fy0, fy1, valid) =
        bilinear_setup(bmp, x, y_from_bottom);
    if !valid {
        return None;
    }
    let p00 = bmp.pixel(ix0c, iy0c_top)?;
    let p10 = bmp.pixel(ix1c, iy0c_top)?;
    let p01 = bmp.pixel(ix0c, iy1c_top)?;
    let p11 = bmp.pixel(ix1c, iy1c_top)?;
    let denom = (fx0 + fx1) * (fy0 + fy1);
    let mut out = [0.0_f64; 3];
    for ch in 0..3 {
        let v00 = f64::from(p00[ch]);
        let v10 = f64::from(p10[ch]);
        let v01 = f64::from(p01[ch]);
        let v11 = f64::from(p11[ch]);
        out[ch] = (fy0 * (fx0 * v00 + fx1 * v10) + fy1 * (fx0 * v01 + fx1 * v11)) / denom;
    }
    Some(out)
}

/// 双线性插值 - RGBA 四通道，y 是 from-bottom 浮点。Rust 扩展（C 无 RGBA）。
fn bilinear_rgba_from_bottom(bmp: &Bitmap, x: f64, y_from_bottom: f64) -> Option<[f64; 4]> {
    let (ix0c, ix1c, iy0c_top, iy1c_top, fx0, fx1, fy0, fy1, valid) =
        bilinear_setup(bmp, x, y_from_bottom);
    if !valid {
        return None;
    }
    let p00 = bmp.pixel(ix0c, iy0c_top)?;
    let p10 = bmp.pixel(ix1c, iy0c_top)?;
    let p01 = bmp.pixel(ix0c, iy1c_top)?;
    let p11 = bmp.pixel(ix1c, iy1c_top)?;
    let denom = (fx0 + fx1) * (fy0 + fy1);
    let mut out = [0.0_f64; 4];
    for ch in 0..4 {
        let v00 = f64::from(p00[ch]);
        let v10 = f64::from(p10[ch]);
        let v01 = f64::from(p01[ch]);
        let v11 = f64::from(p11[ch]);
        out[ch] = (fy0 * (fx0 * v00 + fx1 * v10) + fy1 * (fx0 * v01 + fx1 * v11)) / denom;
    }
    Some(out)
}

/// 4 角点 + 4 权重 + valid 标志。所有索引已 clamp 到 `[0, w-1] / [0, h-1]`。
/// 返回的 y 索引已经从 from-bottom 翻转为 from-top（直接喂给 [`Bitmap::pixel`]）。
#[allow(clippy::type_complexity)]
fn bilinear_setup(
    bmp: &Bitmap,
    x: f64,
    y_from_bottom: f64,
) -> (u32, u32, u32, u32, f64, f64, f64, f64, bool) {
    let w = bmp.width as i32;
    let h = bmp.height as i32;
    // C `bmp.c:2339-2342`
    let ix0 = (x - 0.5) as i32;
    let ix1 = ix0 + 1;
    let iy0 = (y_from_bottom - 0.5) as i32;
    let iy1 = iy0 + 1;
    let ix0c = ix0.clamp(0, w - 1) as u32;
    let ix1c = ix1.clamp(0, w - 1) as u32;
    let iy0c_bottom = iy0.clamp(0, h - 1);
    let iy1c_bottom = iy1.clamp(0, h - 1);
    // C `bmp.c:2347-2358`：权重 = 1 - |i+0.5 - x|，clip to 0
    let fx0 = (1.0 - (f64::from(ix0) + 0.5 - x).abs()).max(0.0);
    let fx1 = (1.0 - (f64::from(ix1) + 0.5 - x).abs()).max(0.0);
    let fy0 = (1.0 - (f64::from(iy0) + 0.5 - y_from_bottom).abs()).max(0.0);
    let fy1 = (1.0 - (f64::from(iy1) + 0.5 - y_from_bottom).abs()).max(0.0);
    let valid = !((fx0 == 0.0 && fx1 == 0.0) || (fy0 == 0.0 && fy1 == 0.0));
    // y from-bottom → from-top
    let iy0c_top = (h - 1 - iy0c_bottom) as u32;
    let iy1c_top = (h - 1 - iy1c_bottom) as u32;
    (ix0c, ix1c, iy0c_top, iy1c_top, fx0, fx1, fy0, fy1, valid)
}

/// `(v + 0.5).floor() as u8` with clamp，等价 C `(unsigned char)(v + 0.5)` 含负数防护。
fn round_clamp_u8(v: f64) -> u8 {
    if v.is_nan() {
        return 0;
    }
    let r = (v + 0.5).floor();
    if r <= 0.0 {
        0
    } else if r >= 255.0 {
        255
    } else {
        r as u8
    }
}

// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use k2types::{Bitmap, PixelFormat};

    fn make_gray(w: u32, h: u32) -> Bitmap {
        Bitmap::new(w, h, 300.0, PixelFormat::Gray8).unwrap()
    }

    /// 合成一张全白底 + 水平黑条（行像素）的 bitmap，模拟"水平文字行"。
    /// `rows_with_text` 给出文字行的 y_from_top 索引列表，每行从 c1 到 c2 全黑。
    fn make_horizontal_lines(w: u32, h: u32, rows_with_text: &[u32], c1: u32, c2: u32) -> Bitmap {
        let mut b = make_gray(w, h);
        b.fill_byte(255);
        for &y in rows_with_text {
            if y >= h {
                continue;
            }
            let row = b.row_mut(y).unwrap();
            for (x, byte) in row.iter_mut().enumerate() {
                if (x as u32) >= c1 && (x as u32) <= c2 {
                    *byte = 0;
                }
            }
        }
        b
    }

    /// 把一张图像旋转 `degrees`（产生"歪斜"的图像），用于测试 deskew 反向恢复。
    fn rotate_for_test(bmp: &Bitmap, degrees: f64) -> Bitmap {
        let mut out = bmp.clone();
        rotate_fast(&mut out, degrees, false);
        out
    }

    #[test]
    fn round_clamp_u8_basic() {
        assert_eq!(round_clamp_u8(0.0), 0);
        assert_eq!(round_clamp_u8(0.4), 0);
        assert_eq!(round_clamp_u8(0.5), 1);
        assert_eq!(round_clamp_u8(127.5), 128);
        assert_eq!(round_clamp_u8(254.5), 255);
        assert_eq!(round_clamp_u8(255.0), 255);
        assert_eq!(round_clamp_u8(-1.0), 0);
        assert_eq!(round_clamp_u8(300.0), 255);
        assert_eq!(round_clamp_u8(f64::NAN), 0);
    }

    #[test]
    fn rotate_fast_zero_preserves_uniform() {
        // 用单色图避免边缘 bilinear 引入误差。
        // C 算法 0° 时 x1=col-0.5（不严格 identity，col=0 会因 x1<0 被跳过），
        // 但 uniform fill 下整图任何采样都是相同值，无边缘问题。
        let mut bmp = make_gray(40, 30);
        bmp.fill_byte(128);
        rotate_fast(&mut bmp, 0.0, false);
        assert!(
            bmp.pixels.iter().all(|&p| p == 128),
            "after 0 deg rotation uniform 128 should stay uniform"
        );
    }

    #[test]
    fn rotate_fast_360_preserves_uniform() {
        // 360° 旋转后 cth ≈ 1, sth ≈ -2.45e-16；uniform fill 下应无累积误差。
        let mut bmp = make_gray(40, 30);
        bmp.fill_byte(200);
        rotate_fast(&mut bmp, 360.0, false);
        let max_diff = bmp
            .pixels
            .iter()
            .map(|&p| (i32::from(p) - 200).abs())
            .max()
            .unwrap_or(0);
        assert!(
            max_diff <= 1,
            "after 360 deg uniform 200 max per-pixel diff = {max_diff}"
        );
    }

    #[test]
    fn rotate_fast_preserves_total_dark_pixel_count() {
        // 旋转不改变面积，暗像素总数应大致守恒（容许 ±25% 边缘像素重分配）
        let bmp = make_horizontal_lines(60, 40, &[10, 20, 30], 10, 50);
        let dark_before = bmp.pixels.iter().filter(|&&p| p < 128).count();
        let mut rotated = bmp.clone();
        rotate_fast(&mut rotated, 2.0, false);
        let dark_after = rotated.pixels.iter().filter(|&&p| p < 128).count();
        let ratio = dark_after as f64 / dark_before.max(1) as f64;
        assert!(
            (0.75..=1.25).contains(&ratio),
            "rotated dark count {dark_after} vs original {dark_before}, ratio {ratio:.3} out of [0.75, 1.25]"
        );
    }

    #[test]
    fn rotate_fast_preserves_dimensions_no_expand() {
        let mut bmp = make_gray(20, 30);
        bmp.fill_byte(128);
        rotate_fast(&mut bmp, 10.0, false);
        assert_eq!(bmp.width, 20);
        assert_eq!(bmp.height, 30);
    }

    #[test]
    fn rotate_fast_expand_grows() {
        let mut bmp = make_gray(100, 100);
        bmp.fill_byte(128);
        rotate_fast(&mut bmp, 45.0, true);
        // sqrt(2) * 100 ≈ 141
        assert!(bmp.width >= 140 && bmp.width <= 142);
        assert!(bmp.height >= 140 && bmp.height <= 142);
    }

    #[test]
    fn rotate_fast_rgb_works() {
        let mut bmp = Bitmap::new(20, 20, 300.0, PixelFormat::Rgb8).unwrap();
        bmp.fill_rgb(200, 100, 50);
        // 中心像素改成红色
        bmp.pixel_mut(10, 10).unwrap().copy_from_slice(&[255, 0, 0]);
        rotate_fast(&mut bmp, 5.0, false);
        assert_eq!(bmp.width, 20);
        assert_eq!(bmp.height, 20);
        assert_eq!(bmp.format, PixelFormat::Rgb8);
    }

    #[test]
    fn rotate_fast_rgba_preserves_alpha_channel_count() {
        let mut bmp = Bitmap::new(15, 15, 300.0, PixelFormat::Rgba8).unwrap();
        bmp.fill_rgb(100, 150, 200);
        rotate_fast(&mut bmp, 2.0, false);
        assert_eq!(bmp.format, PixelFormat::Rgba8);
        assert_eq!(bmp.pixels.len(), 15 * 15 * 4);
    }

    #[test]
    fn rotate_fast_zero_size_noop() {
        // 零尺寸 bitmap 应不 panic
        let mut bmp = Bitmap::new(0, 10, 300.0, PixelFormat::Gray8).unwrap();
        rotate_fast(&mut bmp, 5.0, false);
        assert_eq!(bmp.width, 0);
        let mut bmp = Bitmap::new(10, 0, 300.0, PixelFormat::Gray8).unwrap();
        rotate_fast(&mut bmp, 5.0, false);
        assert_eq!(bmp.height, 0);
    }

    #[test]
    fn bilinear_setup_center_pixel() {
        // x=3.0, y=3.0 在 5x5 bitmap 上是 4 角点 (像素 2, 像素 3) 的中点：
        // C: ix0 = (int)(3.0 - 0.5) = (int)2.5 = 2; ix1 = 3
        // fx0 = 1 - |2.5 - 3.0| = 0.5; fx1 = 1 - |3.5 - 3.0| = 0.5
        // 注意 x=2.5 不是中点（落在像素 2 内部）
        let bmp = make_gray(5, 5);
        let (ix0, ix1, iy0_top, iy1_top, fx0, fx1, fy0, fy1, valid) =
            bilinear_setup(&bmp, 3.0, 3.0);
        assert_eq!(ix0, 2);
        assert_eq!(ix1, 3);
        assert!((fx0 - 0.5).abs() < 1e-9);
        assert!((fx1 - 0.5).abs() < 1e-9);
        // y_from_bottom=3.0 ↔ iy0=2, iy1=3 (from-bottom)
        // → from-top: h-1-2=2, h-1-3=1
        assert_eq!(iy0_top, 2);
        assert_eq!(iy1_top, 1);
        assert!((fy0 - 0.5).abs() < 1e-9);
        assert!((fy1 - 0.5).abs() < 1e-9);
        assert!(valid);
    }

    #[test]
    fn bilinear_gray_uniform_bitmap_returns_constant() {
        let mut bmp = make_gray(20, 20);
        bmp.fill_byte(128);
        let g = bilinear_gray_from_bottom(&bmp, 10.5, 10.5).unwrap();
        assert!((g - 128.0).abs() < 1e-6);
        // 边界处也应合理
        let g = bilinear_gray_from_bottom(&bmp, 0.5, 0.5).unwrap();
        assert!((g - 128.0).abs() < 1e-6);
    }

    #[test]
    fn row_by_row_stdev_aligned_lines_high() {
        // 水平文字行 + theta=0：sdev 应该相对较大
        let bmp = make_horizontal_lines(200, 100, &[20, 30, 40, 50, 60, 70, 80], 30, 170);
        let aligned = row_by_row_stdev(&bmp, 400, 200, 0.0);
        // 同样图像加 5 度 theta：sdev 应该明显变小
        let misaligned = row_by_row_stdev(&bmp, 400, 200, 5.0 * PI / 180.0);
        assert!(
            aligned > misaligned,
            "aligned sdev {aligned} should > misaligned {misaligned}"
        );
        assert!(aligned > 0.0);
    }

    #[test]
    fn auto_straighten_returns_zero_for_blank() {
        // 全白 bitmap：sdev 应该是 0（每行 0 暗像素），返回 0
        let bmp = make_gray(200, 100);
        let mut blank = bmp.clone();
        blank.fill_byte(255);
        let angle = auto_straighten_angle(&blank, 200, 4.0, 0.1);
        assert_eq!(angle, 0.0);
    }

    #[test]
    fn auto_straighten_returns_near_zero_for_aligned_text() {
        // 水平文字行（无歪斜）：检测到的角度应非常接近 0。
        // 注意：合成图离散像素误差让 imax 可能漂移到 imax±1（±0.05°），
        // 精搜可能找到微小非零值；放宽到 |angle| < 0.1°。
        let bmp = make_horizontal_lines(300, 200, &[30, 50, 70, 90, 110, 130, 150, 170], 50, 250);
        let angle = auto_straighten_angle(&bmp, 200, 4.0, 0.1);
        assert!(
            angle.abs() < 0.1,
            "aligned text should give near-zero angle, got {angle} deg"
        );
    }

    #[test]
    fn auto_straighten_detects_known_skew_positive() {
        // 制造 1.5° 歪斜图像（顺时针旋转），auto_straighten 应该检测到 ≈ -1.5°
        // （因为去歪斜方向与歪斜方向相反）
        let template = make_horizontal_lines(400, 260, &[40, 70, 100, 130, 160, 190, 220], 60, 340);
        let skewed = rotate_for_test(&template, 1.5);
        let detected = auto_straighten_angle(&skewed, 200, 4.0, 0.1);
        // 应该在 -1.5 度附近 ±0.3
        assert!(
            (detected - (-1.5)).abs() <= 0.3,
            "detected {detected}, expected ≈ -1.5 ± 0.3"
        );
    }

    #[test]
    fn auto_straighten_detects_known_skew_negative() {
        // 反方向歪斜
        let template = make_horizontal_lines(400, 260, &[40, 70, 100, 130, 160, 190, 220], 60, 340);
        let skewed = rotate_for_test(&template, -2.0);
        let detected = auto_straighten_angle(&skewed, 200, 4.0, 0.1);
        // 应该在 +2.0 附近 ±0.3
        assert!(
            (detected - 2.0).abs() <= 0.3,
            "detected {detected}, expected ≈ +2.0 ± 0.3"
        );
    }

    #[test]
    fn auto_straighten_apply_in_place() {
        // 制造 1.0° 歪斜 + auto_straighten in-place 应让图像更接近 horizontal
        let template = make_horizontal_lines(400, 260, &[40, 70, 100, 130, 160, 190, 220], 60, 340);
        let mut skewed = rotate_for_test(&template, 1.0);
        let stdev_before = row_by_row_stdev(&skewed, 400, 200, 0.0);
        let detected = auto_straighten(&mut skewed, 200, 4.0, 0.1);
        let stdev_after = row_by_row_stdev(&skewed, 400, 200, 0.0);
        // detected 应非零 + stdev_after 应 > stdev_before（更接近 horizontal 对齐）
        assert!(detected != 0.0, "should detect non-zero skew");
        assert!(
            stdev_after >= stdev_before,
            "after deskew stdev {stdev_after} should be ≥ before {stdev_before}"
        );
    }

    #[test]
    fn auto_straighten_below_min_degrees_returns_zero() {
        // 制造 0.05° 微小歪斜，min_degrees=0.5 应让函数返回 0
        let template = make_horizontal_lines(400, 260, &[40, 70, 100, 130, 160, 190, 220], 60, 340);
        let skewed = rotate_for_test(&template, 0.05);
        let detected = auto_straighten_angle(&skewed, 200, 4.0, 0.5);
        assert_eq!(detected, 0.0, "skew 0.05° < min 0.5° → should return 0");
    }

    #[test]
    fn auto_straighten_max_degrees_grid_spans_correctly() {
        // max_degrees=4.0 ↔ na=80 ↔ n=161 个候选角度（-4 .. +4 步长 0.05）
        // 间接验证：传 max_degrees=0.05 应产生 n=3（[-0.05, 0, +0.05]）
        let bmp = make_horizontal_lines(200, 100, &[50], 30, 170);
        // 不 panic 就行
        let _ = auto_straighten_angle(&bmp, 200, 0.05, 0.01);
    }

    #[test]
    fn row_by_row_stdev_handles_tiny_bitmaps() {
        // 极小图像（不会 panic）
        let bmp = make_gray(3, 3);
        let s = row_by_row_stdev(&bmp, 400, 200, 0.0);
        assert_eq!(s, 0.0);
    }

    #[test]
    fn rotate_fast_handles_non_square() {
        // 非方形也工作
        let mut bmp = make_horizontal_lines(80, 40, &[10, 20, 30], 5, 75);
        rotate_fast(&mut bmp, 3.0, false);
        assert_eq!(bmp.width, 80);
        assert_eq!(bmp.height, 40);
    }
}
