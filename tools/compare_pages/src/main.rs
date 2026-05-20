//! compare_pages CLI - 比对两份 PDF，输出 SSIM + diff 报告 (JSON + HTML)。
//!
//! 用法：
//!   compare_pages --c <c-output.pdf> --rs <rust-output.pdf> [--dpi 150]
//!                 [--password <pw>] [--max-pages N]
//!                 [--mutool <path>] [--work-dir <dir>]
//!                 [--json <out.json>] [--html <out.html>]
//!                 [--min-ssim 0.95]
//!
//! 退出码：0 = 全部页 SSIM >= --min-ssim；2 = 至少有一页低于阈值；其他 = 错误。

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use compare_pages::{
    compare_pdfs, ensure_mutool, write_html_report, write_json_report, CompareOptions,
    ComparisonReport,
};

#[derive(Debug, Parser)]
#[command(
    name = "compare_pages",
    about = "比对两份 PDF 的页面 (SSIM + pixel diff)，输出 JSON + HTML 报告",
    long_about = None,
)]
struct Cli {
    /// C 版（参考）PDF 路径
    #[arg(long = "c", value_name = "PDF")]
    c: PathBuf,
    /// Rust 版（待测）PDF 路径
    #[arg(long = "rs", value_name = "PDF")]
    rs: PathBuf,
    /// 渲染 DPI（mutool draw -r）
    #[arg(long, default_value_t = compare_pages::DEFAULT_COMPARE_DPI)]
    dpi: u32,
    /// PDF 密码（加密 fixture）
    #[arg(long)]
    password: Option<String>,
    /// 仅比对前 N 页
    #[arg(long)]
    max_pages: Option<usize>,
    /// mutool 二进制路径
    #[arg(long, default_value = "mutool")]
    mutool: PathBuf,
    /// 工作目录（PDF→PNG 中间产物）
    #[arg(long)]
    work_dir: Option<PathBuf>,
    /// 输出 JSON 报告路径
    #[arg(long)]
    json: Option<PathBuf>,
    /// 输出 HTML 报告路径
    #[arg(long)]
    html: Option<PathBuf>,
    /// 退出码门槛：任一页 SSIM < 此值 → exit 2
    #[arg(long, default_value_t = 0.95)]
    min_ssim: f64,
    /// 仅检查 mutool 可用性后退出（self-test）
    #[arg(long)]
    check_mutool: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("compare_pages 失败: {e:#}");
            ExitCode::from(10)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let opts = CompareOptions {
        dpi: cli.dpi,
        mutool_bin: cli.mutool.clone(),
        password: cli.password.clone(),
        max_pages: cli.max_pages,
    };

    if cli.check_mutool {
        let v = ensure_mutool(&opts)?;
        println!("mutool ok: {v}");
        return Ok(ExitCode::SUCCESS);
    }

    let work_dir = match cli.work_dir.clone() {
        Some(p) => p,
        None => std::env::temp_dir().join(format!("compare_pages-{}", std::process::id())),
    };
    std::fs::create_dir_all(&work_dir)
        .with_context(|| format!("创建工作目录失败: {}", work_dir.display()))?;

    let report = compare_pdfs(&cli.c, &cli.rs, &work_dir, &opts)?;
    print_summary(&report);

    if let Some(json) = &cli.json {
        write_json_report(&report, json)?;
    }
    if let Some(html) = &cli.html {
        write_html_report(&report, html)?;
    }

    let any_fail = report.pages.iter().any(|p| p.ssim_mean < cli.min_ssim);
    if any_fail {
        Ok(ExitCode::from(2))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn print_summary(report: &ComparisonReport) {
    println!(
        "compare_pages: pages A={} B={} compared={} overall_SSIM={:.4}",
        report.pages_a, report.pages_b, report.pages_compared, report.overall_ssim
    );
    for p in &report.pages {
        println!(
            "  page {:>3}: SSIM={:.4} mean|d|={:.2} max|d|={} >10={:.2}% >30={:.2}% ({}x{} vs {}x{}){}",
            p.page_index,
            p.ssim_mean,
            p.diff.mean_abs,
            p.diff.max_abs,
            p.diff.pct_gt10 * 100.0,
            p.diff.pct_gt30 * 100.0,
            p.width_a,
            p.height_a,
            p.width_b,
            p.height_b,
            if p.size_mismatch { " [MISMATCH]" } else { "" },
        );
    }
}
