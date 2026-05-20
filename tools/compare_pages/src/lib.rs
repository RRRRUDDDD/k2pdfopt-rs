//! compare_pages - PDF page comparison via SSIM + pixel diff.
//!
//! Step 5.7 (M4.5) - 交叉验证基础设施。
//! 用 mutool 把两份 PDF 渲染为 PNG 序列，对每页跑 SSIM（11x11 高斯窗口，wikipedia 公式）
//! + 像素级差异统计，输出 JSON + HTML 报告。
//!
//! 详见 rust-rewrite-execution-plan.md Step 5.7 / rust-rewrite-plan.md v2.1 §11。
//!
//! # 当前阶段约束
//!
//! - M5 (Step 7.x) 之前 Rust 端尚未产出 PDF 输出，本工具主要用作：
//!   1. 自比 sanity 测试（同 PDF vs 自身 → SSIM=1.0 验证算法正确）
//!   2. C baseline ↔ C baseline 一致性回归（确保 fixture 重新生成可复现）
//! - M5 起切换为 Rust output vs C baseline 的真实回归对照

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use image::GrayImage;
use serde::{Deserialize, Serialize};

/// SSIM 高斯窗口大小（wikipedia 标准，11×11）。
pub const SSIM_WINDOW: usize = 11;
/// SSIM 高斯标准差（wikipedia 标准，σ=1.5）。
pub const SSIM_SIGMA: f64 = 1.5;
/// K1 常数 (默认 0.01)。
pub const SSIM_K1: f64 = 0.01;
/// K2 常数 (默认 0.03)。
pub const SSIM_K2: f64 = 0.03;
/// 8-bit 灰度动态范围。
pub const SSIM_L: f64 = 255.0;

/// 默认对比 DPI（mutool 渲染 PDF → PNG 时用）。
pub const DEFAULT_COMPARE_DPI: u32 = 150;

/// 像素差异统计（基于 8-bit 灰度逐像素 abs(a-b)）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DiffStats {
    /// 平均绝对差（0~255）。
    pub mean_abs: f64,
    /// 最大绝对差（0~255）。
    pub max_abs: u8,
    /// 差异 > 10 的像素占比（0~1）。
    pub pct_gt10: f64,
    /// 差异 > 30 的像素占比（0~1）。
    pub pct_gt30: f64,
    /// 比对的像素数。
    pub n_pixels: usize,
}

/// 单页对比结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageComparison {
    pub page_index: usize,
    pub width_a: u32,
    pub height_a: u32,
    pub width_b: u32,
    pub height_b: u32,
    /// 若尺寸不一致，会先裁/缩到公共最小尺寸再比对（resize 方式见 [`resize_strategy`]）。
    pub size_mismatch: bool,
    pub ssim_mean: f64,
    pub diff: DiffStats,
}

/// 跨页汇总报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonReport {
    pub path_a: PathBuf,
    pub path_b: PathBuf,
    pub dpi: u32,
    pub pages_a: usize,
    pub pages_b: usize,
    pub pages_compared: usize,
    /// 全局平均 SSIM（各页 ssim_mean 的算术平均；pages_compared=0 时为 0.0）。
    pub overall_ssim: f64,
    pub pages: Vec<PageComparison>,
}

/// 比对选项。
#[derive(Debug, Clone)]
pub struct CompareOptions {
    pub dpi: u32,
    pub mutool_bin: PathBuf,
    /// PDF 密码（可选）；用于加密 fixture。
    pub password: Option<String>,
    /// 仅比对前 N 页（None = 全部）。
    pub max_pages: Option<usize>,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            dpi: DEFAULT_COMPARE_DPI,
            mutool_bin: PathBuf::from("mutool"),
            password: None,
            max_pages: None,
        }
    }
}

/// 比对策略说明（含在 HTML 报告中）。
pub fn resize_strategy() -> &'static str {
    "若两图尺寸不一致，裁到公共最小宽高（左上角对齐）后再比对，并在报告里置 size_mismatch=true"
}

// ============================================================================
// SSIM 实现 - wikipedia 公式 + 11x11 高斯窗口
// ============================================================================

/// 构造 11×11 高斯窗口（σ=1.5），归一化后总和为 1.0。
pub fn gaussian_window_11x11() -> [[f64; SSIM_WINDOW]; SSIM_WINDOW] {
    let mut w = [[0f64; SSIM_WINDOW]; SSIM_WINDOW];
    let center = (SSIM_WINDOW as f64 - 1.0) / 2.0;
    let two_sigma_sq = 2.0 * SSIM_SIGMA * SSIM_SIGMA;
    let mut sum = 0.0;
    for (i, row) in w.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            let dy = i as f64 - center;
            let dx = j as f64 - center;
            let g = (-(dx * dx + dy * dy) / two_sigma_sq).exp();
            *cell = g;
            sum += g;
        }
    }
    // 归一化
    for row in &mut w {
        for cell in row.iter_mut() {
            *cell /= sum;
        }
    }
    w
}

/// 计算两张灰度图的 mean SSIM。
///
/// 假设 a 与 b 同尺寸（若不同请预先 crop）；尺寸不足 11x11 时返回 1.0（认为完全一致，
/// 避免极小区域统计无意义）。
pub fn ssim_mean(a: &GrayImage, b: &GrayImage) -> Result<f64> {
    if a.dimensions() != b.dimensions() {
        bail!(
            "ssim_mean: 尺寸不一致 {:?} vs {:?}",
            a.dimensions(),
            b.dimensions()
        );
    }
    let (w, h) = a.dimensions();
    if w < SSIM_WINDOW as u32 || h < SSIM_WINDOW as u32 {
        return Ok(1.0);
    }
    let window = gaussian_window_11x11();
    let c1 = (SSIM_K1 * SSIM_L).powi(2);
    let c2 = (SSIM_K2 * SSIM_L).powi(2);

    let a_buf = a.as_raw();
    let b_buf = b.as_raw();
    let stride = w as usize;
    let half = SSIM_WINDOW / 2;
    let mut ssim_sum = 0.0;
    let mut count = 0u64;

    for y in half..(h as usize - half) {
        for x in half..(w as usize - half) {
            let mut mu_x = 0.0;
            let mut mu_y = 0.0;
            for ky in 0..SSIM_WINDOW {
                for kx in 0..SSIM_WINDOW {
                    let pa = a_buf[(y + ky - half) * stride + (x + kx - half)] as f64;
                    let pb = b_buf[(y + ky - half) * stride + (x + kx - half)] as f64;
                    let wgt = window[ky][kx];
                    mu_x += wgt * pa;
                    mu_y += wgt * pb;
                }
            }

            let mut sigma_x = 0.0;
            let mut sigma_y = 0.0;
            let mut sigma_xy = 0.0;
            for ky in 0..SSIM_WINDOW {
                for kx in 0..SSIM_WINDOW {
                    let pa = a_buf[(y + ky - half) * stride + (x + kx - half)] as f64;
                    let pb = b_buf[(y + ky - half) * stride + (x + kx - half)] as f64;
                    let wgt = window[ky][kx];
                    let dx = pa - mu_x;
                    let dy = pb - mu_y;
                    sigma_x += wgt * dx * dx;
                    sigma_y += wgt * dy * dy;
                    sigma_xy += wgt * dx * dy;
                }
            }

            let numerator = (2.0 * mu_x * mu_y + c1) * (2.0 * sigma_xy + c2);
            let denominator = (mu_x * mu_x + mu_y * mu_y + c1) * (sigma_x + sigma_y + c2);
            ssim_sum += numerator / denominator;
            count += 1;
        }
    }

    if count == 0 {
        Ok(1.0)
    } else {
        Ok(ssim_sum / count as f64)
    }
}

// ============================================================================
// 像素差异统计
// ============================================================================

/// 计算两张同尺寸灰度图的逐像素差异统计。
pub fn pixel_diff(a: &GrayImage, b: &GrayImage) -> Result<DiffStats> {
    if a.dimensions() != b.dimensions() {
        bail!(
            "pixel_diff: 尺寸不一致 {:?} vs {:?}",
            a.dimensions(),
            b.dimensions()
        );
    }
    let a_buf = a.as_raw();
    let b_buf = b.as_raw();
    let n = a_buf.len();
    if n == 0 {
        return Ok(DiffStats {
            mean_abs: 0.0,
            max_abs: 0,
            pct_gt10: 0.0,
            pct_gt30: 0.0,
            n_pixels: 0,
        });
    }
    let mut sum_abs: u64 = 0;
    let mut max_abs: u8 = 0;
    let mut gt10: u64 = 0;
    let mut gt30: u64 = 0;
    for (pa, pb) in a_buf.iter().zip(b_buf.iter()) {
        let d = pa.abs_diff(*pb);
        sum_abs += d as u64;
        if d > max_abs {
            max_abs = d;
        }
        if d > 10 {
            gt10 += 1;
        }
        if d > 30 {
            gt30 += 1;
        }
    }
    Ok(DiffStats {
        mean_abs: sum_abs as f64 / n as f64,
        max_abs,
        pct_gt10: gt10 as f64 / n as f64,
        pct_gt30: gt30 as f64 / n as f64,
        n_pixels: n,
    })
}

// ============================================================================
// 图像加载 + 尺寸对齐
// ============================================================================

/// 加载 PNG 为 8-bit 灰度。
pub fn load_png_gray(path: &Path) -> Result<GrayImage> {
    let img = image::open(path).with_context(|| format!("打开 PNG 失败: {}", path.display()))?;
    Ok(img.into_luma8())
}

/// 把两张图裁到公共最小尺寸（左上角对齐）。
pub fn crop_to_common(a: GrayImage, b: GrayImage) -> (GrayImage, GrayImage, bool) {
    let (wa, ha) = a.dimensions();
    let (wb, hb) = b.dimensions();
    if wa == wb && ha == hb {
        return (a, b, false);
    }
    let w = wa.min(wb);
    let h = ha.min(hb);
    let a_crop = image::imageops::crop_imm(&a, 0, 0, w, h).to_image();
    let b_crop = image::imageops::crop_imm(&b, 0, 0, w, h).to_image();
    (a_crop, b_crop, true)
}

// ============================================================================
// mutool 渲染 PDF → PNG 序列
// ============================================================================

/// 调用 mutool draw 将 PDF 渲染到 out_dir 下 page-NNNN.png（1-based）。
/// 返回生成的 PNG 路径列表（按页号升序）。
pub fn render_pdf_to_pngs(
    pdf: &Path,
    out_dir: &Path,
    opts: &CompareOptions,
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("创建输出目录失败: {}", out_dir.display()))?;
    // 先清空已有 PNG（避免上次残留干扰）
    for entry in std::fs::read_dir(out_dir)? {
        let p = entry?.path();
        if p.extension().and_then(|s| s.to_str()) == Some("png") {
            let _ = std::fs::remove_file(&p);
        }
    }
    let pattern = out_dir.join("page-%04d.png");

    let mut cmd = Command::new(&opts.mutool_bin);
    cmd.arg("draw")
        .arg("-r")
        .arg(opts.dpi.to_string())
        .arg("-c")
        .arg("gray")
        .arg("-o")
        .arg(&pattern);
    if let Some(pw) = &opts.password {
        cmd.arg("-p").arg(pw);
    }
    cmd.arg(pdf);

    let output = cmd
        .output()
        .with_context(|| format!("启动 mutool 失败: {}", opts.mutool_bin.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "mutool draw 失败 (exit={:?}): {}",
            output.status.code(),
            stderr.trim()
        );
    }

    let mut pngs: Vec<PathBuf> = std::fs::read_dir(out_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("png"))
        .collect();
    pngs.sort();
    if pngs.is_empty() {
        bail!("mutool draw 未生成 PNG: {}", pdf.display());
    }
    Ok(pngs)
}

// ============================================================================
// 端到端比对
// ============================================================================

/// 比对两份 PDF。先用 mutool 渲染为 PNG，逐页计算 SSIM + diff。
pub fn compare_pdfs(
    pdf_a: &Path,
    pdf_b: &Path,
    work_dir: &Path,
    opts: &CompareOptions,
) -> Result<ComparisonReport> {
    let dir_a = work_dir.join("a");
    let dir_b = work_dir.join("b");
    let pages_a = render_pdf_to_pngs(pdf_a, &dir_a, opts)?;
    let pages_b = render_pdf_to_pngs(pdf_b, &dir_b, opts)?;

    let mut max_pairs = pages_a.len().min(pages_b.len());
    if let Some(cap) = opts.max_pages {
        max_pairs = max_pairs.min(cap);
    }

    let mut pages = Vec::with_capacity(max_pairs);
    let mut ssim_sum = 0.0;
    for i in 0..max_pairs {
        let pc = compare_png_pair(i, &pages_a[i], &pages_b[i])?;
        ssim_sum += pc.ssim_mean;
        pages.push(pc);
    }
    let overall_ssim = if pages.is_empty() {
        0.0
    } else {
        ssim_sum / pages.len() as f64
    };

    Ok(ComparisonReport {
        path_a: pdf_a.to_path_buf(),
        path_b: pdf_b.to_path_buf(),
        dpi: opts.dpi,
        pages_a: pages_a.len(),
        pages_b: pages_b.len(),
        pages_compared: pages.len(),
        overall_ssim,
        pages,
    })
}

/// 直接对一对 PNG 做比对（不走 PDF 渲染），用于 self-test 与单元测试。
pub fn compare_png_pair(page_index: usize, a: &Path, b: &Path) -> Result<PageComparison> {
    let img_a = load_png_gray(a)?;
    let img_b = load_png_gray(b)?;
    let (wa, ha) = img_a.dimensions();
    let (wb, hb) = img_b.dimensions();
    let (img_a, img_b, mismatch) = crop_to_common(img_a, img_b);
    let ssim = ssim_mean(&img_a, &img_b)?;
    let diff = pixel_diff(&img_a, &img_b)?;
    Ok(PageComparison {
        page_index,
        width_a: wa,
        height_a: ha,
        width_b: wb,
        height_b: hb,
        size_mismatch: mismatch,
        ssim_mean: ssim,
        diff,
    })
}

// ============================================================================
// 报告输出
// ============================================================================

/// 写 JSON 报告（pretty）。
pub fn write_json_report(report: &ComparisonReport, out: &Path) -> Result<()> {
    let s = serde_json::to_string_pretty(report)?;
    std::fs::write(out, s).with_context(|| format!("写 JSON 报告失败: {}", out.display()))?;
    Ok(())
}

/// 写 HTML 报告（含侧边栏 + 缩略图引用）。
pub fn write_html_report(report: &ComparisonReport, out: &Path) -> Result<()> {
    let mut html = String::new();
    html.push_str("<!doctype html>\n<html><head><meta charset=\"utf-8\">");
    html.push_str("<title>compare_pages report</title>");
    html.push_str("<style>");
    html.push_str("body{font-family:sans-serif;margin:0;display:flex}");
    html.push_str("nav{width:240px;background:#f5f5f5;padding:1em;height:100vh;overflow:auto;box-sizing:border-box}");
    html.push_str("main{flex:1;padding:1em;overflow:auto}");
    html.push_str("table{border-collapse:collapse;width:100%}");
    html.push_str("th,td{border:1px solid #ccc;padding:.3em .6em;text-align:right}");
    html.push_str("th:first-child,td:first-child{text-align:left}");
    html.push_str(".ok{color:#080}.warn{color:#a60}.bad{color:#c00}");
    html.push_str("</style></head><body>");

    // sidebar
    html.push_str("<nav><h2>Pages</h2><ol>");
    for p in &report.pages {
        let cls = ssim_class(p.ssim_mean);
        html.push_str(&format!(
            "<li><a href=\"#p{0}\" class=\"{1}\">page {0}: SSIM={2:.4}</a></li>",
            p.page_index, cls, p.ssim_mean
        ));
    }
    html.push_str("</ol></nav>");

    // main
    html.push_str("<main>");
    html.push_str("<h1>compare_pages report</h1>");
    html.push_str(&format!(
        "<p>A: <code>{}</code><br>B: <code>{}</code><br>DPI: {} | pages A/B: {}/{} | compared: {} | overall SSIM: <b>{:.4}</b></p>",
        html_escape(&report.path_a.display().to_string()),
        html_escape(&report.path_b.display().to_string()),
        report.dpi,
        report.pages_a,
        report.pages_b,
        report.pages_compared,
        report.overall_ssim,
    ));
    html.push_str(&format!("<p><small>{}</small></p>", resize_strategy()));

    html.push_str("<table><thead><tr><th>page</th><th>SSIM</th><th>mean|diff|</th><th>max|diff|</th><th>%&gt;10</th><th>%&gt;30</th><th>w×h A</th><th>w×h B</th><th>mismatch</th></tr></thead><tbody>");
    for p in &report.pages {
        let cls = ssim_class(p.ssim_mean);
        html.push_str(&format!(
            "<tr id=\"p{}\"><td class=\"{}\">{}</td><td class=\"{}\">{:.4}</td><td>{:.2}</td><td>{}</td><td>{:.2}%</td><td>{:.2}%</td><td>{}×{}</td><td>{}×{}</td><td>{}</td></tr>",
            p.page_index,
            cls,
            p.page_index,
            cls,
            p.ssim_mean,
            p.diff.mean_abs,
            p.diff.max_abs,
            p.diff.pct_gt10 * 100.0,
            p.diff.pct_gt30 * 100.0,
            p.width_a,
            p.height_a,
            p.width_b,
            p.height_b,
            if p.size_mismatch { "YES" } else { "no" },
        ));
    }
    html.push_str("</tbody></table>");
    html.push_str("</main></body></html>");

    std::fs::write(out, html).with_context(|| format!("写 HTML 报告失败: {}", out.display()))?;
    Ok(())
}

fn ssim_class(s: f64) -> &'static str {
    if s >= 0.95 {
        "ok"
    } else if s >= 0.85 {
        "warn"
    } else {
        "bad"
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ============================================================================
// 工具：把工作目录用作临时空间（caller 控制清理）
// ============================================================================

/// 返回 mutool 可用性（用于 self-test）。
pub fn ensure_mutool(opts: &CompareOptions) -> Result<String> {
    let out = Command::new(&opts.mutool_bin)
        .arg("-v")
        .output()
        .with_context(|| format!("启动 mutool 失败: {}", opts.mutool_bin.display()))?;
    if !out.status.success() {
        return Err(anyhow!("mutool -v 退出码非零: {:?}", out.status.code()));
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(if stderr.is_empty() { stdout } else { stderr })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use image::{GrayImage, Luma};

    fn make_gray(w: u32, h: u32, fill: u8) -> GrayImage {
        GrayImage::from_pixel(w, h, Luma([fill]))
    }

    #[test]
    fn gaussian_window_is_normalized() {
        let w = gaussian_window_11x11();
        let total: f64 = w.iter().flatten().sum();
        assert!((total - 1.0).abs() < 1e-9);
        let center = w[5][5];
        let corner = w[0][0];
        assert!(center > corner);
    }

    #[test]
    fn ssim_identical_images_is_one() {
        let a = make_gray(64, 64, 128);
        let b = make_gray(64, 64, 128);
        let s = ssim_mean(&a, &b).unwrap();
        assert!((s - 1.0).abs() < 1e-9, "expected SSIM=1.0 got {s}");
    }

    #[test]
    fn ssim_completely_different_is_low() {
        let a = make_gray(64, 64, 0);
        let b = make_gray(64, 64, 255);
        let s = ssim_mean(&a, &b).unwrap();
        // 极端差异下 SSIM 应该非常低
        assert!(s < 0.05, "expected SSIM<0.05 got {s}");
    }

    #[test]
    fn ssim_size_lt_window_returns_one() {
        let a = make_gray(8, 8, 100);
        let b = make_gray(8, 8, 200);
        let s = ssim_mean(&a, &b).unwrap();
        // 小于窗口尺寸返回 1.0 (无有效统计点)
        assert!((s - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ssim_size_mismatch_errors() {
        let a = make_gray(64, 32, 100);
        let b = make_gray(32, 64, 100);
        assert!(ssim_mean(&a, &b).is_err());
    }

    #[test]
    fn ssim_slightly_different_is_high() {
        let mut a = make_gray(64, 64, 128);
        let mut b = make_gray(64, 64, 128);
        // 加微弱 pattern 让 σx²/σy² 非零，避免极端常量 corner case
        for y in 0..64 {
            for x in 0..64 {
                a.put_pixel(x, y, Luma([(120 + ((x + y) % 8) * 2) as u8]));
                b.put_pixel(x, y, Luma([(122 + ((x + y) % 8) * 2) as u8]));
            }
        }
        let s = ssim_mean(&a, &b).unwrap();
        assert!(s > 0.95, "expected SSIM>0.95 got {s}");
        assert!(s < 0.9999);
    }

    #[test]
    fn pixel_diff_identical_zero() {
        let a = make_gray(32, 32, 100);
        let b = make_gray(32, 32, 100);
        let d = pixel_diff(&a, &b).unwrap();
        assert_eq!(d.mean_abs, 0.0);
        assert_eq!(d.max_abs, 0);
        assert_eq!(d.pct_gt10, 0.0);
        assert_eq!(d.pct_gt30, 0.0);
        assert_eq!(d.n_pixels, 32 * 32);
    }

    #[test]
    fn pixel_diff_constant_offset() {
        let a = make_gray(10, 10, 50);
        let b = make_gray(10, 10, 100);
        let d = pixel_diff(&a, &b).unwrap();
        assert_eq!(d.mean_abs, 50.0);
        assert_eq!(d.max_abs, 50);
        assert_eq!(d.pct_gt10, 1.0);
        assert_eq!(d.pct_gt30, 1.0);
        assert_eq!(d.n_pixels, 100);
    }

    #[test]
    fn pixel_diff_mixed() {
        let mut a = make_gray(2, 2, 0);
        let mut b = make_gray(2, 2, 0);
        a.put_pixel(0, 0, Luma([0]));
        b.put_pixel(0, 0, Luma([5])); // d=5, no thresh
        a.put_pixel(1, 0, Luma([0]));
        b.put_pixel(1, 0, Luma([15])); // d=15 > 10
        a.put_pixel(0, 1, Luma([0]));
        b.put_pixel(0, 1, Luma([40])); // d=40 > 30
        a.put_pixel(1, 1, Luma([0]));
        b.put_pixel(1, 1, Luma([0])); // d=0
        let d = pixel_diff(&a, &b).unwrap();
        assert_eq!(d.n_pixels, 4);
        assert_eq!(d.max_abs, 40);
        assert!((d.mean_abs - 15.0).abs() < 1e-9);
        assert!((d.pct_gt10 - 0.5).abs() < 1e-9);
        assert!((d.pct_gt30 - 0.25).abs() < 1e-9);
    }

    #[test]
    fn crop_to_common_aligns_topleft() {
        let a = make_gray(100, 80, 50);
        let b = make_gray(80, 100, 50);
        let (a2, b2, mismatch) = crop_to_common(a, b);
        assert!(mismatch);
        assert_eq!(a2.dimensions(), (80, 80));
        assert_eq!(b2.dimensions(), (80, 80));
    }

    #[test]
    fn crop_to_common_noop_when_equal() {
        let a = make_gray(64, 64, 50);
        let b = make_gray(64, 64, 50);
        let (a2, b2, mismatch) = crop_to_common(a, b);
        assert!(!mismatch);
        assert_eq!(a2.dimensions(), (64, 64));
        assert_eq!(b2.dimensions(), (64, 64));
    }

    #[test]
    fn ssim_class_thresholds() {
        assert_eq!(ssim_class(1.0), "ok");
        assert_eq!(ssim_class(0.95), "ok");
        assert_eq!(ssim_class(0.94), "warn");
        assert_eq!(ssim_class(0.85), "warn");
        assert_eq!(ssim_class(0.84), "bad");
    }

    #[test]
    fn html_escape_basics() {
        assert_eq!(html_escape("a&b<c>"), "a&amp;b&lt;c&gt;");
    }

    #[test]
    fn report_roundtrip_json() {
        let report = ComparisonReport {
            path_a: PathBuf::from("a.pdf"),
            path_b: PathBuf::from("b.pdf"),
            dpi: 150,
            pages_a: 1,
            pages_b: 1,
            pages_compared: 1,
            overall_ssim: 0.987,
            pages: vec![PageComparison {
                page_index: 0,
                width_a: 100,
                height_a: 200,
                width_b: 100,
                height_b: 200,
                size_mismatch: false,
                ssim_mean: 0.987,
                diff: DiffStats {
                    mean_abs: 1.2,
                    max_abs: 30,
                    pct_gt10: 0.01,
                    pct_gt30: 0.0,
                    n_pixels: 20000,
                },
            }],
        };
        let s = serde_json::to_string(&report).unwrap();
        let r: ComparisonReport = serde_json::from_str(&s).unwrap();
        assert_eq!(r.pages_compared, 1);
        assert!((r.overall_ssim - 0.987).abs() < 1e-9);
    }
}
