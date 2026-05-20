//! preprocess 集成测试 - 覆盖 contrast / gamma / sharpen 三滤镜在真实图像数据上的行为。
//!
//! ## 测试输入策略
//!
//! v2.1 §10 M3 + Step 5.3 操作清单要求"跑 fixture 单页 + diff golden"。但本工程的
//! `tests/golden/<fixture>/c-pages/page-*.png` 是 **k2pdfopt C 版处理后**的输出 PNG
//! （Step 2.4 baseline），不是"预处理前"的对比基准。换言之：当前 baseline 工具链
//! 没有保存 `apply_contrast/gamma/sharpen` 各自的 single-stage 输出。
//!
//! 严格 "diff ≤ 1%" 对比推迟到 Step 5.7（M4.5 交叉验证基础设施）补完
//! pipeline + golden 生成器。本步骤的集成测试覆盖：
//!
//! - **合成图像精确比对**：用已知输入（ramp / checker / 单点亮斑）验证 LUT 与卷积输出
//! - **fixture 数据 smoke**：12 fixture 的首页 PNG → 三滤镜逐个跑 → 不 panic + 尺寸/格式不变 + 像素值变化符合滤镜性质
//! - **统计性断言**：contrast↑ → 标准差↑；gamma>1 → 平均亮度↑；sharpen → 高频能量↑
//!
//! 来源：`rust-rewrite-execution-plan.md` Step 5.3；C 源 `willuslib/bmp.c:3345-3673`。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use k2core::{
    apply_contrast, apply_filter_3x3, apply_gamma, apply_sharpen, build_contrast_lut,
    build_gamma_lut, read_png, Bitmap, PixelFormat,
};
use std::path::{Path, PathBuf};

// --------------------------------------------------------------------------
// 工具函数
// --------------------------------------------------------------------------

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("golden")
}

fn load_first_page_png(fixture_name: &str) -> Option<Bitmap> {
    let path = golden_dir()
        .join(fixture_name)
        .join("c-pages")
        .join("page-0001.png");
    if !path.exists() {
        return None;
    }
    load_png_with_default_dpi(&path)
}

fn load_png_with_default_dpi(path: &Path) -> Option<Bitmap> {
    read_png(path, 150.0).ok()
}

fn pixel_mean(bmp: &Bitmap) -> f64 {
    if bmp.pixels.is_empty() {
        return 0.0;
    }
    let sum: u64 = bmp.pixels.iter().map(|&p| u64::from(p)).sum();
    sum as f64 / bmp.pixels.len() as f64
}

fn pixel_stddev(bmp: &Bitmap) -> f64 {
    if bmp.pixels.is_empty() {
        return 0.0;
    }
    let mean = pixel_mean(bmp);
    let var: f64 = bmp
        .pixels
        .iter()
        .map(|&p| {
            let d = f64::from(p) - mean;
            d * d
        })
        .sum::<f64>()
        / bmp.pixels.len() as f64;
    var.sqrt()
}

/// 简单高频能量估计：相邻像素差的绝对值之和。Sharpen 后应增大。
fn high_freq_energy(bmp: &Bitmap) -> u64 {
    let bpp = bmp.format.bytes_per_pixel();
    let bpr = bmp.bytes_per_row();
    let mut sum: u64 = 0;
    for y in 0..bmp.height as usize {
        for x in 1..bmp.width as usize {
            let off = y * bpr + x * bpp;
            let prev_off = y * bpr + (x - 1) * bpp;
            for c in 0..bpp.min(3) {
                let d = i32::from(bmp.pixels[off + c]) - i32::from(bmp.pixels[prev_off + c]);
                sum += d.unsigned_abs() as u64;
            }
        }
    }
    sum
}

fn make_gray_ramp(width: u32, height: u32) -> Bitmap {
    let mut bmp = Bitmap::new(width, height, 72.0, PixelFormat::Gray8).unwrap();
    for y in 0..height {
        for x in 0..width {
            let v = ((u64::from(x) + u64::from(y) * u64::from(width)) % 256) as u8;
            bmp.pixel_mut(x, y).unwrap()[0] = v;
        }
    }
    bmp
}

// --------------------------------------------------------------------------
// 合成图像精确比对
// --------------------------------------------------------------------------

#[test]
fn contrast_lut_exact_known_values() {
    // C 行 3345-3377 已知输出（手算）：
    // contrast=1.5, i=64:
    //   x_raw = (64-127.5)/127.5 = -63.5/127.5 ≈ -0.49804
    //   sgn = -1（x_raw<0），c>0 不翻转
    //   x = 0.49804
    //   |c|=1.5，C 源 `if (fabs(contrast)>1.5)` 严格大于 → 走线性
    //   y = 1.5 * 0.49804 = 0.74706
    //   y_final = 127.5 + 0.74706 * (-1) * 127.5 = 127.5 - 95.25 = 32.25
    //   round_half_up(32.25) = 32
    let lut = build_contrast_lut(1.5);
    assert_eq!(
        lut[64], 32,
        "contrast=1.5 lut[64] expected 32, got {}",
        lut[64]
    );

    // i=192: x_raw = 64.5/127.5 ≈ 0.50588（非严格对称，因为 192-127.5=64.5 ≠ 127.5-64=63.5）
    //   y = 1.5 * 0.50588 = 0.75882
    //   y_final = 127.5 + 0.75882 * 127.5 = 127.5 + 96.75 = 224.25
    //   round_half_up(224.25) = 224
    assert_eq!(
        lut[192], 224,
        "contrast=1.5 lut[192] expected 224, got {}",
        lut[192]
    );

    // 严格对称对照 i=63 vs i=192:
    //   i=63: x_raw = -64.5/127.5 ≈ -0.50588 → 镜像于 i=192
    //   lut[63] + lut[192] ≈ 256（±1 舍入残差）
    let sum = i32::from(lut[63]) + i32::from(lut[192]);
    assert!(
        (255..=256).contains(&sum),
        "lut[63]({})+lut[192]({})={} should be ~256",
        lut[63],
        lut[192],
        sum
    );
}

#[test]
fn gamma_lut_exact_known_values() {
    // gamma=2.2, gc = 1/2.2 ≈ 0.4545
    // i=128: 255 * (128/255)^0.4545 + 0.5 = 255 * 0.7375 + 0.5 ≈ 188.07 + 0.5 = 188.57 → 188
    let lut = build_gamma_lut(2.2);
    let i = 128;
    let expected = (255.0 * (i as f64 / 255.0).powf(1.0 / 2.2) + 0.5) as u8;
    assert_eq!(
        lut[i], expected,
        "gamma=2.2 lut[128] expected {}, got {}",
        expected, lut[i]
    );

    // i=0 应为 0，i=255 应为 255（gamma 在 0/255 端点恒等）
    assert_eq!(lut[0], 0);
    assert_eq!(lut[255], 255);
}

#[test]
fn contrast_increases_stddev_on_ramp() {
    // contrast > 1 应增大动态范围 → 标准差增大
    let original = make_gray_ramp(64, 64);
    let stddev_before = pixel_stddev(&original);

    let mut bmp = original.clone();
    apply_contrast(&mut bmp, 1.5);
    let stddev_after = pixel_stddev(&bmp);

    assert!(
        stddev_after > stddev_before,
        "contrast 1.5 stddev after {stddev_after} <= before {stddev_before}"
    );
}

#[test]
fn contrast_zero_collapses_all_to_midgray() {
    let mut bmp = make_gray_ramp(32, 32);
    apply_contrast(&mut bmp, 0.0);
    // 全部钳到 127 或 128
    for &p in &bmp.pixels {
        assert!(
            p == 127 || p == 128,
            "contrast=0 pixel should be 127/128, got {p}"
        );
    }
}

#[test]
fn gamma_gt_one_brightens_ramp_mean() {
    let original = make_gray_ramp(64, 64);
    let mean_before = pixel_mean(&original);

    let mut bmp = original.clone();
    apply_gamma(&mut bmp, 2.0);
    let mean_after = pixel_mean(&bmp);

    assert!(
        mean_after > mean_before,
        "gamma=2.0 mean after {mean_after} <= before {mean_before}"
    );
}

#[test]
fn gamma_lt_one_darkens_ramp_mean() {
    let original = make_gray_ramp(64, 64);
    let mean_before = pixel_mean(&original);

    let mut bmp = original.clone();
    apply_gamma(&mut bmp, 0.5);
    let mean_after = pixel_mean(&bmp);

    assert!(
        mean_after < mean_before,
        "gamma=0.5 mean after {mean_after} >= before {mean_before}"
    );
}

#[test]
fn sharpen_increases_high_freq_energy_on_edge() {
    // 8x8 **低对比度**边缘图（左半 100，右半 156）→ sharpen 应让边缘对比扩大
    // 注：用 0/255 锐边时 sharpen 无效——边缘已饱和，u8 上限阻止进一步"陡峭化"。
    // C 行 3571-3587 sharpen 的本质是放大局部高频，对中等对比度边缘最明显。
    let mut bmp = Bitmap::new(8, 8, 72.0, PixelFormat::Gray8).unwrap();
    for y in 0..8 {
        for x in 0..8 {
            let v = if x < 4 { 100u8 } else { 156 };
            bmp.pixel_mut(x, y).unwrap()[0] = v;
        }
    }
    let energy_before = high_freq_energy(&bmp);
    apply_sharpen(&mut bmp);
    let energy_after = high_freq_energy(&bmp);
    assert!(
        energy_after > energy_before,
        "sharpen edge energy after {energy_after} <= before {energy_before}"
    );
}

#[test]
fn sharpen_preserves_dimensions_and_format() {
    let cases = [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8];
    for fmt in cases {
        let mut bmp = Bitmap::new(10, 7, 96.0, fmt).unwrap();
        bmp.fill_byte(64);
        let (w, h, f) = (bmp.width, bmp.height, bmp.format);
        let total = bmp.pixels.len();
        apply_sharpen(&mut bmp);
        assert_eq!(bmp.width, w);
        assert_eq!(bmp.height, h);
        assert_eq!(bmp.format, f);
        assert_eq!(bmp.pixels.len(), total);
    }
}

#[test]
fn apply_filter_3x3_identity_kernel_preserves_image() {
    // 单位卷积核 filter[1][1]=1.0，其余 0 → 输出等于输入（边界处 weight=1.0 仍恒等）
    let mut bmp = make_gray_ramp(16, 8);
    let original = bmp.clone();
    let identity = [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]];
    apply_filter_3x3(&mut bmp, &identity);
    // 由于边界 weight = 1.0（只剩中心点），所有像素 = 原 src/1.0 = 原值
    assert_eq!(
        bmp.pixels, original.pixels,
        "identity filter should be no-op"
    );
}

#[test]
fn apply_filter_3x3_box_blur_averages() {
    // 3x3 box blur filter (全 1/9) → 中心点应为 9 邻域均值
    let mut bmp = Bitmap::new(5, 5, 72.0, PixelFormat::Gray8).unwrap();
    // 中心 (2,2) = 90, 其他全 0
    bmp.pixel_mut(2, 2).unwrap()[0] = 90;
    let box_filter = [[1.0 / 9.0; 3]; 3];
    apply_filter_3x3(&mut bmp, &box_filter);
    // 中心 (2,2) 9 邻居中只有自己 = 90 → 均值 90/9 = 10
    assert_eq!(bmp.gray_at(2, 2), Some(10));
    // 周边 (1,1)/(2,1)/(3,1)/(1,2)/(3,2)/(1,3)/(2,3)/(3,3) 各有 90 在邻域 → 均值 10
    assert_eq!(bmp.gray_at(1, 1), Some(10));
    assert_eq!(bmp.gray_at(2, 1), Some(10));
}

// --------------------------------------------------------------------------
// Fixture smoke 测试 - 12 个 fixture 首页 PNG
// --------------------------------------------------------------------------

#[test]
fn fixture_pngs_contrast_smoke() {
    let fixtures = [
        "single-column",
        "two-column",
        "three-column",
        "scanned",
        "skewed-scan",
        "mixed-text-image",
        "complex-layout",
        "chinese",
        "formula",
        "cover",
        "encrypted",
        // blank-page 跳过：k2pdfopt 对 0 页 PDF 输出 0 页 PNG
    ];
    let mut tested = 0;
    for name in fixtures {
        let Some(bmp) = load_first_page_png(name) else {
            continue;
        };
        let (w, h, fmt) = (bmp.width, bmp.height, bmp.format);
        let total_before = bmp.pixels.len();

        let mut copy = bmp.clone();
        apply_contrast(&mut copy, 1.2);
        assert_eq!(copy.width, w, "{name}: width changed");
        assert_eq!(copy.height, h, "{name}: height changed");
        assert_eq!(copy.format, fmt, "{name}: format changed");
        assert_eq!(
            copy.pixels.len(),
            total_before,
            "{name}: total bytes changed"
        );
        // contrast 1.2 应让方差变大或不变（极端均匀图像可能不变）
        let stddev_before = pixel_stddev(&bmp);
        let stddev_after = pixel_stddev(&copy);
        // 允许等于（均匀图像）；不允许显著变小
        assert!(
            stddev_after + 0.5 >= stddev_before,
            "{name}: contrast 1.2 stddev decreased: {stddev_before} -> {stddev_after}"
        );
        tested += 1;
    }
    assert!(tested >= 8, "expected >=8 fixtures, got {tested}");
}

#[test]
fn fixture_pngs_gamma_smoke() {
    let fixtures = ["single-column", "two-column", "scanned", "chinese", "cover"];
    let mut tested = 0;
    for name in fixtures {
        let Some(bmp) = load_first_page_png(name) else {
            continue;
        };
        let mean_before = pixel_mean(&bmp);

        let mut bright = bmp.clone();
        apply_gamma(&mut bright, 2.0);
        let mean_bright = pixel_mean(&bright);
        assert!(
            mean_bright >= mean_before - 1.0,
            "{name}: gamma=2 mean dropped {mean_before} -> {mean_bright}"
        );

        let mut dark = bmp.clone();
        apply_gamma(&mut dark, 0.5);
        let mean_dark = pixel_mean(&dark);
        assert!(
            mean_dark <= mean_before + 1.0,
            "{name}: gamma=0.5 mean rose {mean_before} -> {mean_dark}"
        );

        // gamma=1 必须恒等
        let mut id = bmp.clone();
        apply_gamma(&mut id, 1.0);
        assert_eq!(id.pixels, bmp.pixels, "{name}: gamma=1.0 should be no-op");

        tested += 1;
    }
    assert!(tested >= 3, "expected >=3 fixtures, got {tested}");
}

#[test]
fn fixture_pngs_sharpen_smoke() {
    let fixtures = [
        "single-column",
        "two-column",
        "scanned",
        "chinese",
        "formula",
    ];
    let mut tested = 0;
    for name in fixtures {
        let Some(bmp) = load_first_page_png(name) else {
            continue;
        };
        let (w, h, fmt) = (bmp.width, bmp.height, bmp.format);
        let total_before = bmp.pixels.len();

        let mut copy = bmp.clone();
        apply_sharpen(&mut copy);
        assert_eq!(copy.width, w, "{name}: width changed");
        assert_eq!(copy.height, h, "{name}: height changed");
        assert_eq!(copy.format, fmt, "{name}: format changed");
        assert_eq!(
            copy.pixels.len(),
            total_before,
            "{name}: total bytes changed"
        );

        // 大多数真实文档 sharpen 后高频能量应增大（除非极均匀）
        let energy_before = high_freq_energy(&bmp);
        let energy_after = high_freq_energy(&copy);
        // 容差：≥ 95% 原值即可（个别极均匀页面允许少量下降）
        assert!(
            energy_after as f64 >= energy_before as f64 * 0.95,
            "{name}: sharpen energy {energy_before} -> {energy_after}"
        );
        tested += 1;
    }
    assert!(tested >= 3, "expected >=3 fixtures, got {tested}");
}

#[test]
fn fixture_chain_contrast_gamma_sharpen_no_panic() {
    // 串联三滤镜应不 panic + 输出仍是合法 bitmap
    let Some(mut bmp) = load_first_page_png("single-column") else {
        return; // skip if fixture missing
    };
    let (w, h, fmt) = (bmp.width, bmp.height, bmp.format);
    apply_contrast(&mut bmp, 1.3);
    apply_gamma(&mut bmp, 1.8);
    apply_sharpen(&mut bmp);
    assert_eq!(bmp.width, w);
    assert_eq!(bmp.height, h);
    assert_eq!(bmp.format, fmt);
    // 所有像素仍在 [0, 255] u8 范围（隐式保证）
}
