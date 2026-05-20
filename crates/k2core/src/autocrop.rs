//! Autocrop —— 自动裁剪（基于 trim margins + 智能边缘判定，避免裁掉脚注/页码）。
//!
//! ## 算法来源（C 版 `k2pdfoptlib/k2bmp.c`）
//!
//! - [`auto_crop`]：`bmp_autocrop2` (`k2bmp.c:1216-1339`)。主入口，输入 aggressiveness ∈ [0,1]，
//!   返回 [`AutoCropResult`]（`AutoCropMargins` + success 标志）。
//! - [`apply_auto_crop`]：`k2bmp_apply_autocrop` (`k2bmp.c:1345-1370`)。把外面区域填 255（白）。
//! - 私有 [`autocrop_search`]：`bmp_autocrop2_ex` (`k2bmp.c:1422-1576`)。4 层嵌套搜索 +
//!   `frame_area`/`frame_black_percentage`/`frame_stdev_norm` 几何度量。
//! - 私有 [`autocrop_refine`]：`bmp_autocrop_refine` (`k2bmp.c:1582-1730`)。逐列/逐行精修
//!   边界（用 [`xsmooth`] + [`find_threshold`]）。
//! - 私有 [`bmp_integer_resample_gray`]：`bmp_integer_resample_ex` (`willuslib/bmp.c:2574-2632`)。
//!   整数下采样到 1-byte/pixel grayscale。
//!
//! ## 边距语义（与 C 版 `cx[4]` 完全一致）
//!
//! [`AutoCropMargins`] 四个字段是"半绝对半相对"语义：
//! - `left` / `top`：从左/上边数的像素数（裁后左/上界 = `left` / `top`）
//! - `right` / `bottom`：从右/下边数的像素数（裁后右/下界 = `width-1-right` / `height-1-bottom`）
//!
//! 这种混合表达对应 C `bmp_autocrop2` 末尾的:
//! ```text
//! cx[2] = bmp->width  - 1 - cx[2];   // k2bmp.c:1330
//! cx[3] = bmp->height - 1 - cx[3];   // k2bmp.c:1331
//! ```
//! 让 `masterinfo->autocrop_margins[4]` 在跨页累积时直接是"边距像素数"，与 C 版无缝互换。
//!
//! ## 精度承诺
//!
//! 算法 1:1 复刻 C 版（含 [`xsmooth`] 的 C 原版 bug——`sum+=y[i]` 应为 `y[j]`，使得 xsmooth
//! 实际为 no-op；保留 bug 行为以匹配 C 输出）。在合成已知裁剪框的图像上，边距精度
//! ±少量像素。vs C 版 fixture 输出的精确比对推迟到 Step 5.7（M4.5 交叉验证基础设施）。
//!
//! 来源：`rust-rewrite-execution-plan.md` Step 5.5；`rust-rewrite-plan.md` v2.1 §5.2 / §8.2。

use k2types::{Bitmap, PixelFormat};

// ---------------------------------------------------------------------------
// 公开数据结构
// ---------------------------------------------------------------------------

/// Autocrop 计算出的四个边距（像素数）。
///
/// 语义对应 C 版 `masterinfo->autocrop_margins[4]`：
/// - `left` / `top`：从左/上边数的像素数（裁后左/上保留边界 = 这些值）
/// - `right` / `bottom`：从右/下边数的像素数（裁后右/下保留边界 = `width-1-right` / `height-1-bottom`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AutoCropMargins {
    /// 左边距像素数（裁后左界）
    pub left: i32,
    /// 上边距像素数（裁后上界）
    pub top: i32,
    /// 右边距像素数（裁后右界 = `width-1-right`）
    pub right: i32,
    /// 下边距像素数（裁后下界 = `height-1-bottom`）
    pub bottom: i32,
}

impl AutoCropMargins {
    /// 从 C 版 `cx[4]` 数组构造（顺序与 C 一致：left/top/right/bottom）。
    #[must_use]
    pub const fn from_cx(cx: [i32; 4]) -> Self {
        Self {
            left: cx[0],
            top: cx[1],
            right: cx[2],
            bottom: cx[3],
        }
    }

    /// 转换为 C 版 `cx[4]` 数组（用于与移植代码对照）。
    #[must_use]
    pub const fn to_cx(&self) -> [i32; 4] {
        [self.left, self.top, self.right, self.bottom]
    }

    /// 全 0 边距（noop autocrop 的初值）。
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        }
    }
}

/// [`auto_crop`] 返回值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoCropResult {
    /// 计算得到的边距。
    pub margins: AutoCropMargins,
    /// C 版 `status` (`max_area >= 0.0`)。`false` 表示未找到合理裁剪框，调用方应使用全 0 边距。
    pub success: bool,
}

// ---------------------------------------------------------------------------
// 公开 API
// ---------------------------------------------------------------------------

/// 自动检测裁剪边距（不修改 `bmp`）。
///
/// 1:1 复刻 C 版 `bmp_autocrop2` (`k2bmp.c:1216-1339`)：
/// 1. 把 `bmp` 转灰度副本；
/// 2. 算亮度直方图，取累计 30% 像素的"白上限" `whitemax`；
/// 3. 根据宽高比定下采样步长 `pw` / `ph`；
/// 4. 算白阈值 `wt = 192 + (whitemax-192)*(pw-1)/pw`；
/// 5. 调 [`autocrop_search`] 在下采样灰度图上 4 层嵌套搜索最优框；
/// 6. 调 [`autocrop_refine`] 在原图上精修边界；
/// 7. 把右/下边界翻转为"边距" (`cx[2] = width-1-cx[2]`)。
///
/// # 参数
///
/// - `bmp`：源位图（Gray8 / Rgb8 / Rgba8 均可，内部转灰度）。
/// - `aggressiveness`：0.0 ~ 1.0，对应 C 版 `k2settings->autocrop / 1000.0`。
///   越大越激进（更乐意裁掉看起来"不重要"的边缘）。`blackweight = aggressiveness * 50.0`。
///
/// # 返回
///
/// [`AutoCropResult`]：含 `margins` 与 `success` 标志。`success=false` 时 margins 已是合理回退值
/// （搜索失败时是缩小到 minarea 的对称裁剪，但调用方可选择忽略改用全 0 边距）。
///
/// # 边界
///
/// - 空 bitmap（width 或 height = 0）：返回 `success=false` + 全 0 margins。
/// - `aggressiveness` clamp 到 [0, 1]。
#[must_use]
pub fn auto_crop(bmp: &Bitmap, aggressiveness: f64) -> AutoCropResult {
    // 防御性参数处理（C 版无 clamp，但 aggressiveness 来自 settings 通常已规整）
    let aggressiveness = aggressiveness.clamp(0.0, 1.0);

    let w = bmp.width;
    let h = bmp.height;
    if w == 0 || h == 0 {
        return AutoCropResult {
            margins: AutoCropMargins::zero(),
            success: false,
        };
    }

    // C k2bmp.c:1230 - bmp_copy + bmp_convert_to_grayscale
    let gray = bmp_to_grayscale_buffer(bmp);
    let w_i = w as i32;
    let h_i = h as i32;

    // C k2bmp.c:1232-1242 - 亮度直方图
    let mut hist = [0u64; 256];
    for &v in &gray {
        hist[v as usize] += 1;
    }

    // C k2bmp.c:1243 - blackweight
    let blackweight = aggressiveness * 50.0;

    // C k2bmp.c:1268-1270 - whitemax: 从最白往低累加，到达 30% 像素时停
    let s30 = 0.3 * (w as f64) * (h as f64);
    let whitemax = compute_whitemax(&hist, s30);

    // C k2bmp.c:1274-1287 - 根据宽高比决定下采样步长
    let (pw, ph) = if h > w {
        (((w_i / 150).max(1)), ((h_i / 200).max(1)))
    } else {
        (((w_i / 200).max(1)), ((h_i / 150).max(1)))
    };

    // C k2bmp.c:1288 - 白阈值 wt
    let wt = 192 + (whitemax - 192) * (pw - 1) / pw;

    // C k2bmp.c:1296 - 调用核心搜索
    // 参数：pixwidthx=pw, pixstepx=pw, pixwidthy=ph, pixstepy=ph, whitethresh=wt,
    //       blackweight, minarea=0.6, threshold=0.05
    let mut cx = [0i32; 4];
    let success = autocrop_search(
        &gray,
        w_i,
        h_i,
        pw,
        pw,
        ph,
        ph,
        wt,
        blackweight,
        0.6,
        0.05,
        &mut cx,
    );

    // C k2bmp.c:1330-1331 - 把 cx[2]/cx[3] 翻转为"边距"
    cx[2] = w_i - 1 - cx[2];
    cx[3] = h_i - 1 - cx[3];

    AutoCropResult {
        margins: AutoCropMargins::from_cx(cx),
        success,
    }
}

/// 把 [`AutoCropMargins`] 的"外部"区域填白（255）。
///
/// 1:1 复刻 C 版 `k2bmp_apply_autocrop` (`k2bmp.c:1345-1370`)：
/// - 先把 right/bottom 边距翻转回绝对坐标（`cx[2] = width-1-cx[2]`）；
/// - 列 i < `left` 或 i > `width-1-right` → 整列填 255；
/// - 行 j < `top` 或 j > `height-1-bottom` → 整行填 255。
///
/// # 支持
///
/// - Gray8：填 255
/// - Rgb8：填 (255, 255, 255)
/// - Rgba8：填 (255, 255, 255, 255)（C 版无 RGBA，Rust 端约定 alpha 一并填白）
///
/// # 边界
///
/// 若 margins 已经将整个 bitmap 都判定为"外部"（如 `left > width-1-right`），整张图填白。
pub fn apply_auto_crop(bmp: &mut Bitmap, margins: &AutoCropMargins) {
    let w = bmp.width as i32;
    let h = bmp.height as i32;
    if w == 0 || h == 0 {
        return;
    }
    // C k2bmp.c:1351-1354 - 拷贝并翻转 right/bottom
    let left = margins.left;
    let top = margins.top;
    let right_abs = w - 1 - margins.right;
    let bottom_abs = h - 1 - margins.bottom;

    let bpp = bmp.format.bytes_per_pixel();
    let fill = white_pixel_bytes(bmp.format);

    // C k2bmp.c:1355-1362 - 列 i<left 或 i>right_abs 整列填白
    for j in 0..(h as u32) {
        if let Some(row) = bmp.row_mut(j) {
            for i in 0..w {
                if i < left || i > right_abs {
                    let off = (i as usize) * bpp;
                    row[off..off + bpp].copy_from_slice(&fill[..bpp]);
                }
            }
        }
    }
    // C k2bmp.c:1363-1369 - 行 i<top 或 i>bottom_abs 整行填白
    for j in 0..h {
        if j < top || j > bottom_abs {
            if let Some(row) = bmp.row_mut(j as u32) {
                let bpr = row.len();
                // 整行用 fill 平铺
                let mut written = 0;
                while written < bpr {
                    let take = (bpr - written).min(bpp);
                    row[written..written + take].copy_from_slice(&fill[..take]);
                    written += take;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 内部 helper：灰度化 + 白阈值计算
// ---------------------------------------------------------------------------

/// 把任意 PixelFormat 的 Bitmap 转为 1-byte/pixel 灰度 `Vec<u8>`（length = width*height）。
///
/// 对应 C 版 `bmp_copy + bmp_convert_to_grayscale` 的组合，但产出裸字节缓冲而非新 Bitmap，
/// 因为后续 `frame_*` / `autocrop_refine` 等 helper 都假设 1-byte/pixel 紧凑布局，直接用
/// `&[u8]` + width/height 比 &Bitmap 更贴近 C 风格 `bmp_rowptr_from_top(bmp, r) + c`。
fn bmp_to_grayscale_buffer(bmp: &Bitmap) -> Vec<u8> {
    let n = (bmp.width as usize) * (bmp.height as usize);
    let mut out = Vec::with_capacity(n);
    match bmp.format {
        PixelFormat::Gray8 => {
            out.extend_from_slice(&bmp.pixels);
        }
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => {
            // gray_at 走 0.299R + 0.587G + 0.114B（与 fill_rgb 的灰度回退一致）
            for y in 0..bmp.height {
                for x in 0..bmp.width {
                    out.push(bmp.gray_at(x, y).unwrap_or(255));
                }
            }
        }
    }
    out
}

/// `whitemax`: 从亮度 255 往下累加直方图，累加值首次 `>= s30` 时返回的亮度索引。
///
/// 对应 C k2bmp.c:1269 `for (i=255,sum=0.;sum<s30;sum+=hist[i],i--);` 跳出时的 `i+1`
/// 因为 C 版 `i--` 在 sum>=s30 已经发生时仍执行了一次。
/// 等价的直观写法：返回"最后一次累加完后的 i"。
fn compute_whitemax(hist: &[u64; 256], s30: f64) -> i32 {
    let mut sum = 0.0_f64;
    let mut i: i32 = 255;
    while sum < s30 && i >= 0 {
        sum += hist[i as usize] as f64;
        i -= 1;
    }
    // C 风格：循环退出时 i 已经被 decrement 一次，最后一次"被累加"的索引是 i+1。
    // 但 C 版 whitemax = i（decrement 后的值），这里保持与 C 字面一致。
    i
}

/// 根据 PixelFormat 返回"白色"像素的字节数组。
fn white_pixel_bytes(format: PixelFormat) -> [u8; 4] {
    match format {
        PixelFormat::Gray8 => [255, 0, 0, 0],
        PixelFormat::Rgb8 => [255, 255, 255, 0],
        PixelFormat::Rgba8 => [255, 255, 255, 255],
    }
}

// ---------------------------------------------------------------------------
// 内部 helper：frame 几何度量
// ---------------------------------------------------------------------------

/// C `frame_area` (`k2bmp.c:1807-1811`)：归一化框面积。
fn frame_area(bmp_area: f64, cx: &[i32; 4]) -> f64 {
    let dw = (cx[2] + 1 - cx[0]) as f64;
    let dh = (cx[3] + 1 - cx[1]) as f64;
    dw * dh / bmp_area
}

/// C `frame_black_percentage` (`k2bmp.c:1919-1957`)：周长黑像素百分比。
///
/// - `flags == 1`：仅左侧（含 4 角，长度 = h+2）
/// - `flags == 2`：仅顶部（长度 = w）
/// - `flags == 3`：全周长（top + bottom 长度 w，left + right 长度 h-2 各算一次）
///
/// 返回 `1 - mean_brightness/255`，即 0=全白 1=全黑。
fn frame_black_percentage(bw: &[u8], stride: i32, cx: &[i32; 4], flags: u8) -> f64 {
    let w = cx[2] - cx[0] + 1;
    let mut h = cx[3] - cx[1] + 1 - 2;
    let dr = stride as usize;

    // C: p = rowptr(cx[1]) + cx[0]
    let p_base = (cx[1] as usize) * dr + (cx[0] as usize);
    // C: p1 = p + dr   (row cx[1]+1, leftmost)
    let mut p1_idx = p_base + dr;
    // C: p2 = rowptr(cx[1]+1) + cx[2]
    let mut p2_idx = ((cx[1] + 1) as usize) * dr + (cx[2] as usize);
    // C: p3 = rowptr(cx[3]) + cx[0]
    let p3_base = (cx[3] as usize) * dr + (cx[0] as usize);

    let mut sum: u64 = 0;
    let len: i32;

    if flags == 3 {
        // C: top + bottom 行 (w pixels each)
        for i in 0..w {
            sum += bw[p_base + i as usize] as u64;
            sum += bw[p3_base + i as usize] as u64;
        }
        // C: left + right 列 (h-2 pixels each, 排除上下两个角点)
        for _ in 0..h {
            sum += bw[p1_idx] as u64;
            sum += bw[p2_idx] as u64;
            p1_idx += dr;
            p2_idx += dr;
        }
        len = 2 * w + 2 * h;
    } else if flags == 1 {
        // C k2bmp.c:1942-1947 - 左侧含上下角 (h+2 pixels)
        h += 2;
        // C: p1 -= dr - 即 p1 起点回到 top row at cx[0]
        // 等价：p1_idx = p_base = (cx[1])*dr + cx[0]
        let mut p1_back = p_base;
        for _ in 0..h {
            sum += bw[p1_back] as u64;
            p1_back += dr;
        }
        len = h;
    } else {
        // flags == 2: top row only
        for i in 0..w {
            sum += bw[p_base + i as usize] as u64;
        }
        len = w;
    }

    1.0 - (sum as f64) / (255.0 * len as f64)
}

/// C `frame_stdev_norm` (`k2bmp.c:1844-1912`)：周长方差归一化（相邻像素差的均值 / 255）。
///
/// 返回 4 条边（按 flags 决定）中"相邻像素差均值最大的那条"除以 255，作为该边附近
/// 是否压着文字行的指标（黑/白交替强 → stdev 大）。
fn frame_stdev_norm(bw: &[u8], stride: i32, cx: &[i32; 4], flags: u8) -> f64 {
    let w = cx[2] - cx[0] + 1;
    let mut h = cx[3] - cx[1] + 1 - 2;
    let dr = stride as usize;

    let p_base = (cx[1] as usize) * dr + (cx[0] as usize);
    let mut p1_idx = p_base + dr;
    if flags == 1 {
        h += 2;
        p1_idx = p_base; // C: p1 -= dr
    }
    let p2_base = ((cx[1] + 1) as usize) * dr + (cx[2] as usize);
    let p3_base = (cx[3] as usize) * dr + (cx[0] as usize);

    let mut stdev = 0.0_f64;

    if flags != 1 {
        // top 横向相邻差
        let mut sum: u64 = 0;
        for i in 0..(w - 1) {
            let v1 = bw[p_base + i as usize] as i32;
            let v2 = bw[p_base + (i + 1) as usize] as i32;
            sum += (v2 - v1).unsigned_abs() as u64;
        }
        let stdev0 = (sum as f64) / ((w - 1) as f64);
        if flags == 2 {
            return stdev0 / 255.0;
        }
        if stdev0 > stdev {
            stdev = stdev0;
        }
        // bottom 横向相邻差
        let mut sum2: u64 = 0;
        for i in 0..(w - 1) {
            let v1 = bw[p3_base + i as usize] as i32;
            let v2 = bw[p3_base + (i + 1) as usize] as i32;
            sum2 += (v2 - v1).unsigned_abs() as u64;
        }
        let stdev_bottom = (sum2 as f64) / ((w - 1) as f64);
        if stdev_bottom > stdev {
            stdev = stdev_bottom;
        }
    }

    // left 列纵向相邻差（始终扫描）
    let mut sum_left: u64 = 0;
    let mut idx = p1_idx;
    for _ in 0..(h - 1) {
        let v1 = bw[idx] as i32;
        let v2 = bw[idx + dr] as i32;
        sum_left += (v2 - v1).unsigned_abs() as u64;
        idx += dr;
    }
    let stdev_left = (sum_left as f64) / ((h - 1) as f64);
    if flags == 1 {
        return stdev_left / 255.0;
    }
    if stdev_left > stdev {
        stdev = stdev_left;
    }

    // right 列纵向相邻差（仅 flags==3）
    let mut sum_right: u64 = 0;
    let mut idx = p2_base;
    for _ in 0..(h - 1) {
        let v1 = bw[idx] as i32;
        let v2 = bw[idx + dr] as i32;
        sum_right += (v2 - v1).unsigned_abs() as u64;
        idx += dr;
    }
    let stdev_right = (sum_right as f64) / ((h - 1) as f64);
    if stdev_right > stdev {
        stdev = stdev_right;
    }

    stdev / 255.0
}

// ---------------------------------------------------------------------------
// 内部 helper：整数下采样（bmp_integer_resample_ex 灰度版）
// ---------------------------------------------------------------------------

/// 整数下采样灰度 buffer。
///
/// 对应 C `bmp_integer_resample_ex` (`willuslib/bmp.c:2574-2632`)：每个 `nx*ny` 像素块
/// 求平均（+ `dc*dr/2` 半舍入）得到一个目标像素。
fn bmp_integer_resample_gray(
    src: &[u8],
    src_w: i32,
    src_h: i32,
    nx: i32,
    ny: i32,
) -> (Vec<u8>, i32, i32) {
    debug_assert!(nx >= 1 && ny >= 1);
    let dst_w = (src_w + nx - 1) / nx;
    let dst_h = (src_h + ny - 1) / ny;
    let mut dst = vec![0u8; (dst_w as usize) * (dst_h as usize)];
    for drow in 0..dst_h {
        let r1 = drow * ny;
        let r2 = (r1 + ny).min(src_h);
        let dr = (r2 - r1) as usize;
        for dcol in 0..dst_w {
            let c1 = dcol * nx;
            let c2 = (c1 + nx).min(src_w);
            let dc = (c2 - c1) as usize;
            let half = (dc * dr) / 2;
            let mut pixsum = half;
            for row in r1..r2 {
                let row_base = (row as usize) * (src_w as usize);
                for col in c1..c2 {
                    pixsum += src[row_base + col as usize] as usize;
                }
            }
            pixsum /= dc * dr;
            dst[(drow as usize) * (dst_w as usize) + dcol as usize] = pixsum.min(255) as u8;
        }
    }
    (dst, dst_w, dst_h)
}

// ---------------------------------------------------------------------------
// 内部 helper：autocrop_search（核心 4 层嵌套搜索）
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn autocrop_search(
    bmp_gray: &[u8],
    bmp_w: i32,
    bmp_h: i32,
    pixwidthx: i32,
    pixstepx_in: i32,
    pixwidthy: i32,
    pixstepy_in: i32,
    whitethresh: i32,
    blackweight: f64,
    minarea: f64,
    threshold: f64,
    cx_out: &mut [i32; 4],
) -> bool {
    // C k2bmp.c:1433-1438 - pixstep 重整：pixstep / pixwidth 四舍五入到至少 1
    let pixstepx = (((pixstepx_in + pixwidthx / 2) / pixwidthx).max(1)).max(1);
    let pixstepy = (((pixstepy_in + pixwidthy / 2) / pixwidthy).max(1)).max(1);

    // C k2bmp.c:1441 - 下采样到 bw（1-byte grayscale）
    let (bw, bw_w, bw_h) = bmp_integer_resample_gray(bmp_gray, bmp_w, bmp_h, pixwidthx, pixwidthy);

    let mut max_area = -999.0_f64;
    let bmp_area = (bw_w as f64) * (bw_h as f64);
    let mut cx_best = [0i32, 0i32, bw_w - 1, bw_h - 1];
    let mut cx = [0i32; 4];

    // C k2bmp.c:1477-1556 - 4 层嵌套循环（cx[0] left, cx[1] top, cx[2] right, cx[3] bottom）
    // 每层用 "for (;1;step) { ... if (cond) break; ... }" 风格，靠 frame_area<minarea 主动 break
    cx[0] = 0;
    loop {
        cx[1] = 0;
        cx[2] = bw_w - 1;
        cx[3] = bw_h - 1;
        if frame_area(bmp_area, &cx) < minarea {
            break;
        }
        loop {
            cx[2] = bw_w - 1;
            cx[3] = bw_h - 1;
            if frame_area(bmp_area, &cx) < minarea {
                break;
            }
            loop {
                cx[3] = bw_h - 1;
                if frame_area(bmp_area, &cx) < minarea {
                    break;
                }
                loop {
                    let area = frame_area(bmp_area, &cx);
                    if area < minarea {
                        break;
                    }
                    let black = frame_black_percentage(&bw, bw_w, &cx, 3);
                    let stdev = frame_stdev_norm(&bw, bw_w, &cx, 3);
                    let areaw = area - blackweight * (black + 3.0 * stdev);
                    if areaw > max_area {
                        max_area = areaw;
                        cx_best.copy_from_slice(&cx);
                        break; // C 版同款：找到更大 areaw 立即跳出 cx[3] 内循环
                    }
                    cx[3] -= pixstepy;
                    if cx[3] < cx[1] {
                        break;
                    }
                }
                cx[2] -= pixstepx;
                if cx[2] < cx[0] {
                    break;
                }
            }
            cx[1] += pixstepy;
            if cx[1] >= bw_h {
                break;
            }
        }
        cx[0] += pixstepx;
        if cx[0] >= bw_w {
            break;
        }
    }

    // C k2bmp.c:1558-1565 - 把 cx 从 bw 坐标缩放回 bmp 坐标
    cx_out[0] = cx_best[0] * pixwidthx;
    cx_out[1] = cx_best[1] * pixwidthy;
    cx_out[2] = (cx_best[2] + 1) * pixwidthx - 1;
    cx_out[3] = (cx_best[3] + 1) * pixwidthy - 1;
    if cx_out[2] > bmp_w - 1 {
        cx_out[2] = bmp_w - 1;
    }
    if cx_out[3] > bmp_h - 1 {
        cx_out[3] = bmp_h - 1;
    }

    // C k2bmp.c:1571 - 精修边界
    autocrop_refine(
        bmp_gray,
        bmp_w,
        bmp_h,
        whitethresh,
        threshold,
        pixwidthx,
        pixwidthy,
        cx_out,
    );

    // C k2bmp.c:1575 - return max_area >= 0.0
    max_area >= 0.0
}

// ---------------------------------------------------------------------------
// 内部 helper：autocrop_refine（4 方向独立精修）
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn autocrop_refine(
    bmp_gray: &[u8],
    bmp_w: i32,
    bmp_h: i32,
    _whitethresh: i32, // C 版传入但仅在 #if WILLUSDEBUGX 调试块用，主算法不使用
    threshold: f64,
    pixwidthx: i32,
    pixwidthy: i32,
    cx: &mut [i32; 4],
) {
    let cx_orig = *cx;
    let mut cnew = cx_orig;

    // ----- 左 / 右 边界精修（沿宽度方向）-----
    {
        let n = bmp_w;
        let mut hist = vec![0.0_f64; n as usize];
        let mut x0 = vec![0.0_f64; n as usize];

        let mut cx0 = cx_orig;
        let mut sum = 0.0_f64;
        let mut c = 0_i32;
        // C k2bmp.c:1614-1628 - 对每个候选 left (cx0[0]=0..n) 算 black+3*stdev
        for i in 0..n {
            cx0[0] = i;
            let black = frame_black_percentage(bmp_gray, bmp_w, &cx0, 1);
            let stdev = frame_stdev_norm(bmp_gray, bmp_w, &cx0, 1);
            x0[i as usize] = (i - cx_orig[0]) as f64 / (cx_orig[2] - cx_orig[0] + 1) as f64;
            hist[i as usize] = black + 3.0 * stdev;
            if x0[i as usize] >= 0.25 && x0[i as usize] <= 0.75 {
                c += 1;
                sum += hist[i as usize];
            }
        }
        // C k2bmp.c:1629-1632 - 用中部 25-75% 均值归一化
        let norm = if c > 0 { sum / c as f64 } else { 1.0 };
        for v in &mut hist {
            *v /= norm;
        }
        // C k2bmp.c:1635-1642 - xsmooth (含 bug，no-op 等价)
        let smooth_aperture = (pixwidthx / 3).max(1);
        xsmooth(&mut hist, smooth_aperture);
        // C k2bmp.c:1643 - find_threshold 找新左界
        let frac = find_threshold(&x0, &hist, threshold);
        cnew[0] = cx_orig[0] + (frac * (cx_orig[2] - cx_orig[0] + 1) as f64) as i32;
        // C k2bmp.c:1657-1660 - 对右侧：x0[i] = 1-x0[i]，sortxyd（升序），find_threshold
        for v in x0.iter_mut() {
            *v = 1.0 - *v;
        }
        sortxy_ascending_by_x(&mut x0, &mut hist);
        let frac_right = find_threshold(&x0, &hist, threshold);
        cnew[2] = cx_orig[2] + 1 - (frac_right * (cx_orig[2] - cx_orig[0] + 1) as f64) as i32;
    }

    // ----- 上 / 下 边界精修（沿高度方向）-----
    {
        let n = bmp_h;
        let mut hist = vec![0.0_f64; n as usize];
        let mut x0 = vec![0.0_f64; n as usize];

        let mut cx0 = cx_orig;
        let mut sum = 0.0_f64;
        let mut c = 0_i32;
        for i in 0..n {
            cx0[1] = i;
            let black = frame_black_percentage(bmp_gray, bmp_w, &cx0, 2);
            let stdev = frame_stdev_norm(bmp_gray, bmp_w, &cx0, 2);
            x0[i as usize] = (i - cx_orig[1]) as f64 / (cx_orig[3] - cx_orig[1] + 1) as f64;
            hist[i as usize] = black + 3.0 * stdev;
            if x0[i as usize] >= 0.25 && x0[i as usize] <= 0.75 {
                c += 1;
                sum += hist[i as usize];
            }
        }
        let norm = if c > 0 { sum / c as f64 } else { 1.0 };
        for v in &mut hist {
            *v /= norm;
        }
        let smooth_aperture = (pixwidthy / 3).max(1);
        xsmooth(&mut hist, smooth_aperture);
        let frac = find_threshold(&x0, &hist, threshold);
        cnew[1] = cx_orig[1] + (frac * (cx_orig[3] - cx_orig[1] + 1) as f64) as i32;
        for v in x0.iter_mut() {
            *v = 1.0 - *v;
        }
        sortxy_ascending_by_x(&mut x0, &mut hist);
        let frac_bottom = find_threshold(&x0, &hist, threshold);
        cnew[3] = cx_orig[3] + 1 - (frac_bottom * (cx_orig[3] - cx_orig[1] + 1) as f64) as i32;
    }

    *cx = cnew;
}

// ---------------------------------------------------------------------------
// 内部 helper：xsmooth / find_threshold / indexxd / sortxyd
// ---------------------------------------------------------------------------

/// 滑动平均（窗口 cwin = 2*dn+1）。
///
/// **C 版 bug 复刻**：原 C k2bmp.c:1733-1757 `xsmooth` 内层循环写的是 `sum += y[i]`
/// 而非 `sum += y[j]`，导致 `y2[i] = cwin*y[i]/cwin = y[i]`——xsmooth 实际是 no-op。
/// 为与 C 版输出 1:1 对齐，本实现保留该 bug 行为（核心区段保持原值，仅复制头尾，
/// 等价于不平滑）。若 Step 5.7 决定修复，需同步修订 C 端或承认 intentional divergence。
fn xsmooth(y: &mut [f64], cwin: i32) {
    let n = y.len() as i32;
    if cwin < 1 || n < 1 {
        return;
    }
    let dn = (cwin - 1) / 2;
    // C bug：sum += y[i] for j in [i-dn..i+dn]，最终 y2[i] = y[i]
    // 等价：核心区段拷贝自身，无操作
    // C k2bmp.c:1750-1753 头尾用 y2[dn] 与 y2[n-dn-1] 填充
    // 因 y2 在核心区段 == y，故头尾填充值 = y[dn] 与 y[n-dn-1]
    if dn >= n {
        return;
    }
    let head_val = y[dn as usize];
    let tail_val = y[(n - dn - 1) as usize];
    for i in 0..dn.min(n) {
        y[i as usize] = head_val;
    }
    for i in (n - dn).max(0)..n {
        y[i as usize] = tail_val;
    }
}

/// 在 `x[]` 单调递增数组中找最大的 i 使得 `x[i] <= x0`。
///
/// 1:1 复刻 C `indexxd` (`willuslib/math.c:125-147`)：
/// - `x0 < x[0]` 返回 -1
/// - `x0 >= x[n-1]` 返回 n-1
/// - 否则二分定位
fn indexxd(x0: f64, x: &[f64]) -> i64 {
    let n = x.len();
    if n == 0 {
        return -1;
    }
    if x0 < x[0] {
        return -1;
    }
    if x0 >= x[n - 1] {
        return (n - 1) as i64;
    }
    // C 用 hop-and-bisect 的混合算法；Rust 用稳定的 partition_point
    // partition_point: returns first i where !(x[i] <= x0)，即第一个 > x0 的位置
    let pp = x.partition_point(|&v| v <= x0);
    pp as i64 - 1
}

/// C `find_threshold` (`k2bmp.c:1760-1804`)：在 `(x, y)` 序列中找首次跨越 `thresh` 的 x 值。
///
/// 算法：
/// 1. `i0 = indexxd(0, x)` 找 x=0 附近索引（精修边界的参考点）
/// 2. `max = max(y[i])` 限制在 `x ∈ [0, 1]` 范围
/// 3. 若 max < 0.2 直接返回 0（信号太弱，不精修）
/// 4. 从 i0 起向左右各扫一遍找 `imin`（局部最小值）
/// 5. `thresh = y[imin] + (max - y[imin]) * threshold`
/// 6. 从 imin 起向右扫，找首次 `y[i] >= thresh` 的位置 i，返回 `x[i-1]`
fn find_threshold(x: &[f64], y: &[f64], threshold: f64) -> f64 {
    let n = x.len();
    if n == 0 {
        return 0.0;
    }
    let i0 = {
        let mut idx = indexxd(0.0, x);
        if idx < 0 {
            idx = 0;
        }
        if idx > n as i64 - 1 {
            idx = n as i64 - 1;
        }
        idx as usize
    };
    let mut max = 0.0_f64;
    for i in 0..n {
        if x[i] < 0.0 || x[i] > 1.0 {
            continue;
        }
        if y[i] > max {
            max = y[i];
        }
    }
    if max < 0.2 {
        return 0.0;
    }
    let mut imin = i0;
    // 向右扫
    for i in i0..n {
        if x[i] > 0.35 || y[i] > 0.1 * (max - y[i0]) {
            break;
        }
        if y[i] < y[imin] {
            imin = i;
        }
    }
    // 向左扫
    for i in (0..=i0).rev() {
        if x[i] < -0.2 || y[i] > 0.1 * (max - y[i0]) {
            break;
        }
        if y[i] < y[imin] {
            imin = i;
        }
    }
    let thresh = y[imin] + (max - y[imin]) * threshold;
    let mut i_found = n; // 等价 C 版"for 没 break 时 i==n"
    for i in imin..n {
        if x[i] > 0.35 || y[i] >= thresh {
            i_found = i;
            break;
        }
    }
    // C k2bmp.c:1803 - v2.52 fix: i<1 时返回 x[0]，否则 x[i-1]
    if i_found < 1 {
        x[0]
    } else {
        // 注意：若内层没 break（i_found == n），C 版 `return x[i-1]` = x[n-1]，同样 valid
        x[(i_found - 1).min(n - 1)]
    }
}

/// 对 `(x, y)` 双数组按 `x` 升序排序（联动 y）。
///
/// C `sortxyd` 是 in-place 堆排序；Rust 用 stdlib sort_by 通过中间 index 数组联动。
/// 行为与 C 等价（对相等 x 的 y 顺序，C 不稳定 Rust 这里也不保证稳定，但 autocrop_refine
/// 中 x 由 `1 - i/N` 生成，相邻元素 x 严格不等，因此稳定性差异不影响算法）。
fn sortxy_ascending_by_x(x: &mut [f64], y: &mut [f64]) {
    debug_assert_eq!(x.len(), y.len());
    let n = x.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| {
        x[a].partial_cmp(&x[b])
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    let x_sorted: Vec<f64> = idx.iter().map(|&i| x[i]).collect();
    let y_sorted: Vec<f64> = idx.iter().map(|&i| y[i]).collect();
    x.copy_from_slice(&x_sorted);
    y.copy_from_slice(&y_sorted);
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use k2types::PixelFormat;

    fn make_gray(w: u32, h: u32, fill: u8) -> Bitmap {
        let mut bmp = Bitmap::new(w, h, 300.0, PixelFormat::Gray8).unwrap();
        bmp.fill_byte(fill);
        bmp
    }

    fn paint_black_rect(bmp: &mut Bitmap, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..=y1.min(bmp.height - 1) {
            for x in x0..=x1.min(bmp.width - 1) {
                if let Some(p) = bmp.pixel_mut(x, y) {
                    for b in p.iter_mut() {
                        *b = 0;
                    }
                }
            }
        }
    }

    // ---- frame_area ----

    #[test]
    fn frame_area_full() {
        // 100x100 frame on 100x100 area = 1.0
        let cx = [0, 0, 99, 99];
        assert!((frame_area(10000.0, &cx) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn frame_area_half() {
        // 50x50 on 100x100 = 0.25
        let cx = [0, 0, 49, 49];
        assert!((frame_area(10000.0, &cx) - 0.25).abs() < 1e-9);
    }

    // ---- frame_black_percentage ----

    #[test]
    fn frame_black_percentage_all_white_returns_zero() {
        let bw = vec![255_u8; 100];
        let cx = [0, 0, 9, 9];
        let pct = frame_black_percentage(&bw, 10, &cx, 3);
        assert!(pct.abs() < 1e-9, "all white pct={pct}");
    }

    #[test]
    fn frame_black_percentage_all_black_returns_one() {
        let bw = vec![0_u8; 100];
        let cx = [0, 0, 9, 9];
        let pct = frame_black_percentage(&bw, 10, &cx, 3);
        assert!((pct - 1.0).abs() < 1e-9, "all black pct={pct}");
    }

    #[test]
    fn frame_black_percentage_top_only_flag2() {
        // 10x10 buffer，top row 全黑，其他全白
        let mut bw = vec![255_u8; 100];
        for px in bw.iter_mut().take(10) {
            *px = 0;
        }
        let cx = [0, 0, 9, 9];
        let pct = frame_black_percentage(&bw, 10, &cx, 2);
        assert!((pct - 1.0).abs() < 1e-9, "top-only black pct={pct}");
    }

    #[test]
    fn frame_black_percentage_left_only_flag1() {
        // 10x10 buffer，left col 全黑（含角点）
        let mut bw = vec![255_u8; 100];
        for r in 0..10 {
            bw[r * 10] = 0;
        }
        let cx = [0, 0, 9, 9];
        let pct = frame_black_percentage(&bw, 10, &cx, 1);
        assert!((pct - 1.0).abs() < 1e-9, "left-only black pct={pct}");
    }

    // ---- frame_stdev_norm ----

    #[test]
    fn frame_stdev_norm_uniform_returns_zero() {
        let bw = vec![128_u8; 100];
        let cx = [0, 0, 9, 9];
        let s = frame_stdev_norm(&bw, 10, &cx, 3);
        assert!(s.abs() < 1e-9, "uniform stdev={s}");
    }

    #[test]
    fn frame_stdev_norm_top_alternating() {
        // 10x10，top row 黑白交替（255,0,255,0,...）
        let mut bw = vec![128_u8; 100];
        for (i, px) in bw.iter_mut().take(10).enumerate() {
            *px = if i % 2 == 0 { 255 } else { 0 };
        }
        let cx = [0, 0, 9, 9];
        let s = frame_stdev_norm(&bw, 10, &cx, 2);
        // 每相邻像素差均 255，stdev0 = 255*(w-1)/(w-1) = 255，归一化 = 1.0
        assert!((s - 1.0).abs() < 1e-9, "alternating stdev={s}");
    }

    // ---- bmp_to_grayscale_buffer ----

    #[test]
    fn grayscale_buffer_gray8_passthrough() {
        let mut bmp = make_gray(4, 4, 0);
        for y in 0..4 {
            for x in 0..4 {
                bmp.pixel_mut(x, y).unwrap()[0] = (y * 4 + x) as u8;
            }
        }
        let buf = bmp_to_grayscale_buffer(&bmp);
        assert_eq!(buf, (0..16u8).collect::<Vec<_>>());
    }

    #[test]
    fn grayscale_buffer_rgb_uses_luminance() {
        let mut bmp = Bitmap::new(2, 1, 300.0, PixelFormat::Rgb8).unwrap();
        // 像素 0: 纯红 (255, 0, 0) - 灰度 ≈ 76
        // 像素 1: 纯绿 (0, 255, 0) - 灰度 ≈ 150
        bmp.pixels.copy_from_slice(&[255, 0, 0, 0, 255, 0]);
        let buf = bmp_to_grayscale_buffer(&bmp);
        assert_eq!(buf.len(), 2);
        // 0.299*255 = 76.245 -> round 76
        assert!(buf[0] >= 75 && buf[0] <= 77, "red->gray={}", buf[0]);
        // 0.587*255 = 149.685 -> round 150
        assert!(buf[1] >= 149 && buf[1] <= 151, "green->gray={}", buf[1]);
    }

    // ---- bmp_integer_resample_gray ----

    #[test]
    fn resample_2x2_to_1x1_averages() {
        // 2x2: [0, 100, 200, 0] (left-to-right, top-to-bottom)
        let src = vec![0, 100, 200, 0_u8];
        let (dst, w, h) = bmp_integer_resample_gray(&src, 2, 2, 2, 2);
        assert_eq!((w, h), (1, 1));
        // avg = (0+100+200+0+2)/4 = 75（C 风格 half=dc*dr/2=2 半舍入）
        assert_eq!(dst[0], 75);
    }

    #[test]
    fn resample_with_remainder_uses_partial_block() {
        // 3x1, nx=2 -> dst 2x1: [avg(0,100), 200]
        let src = vec![0, 100, 200_u8];
        let (dst, w, h) = bmp_integer_resample_gray(&src, 3, 1, 2, 1);
        assert_eq!((w, h), (2, 1));
        // dst[0]: dc=2, dr=1, half=1, sum=(0+100+1)/2 = 50
        assert_eq!(dst[0], 50);
        // dst[1]: dc=1, dr=1, half=0, sum=(200+0)/1 = 200
        assert_eq!(dst[1], 200);
    }

    // ---- indexxd ----

    #[test]
    fn indexxd_below_first_returns_minus_one() {
        let x = vec![1.0, 2.0, 3.0];
        assert_eq!(indexxd(0.5, &x), -1);
    }

    #[test]
    fn indexxd_above_last_returns_last() {
        let x = vec![1.0, 2.0, 3.0];
        assert_eq!(indexxd(5.0, &x), 2);
    }

    #[test]
    fn indexxd_middle_finds_lower_bound() {
        let x = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        // 2.5 in [2.0, 3.0) -> index 1
        assert_eq!(indexxd(2.5, &x), 1);
    }

    #[test]
    fn indexxd_exact_match_returns_index() {
        let x = vec![1.0, 2.0, 3.0];
        // x0 == x[1]，partition_point 返回首个 v>x0 = 2，pp-1 = 1
        assert_eq!(indexxd(2.0, &x), 1);
    }

    // ---- find_threshold ----

    #[test]
    fn find_threshold_low_max_returns_zero() {
        // max < 0.2 时直接返回 0
        let x: Vec<f64> = (0..10).map(|i| i as f64 / 9.0).collect();
        let y: Vec<f64> = vec![0.05; 10];
        let r = find_threshold(&x, &y, 0.1);
        assert!(r.abs() < 1e-9);
    }

    #[test]
    fn find_threshold_returns_valid_position_for_peak() {
        // 构造一个明显的峰值：左半 0，中间 1，右半 0
        let x: Vec<f64> = (0..20).map(|i| i as f64 / 19.0).collect();
        let mut y = vec![0.0; 20];
        for v in &mut y[8..12] {
            *v = 1.0;
        }
        let r = find_threshold(&x, &y, 0.1);
        // 应返回 [0, 0.5] 范围内的值（峰值开始之前）
        assert!((0.0..=1.0).contains(&r), "find_threshold returned {r}");
    }

    // ---- xsmooth bug 复刻 ----

    #[test]
    fn xsmooth_is_noop_due_to_c_bug() {
        // C 版 xsmooth 因 bug 实际为 no-op；本实现也应保持原值（除头尾填充外）
        let mut y = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        let original = y.clone();
        xsmooth(&mut y, 3);
        // dn=1, head_val = y[1] = 2.0; tail_val = y[5] = 6.0
        // y[0] 应被填为 head_val=2.0；其他保持
        assert!(
            (y[0] - 2.0).abs() < 1e-9,
            "head fill expected 2.0 got {}",
            y[0]
        );
        assert!(
            (y[6] - 6.0).abs() < 1e-9,
            "tail fill expected 6.0 got {}",
            y[6]
        );
        // 核心区段保持原值
        for i in 1..6 {
            assert!(
                (y[i] - original[i]).abs() < 1e-9,
                "y[{i}] changed: {} vs {}",
                y[i],
                original[i]
            );
        }
    }

    // ---- sortxy_ascending_by_x ----

    #[test]
    fn sortxy_keeps_pairs_linked() {
        let mut x = vec![3.0, 1.0, 2.0];
        let mut y = vec![30.0, 10.0, 20.0];
        sortxy_ascending_by_x(&mut x, &mut y);
        assert_eq!(x, vec![1.0, 2.0, 3.0]);
        assert_eq!(y, vec![10.0, 20.0, 30.0]);
    }

    // ---- compute_whitemax ----

    #[test]
    fn whitemax_with_pure_white_hist() {
        // 全 255 像素 100 个，s30 = 30，从 i=255 倒数 hist[255]=100 一步到位
        let mut hist = [0u64; 256];
        hist[255] = 100;
        let s30 = 30.0;
        let wm = compute_whitemax(&hist, s30);
        // C 行为：sum=0<30，sum+=hist[255]=100，i=254；下次 sum=100>=30 退出，返回 i=254
        assert_eq!(wm, 254);
    }

    // ---- AutoCropMargins ----

    #[test]
    fn margins_from_cx_roundtrip() {
        let m = AutoCropMargins::from_cx([10, 20, 30, 40]);
        assert_eq!(m.left, 10);
        assert_eq!(m.top, 20);
        assert_eq!(m.right, 30);
        assert_eq!(m.bottom, 40);
        assert_eq!(m.to_cx(), [10, 20, 30, 40]);
    }

    #[test]
    fn margins_zero_all_fields_zero() {
        let m = AutoCropMargins::zero();
        assert_eq!(m.to_cx(), [0, 0, 0, 0]);
    }

    // ---- apply_auto_crop ----

    #[test]
    fn apply_auto_crop_fills_outside_white_gray8() {
        let mut bmp = make_gray(10, 10, 0); // 全黑
        let margins = AutoCropMargins {
            left: 2,
            top: 2,
            right: 2,
            bottom: 2,
        };
        apply_auto_crop(&mut bmp, &margins);
        // 内部 (2..=7, 2..=7) 应保持 0；外部应为 255
        assert_eq!(bmp.pixel(0, 0).unwrap()[0], 255);
        assert_eq!(bmp.pixel(1, 5).unwrap()[0], 255);
        assert_eq!(bmp.pixel(8, 5).unwrap()[0], 255);
        assert_eq!(bmp.pixel(5, 1).unwrap()[0], 255);
        assert_eq!(bmp.pixel(5, 8).unwrap()[0], 255);
        // 内部仍为 0
        assert_eq!(bmp.pixel(2, 2).unwrap()[0], 0);
        assert_eq!(bmp.pixel(7, 7).unwrap()[0], 0);
        assert_eq!(bmp.pixel(5, 5).unwrap()[0], 0);
    }

    #[test]
    fn apply_auto_crop_rgb_fills_white_triplet() {
        let mut bmp = Bitmap::new(6, 4, 300.0, PixelFormat::Rgb8).unwrap();
        bmp.pixels.fill(0); // 全黑
        let margins = AutoCropMargins {
            left: 1,
            top: 1,
            right: 1,
            bottom: 1,
        };
        apply_auto_crop(&mut bmp, &margins);
        // (0,0) 外部 -> 255,255,255
        let p = bmp.pixel(0, 0).unwrap();
        assert_eq!(p, [255_u8, 255, 255]);
        // (3, 2) 内部 -> 0,0,0
        let p = bmp.pixel(3, 2).unwrap();
        assert_eq!(p, [0_u8, 0, 0]);
    }

    #[test]
    fn apply_auto_crop_rgba_fills_white_with_alpha() {
        let mut bmp = Bitmap::new(4, 4, 300.0, PixelFormat::Rgba8).unwrap();
        bmp.pixels.fill(0); // 全黑透明
        let margins = AutoCropMargins {
            left: 1,
            top: 1,
            right: 1,
            bottom: 1,
        };
        apply_auto_crop(&mut bmp, &margins);
        let p = bmp.pixel(0, 0).unwrap();
        assert_eq!(p, [255_u8, 255, 255, 255]);
        let p = bmp.pixel(2, 2).unwrap();
        assert_eq!(p, [0_u8, 0, 0, 0]);
    }

    #[test]
    fn apply_auto_crop_zero_margins_noop() {
        let mut bmp = make_gray(5, 5, 100);
        let before = bmp.pixels.clone();
        apply_auto_crop(&mut bmp, &AutoCropMargins::zero());
        assert_eq!(bmp.pixels, before);
    }

    #[test]
    fn apply_auto_crop_zero_size_bitmap_returns_silently() {
        let mut bmp = Bitmap::new(0, 0, 300.0, PixelFormat::Gray8).unwrap();
        apply_auto_crop(&mut bmp, &AutoCropMargins::zero());
        assert!(bmp.pixels.is_empty());
    }

    // ---- auto_crop 端到端 ----

    #[test]
    fn auto_crop_zero_size_returns_failure() {
        let bmp = Bitmap::new(0, 0, 300.0, PixelFormat::Gray8).unwrap();
        let r = auto_crop(&bmp, 0.5);
        assert!(!r.success);
        assert_eq!(r.margins, AutoCropMargins::zero());
    }

    #[test]
    fn auto_crop_pure_white_aggressiveness_zero() {
        let bmp = make_gray(60, 80, 255);
        let r = auto_crop(&bmp, 0.0);
        // 全白图：算法搜索找最大白框（cx_best 充满下采样图），精修 find_threshold 因 max<0.2
        // 返回 0 → cnew[2]=w（"右越界一像素"占位）→ 翻转后 right=-1 / bottom=-1
        // 这是 C 版 documented corner case：margin=-1 等价于"不裁该边"
        // （apply_auto_crop 的判定 `i > w-1-(-1) = w` 永远 false 不污染像素）
        assert!(r.margins.left >= 0, "left={}", r.margins.left);
        assert!(r.margins.top >= 0, "top={}", r.margins.top);
        assert!(r.margins.right >= -1, "right={}", r.margins.right);
        assert!(r.margins.bottom >= -1, "bottom={}", r.margins.bottom);
        // 保留区宽 = w - left - right 必须 > 0
        assert!(60 - r.margins.left - r.margins.right > 0);
        assert!(80 - r.margins.top - r.margins.bottom > 0);
    }

    #[test]
    fn auto_crop_aggressiveness_clamped_to_unit() {
        let bmp = make_gray(40, 40, 200);
        // 越界值应被 clamp，不应 panic
        let r_neg = auto_crop(&bmp, -1.0);
        let r_huge = auto_crop(&bmp, 100.0);
        assert!(r_neg.margins.left >= 0);
        assert!(r_huge.margins.left >= 0);
    }

    #[test]
    fn auto_crop_white_page_with_centered_content() {
        // 80x100 白底，中间 (20,30)-(59,69) 黑块
        let mut bmp = make_gray(80, 100, 255);
        paint_black_rect(&mut bmp, 20, 30, 59, 69);
        let r = auto_crop(&bmp, 0.5);
        // 期望：margins 大致能裁到黑块附近（但因算法激进度+精修可能保留小白边）
        let cx = r.margins.to_cx();
        let kept_left = cx[0];
        let kept_top = cx[1];
        let kept_right_abs = 80 - 1 - cx[2];
        let kept_bottom_abs = 100 - 1 - cx[3];
        // 保留区应该至少覆盖整个黑块
        assert!(kept_left <= 20, "kept_left={kept_left} should be <= 20");
        assert!(kept_top <= 30, "kept_top={kept_top} should be <= 30");
        assert!(
            kept_right_abs >= 59,
            "kept_right_abs={kept_right_abs} should be >= 59"
        );
        assert!(
            kept_bottom_abs >= 69,
            "kept_bottom_abs={kept_bottom_abs} should be >= 69"
        );
    }

    #[test]
    fn auto_crop_then_apply_preserves_content_region() {
        let mut bmp = make_gray(60, 80, 255);
        paint_black_rect(&mut bmp, 15, 20, 44, 59);
        let r = auto_crop(&bmp, 0.3);
        apply_auto_crop(&mut bmp, &r.margins);
        // 内部黑块（15..44, 20..59）应仍是 0
        // 注意 autocrop 可能裁掉部分黑块边缘像素，所以只检查中心
        assert_eq!(bmp.pixel(30, 40).unwrap()[0], 0);
        assert_eq!(bmp.pixel(25, 30).unwrap()[0], 0);
    }
}
