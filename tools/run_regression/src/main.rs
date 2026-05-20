//! run_regression - 批量回归运行器。
//!
//! Step 5.7 (M4.5) - 遍历 tests/golden/* 下的 12 个 fixture，按以下模式比对：
//!
//! 1. `--mode self`（默认，M5 之前）：把 c-output.pdf 与自身比对，
//!    主要用作 SSIM 工具链的 sanity check（期望 overall_SSIM ≈ 1.0）。
//! 2. `--mode rust-vs-c`（M5 起）：用 Rust k2pdfopt 重新转换 fixture，
//!    与 c-output.pdf 做真实回归对照。本步骤仅留接口，实现 stub。
//!
//! 跨平台命令示例（PowerShell）:
//!   cargo run --release --bin run_regression -- --all
//!   cargo run --release --bin run_regression -- --fixture single-column
//!   cargo run --release --bin run_regression -- --all --mode rust-vs-c
//!
//! 输出：
//!   tests/golden/_regression/<timestamp>/summary.json
//!   tests/golden/_regression/<timestamp>/<fixture>.json
//!   tests/golden/_regression/<timestamp>/<fixture>.html
//!   tests/golden/_regression/<timestamp>/index.html

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use compare_pages::{
    compare_pdfs, write_html_report, write_json_report, CompareOptions, ComparisonReport,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Mode {
    /// c-output.pdf vs 自身（SSIM 工具链 sanity）
    #[value(name = "self")]
    SelfCompare,
    /// Rust 端 k2pdfopt 输出 vs c-output.pdf（M5 起启用）
    #[value(name = "rust-vs-c")]
    RustVsC,
}

#[derive(Debug, Parser)]
#[command(
    name = "run_regression",
    about = "批量跑 12 fixture 的 SSIM + diff 回归报告"
)]
struct Cli {
    /// 跑所有 fixture
    #[arg(long)]
    all: bool,
    /// 仅跑指定 fixture（可重复）
    #[arg(long = "fixture", value_name = "NAME")]
    fixtures: Vec<String>,
    /// 比对模式
    #[arg(long, value_enum, default_value_t = Mode::SelfCompare)]
    mode: Mode,
    /// fixture 根目录（包含 single-column/、two-column/ ...）
    #[arg(long, default_value = "tests/golden")]
    golden_root: PathBuf,
    /// 输出根目录（默认 <golden_root>/_regression/<timestamp>）
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// 渲染 DPI
    #[arg(long, default_value_t = compare_pages::DEFAULT_COMPARE_DPI)]
    dpi: u32,
    /// mutool 二进制
    #[arg(long, default_value = "mutool")]
    mutool: PathBuf,
    /// SSIM 阈值；任一 fixture overall < 此值 → 退出码 2
    #[arg(long, default_value_t = 0.95)]
    min_ssim: f64,
    /// 仅检查环境/列出 fixture 后退出
    #[arg(long)]
    dry_run: bool,
}

/// 单 fixture 汇总。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FixtureSummary {
    name: String,
    status: String,
    overall_ssim: f64,
    pages_compared: usize,
    error: Option<String>,
}

/// 跨 fixture 总结。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegressionSummary {
    timestamp_unix: u64,
    mode: String,
    dpi: u32,
    min_ssim: f64,
    fixtures: Vec<FixtureSummary>,
    overall_pass: bool,
}

const DEFAULT_FIXTURES: &[&str] = &[
    "single-column",
    "two-column",
    "three-column",
    "scanned",
    "skewed-scan",
    "mixed-text-image",
    "blank-page",
    "complex-layout",
    "chinese",
    "formula",
    "cover",
    "encrypted",
];

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("run_regression 失败: {e:#}");
            ExitCode::from(10)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    let fixtures: Vec<String> = if cli.all {
        DEFAULT_FIXTURES.iter().map(|s| s.to_string()).collect()
    } else if !cli.fixtures.is_empty() {
        cli.fixtures.clone()
    } else {
        bail!("请使用 --all 或 --fixture <name>");
    };

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let out_root = cli.out_dir.clone().unwrap_or_else(|| {
        cli.golden_root
            .join("_regression")
            .join(timestamp.to_string())
    });

    if cli.dry_run {
        println!("dry-run: mode={:?} fixtures={:?}", cli.mode, fixtures);
        println!("out_dir={}", out_root.display());
        return Ok(ExitCode::SUCCESS);
    }

    std::fs::create_dir_all(&out_root)
        .with_context(|| format!("创建输出目录失败: {}", out_root.display()))?;

    let opts = CompareOptions {
        dpi: cli.dpi,
        mutool_bin: cli.mutool.clone(),
        password: None,
        max_pages: None,
    };

    let mut summaries = Vec::with_capacity(fixtures.len());
    let mut overall_pass = true;

    for name in &fixtures {
        let result = run_one(name, &cli, &opts, &out_root);
        match result {
            Ok((report, password_used)) => {
                let pass = report.overall_ssim >= cli.min_ssim && !report.pages.is_empty();
                if !pass {
                    overall_pass = false;
                }
                println!(
                    "[{}] {}: overall_SSIM={:.4} pages={} {}{}",
                    if pass { "OK" } else { "FAIL" },
                    name,
                    report.overall_ssim,
                    report.pages_compared,
                    if password_used { "(password) " } else { "" },
                    if pass { "" } else { "< min_ssim" },
                );
                summaries.push(FixtureSummary {
                    name: name.clone(),
                    status: if pass { "ok".into() } else { "fail".into() },
                    overall_ssim: report.overall_ssim,
                    pages_compared: report.pages_compared,
                    error: None,
                });
            }
            Err(e) => {
                // blank-page/encrypted 等已知边界条件：标 skipped 而非 fail
                let msg = format!("{e:#}");
                let is_skip = is_known_skip(name, &msg);
                if !is_skip {
                    overall_pass = false;
                }
                println!(
                    "[{}] {}: error: {}",
                    if is_skip { "SKIP" } else { "ERR" },
                    name,
                    msg
                );
                summaries.push(FixtureSummary {
                    name: name.clone(),
                    status: if is_skip {
                        "skipped".into()
                    } else {
                        "error".into()
                    },
                    overall_ssim: 0.0,
                    pages_compared: 0,
                    error: Some(msg),
                });
            }
        }
    }

    let summary = RegressionSummary {
        timestamp_unix: timestamp,
        mode: format!("{:?}", cli.mode),
        dpi: cli.dpi,
        min_ssim: cli.min_ssim,
        fixtures: summaries.clone(),
        overall_pass,
    };
    let summary_path = out_root.join("summary.json");
    std::fs::write(&summary_path, serde_json::to_string_pretty(&summary)?)
        .with_context(|| format!("写 summary.json 失败: {}", summary_path.display()))?;

    write_index_html(&out_root, &summary)?;

    println!(
        "\n== Summary ==\nmode: {:?}\nout_dir: {}\noverall_pass: {}",
        cli.mode,
        out_root.display(),
        overall_pass
    );

    if overall_pass {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(2))
    }
}

fn run_one(
    name: &str,
    cli: &Cli,
    opts: &CompareOptions,
    out_root: &Path,
) -> Result<(ComparisonReport, bool)> {
    let fixture_dir = cli.golden_root.join(name);
    let c_pdf = fixture_dir.join("c-output.pdf");
    if !c_pdf.is_file() {
        bail!("缺少 c-output.pdf: {}", c_pdf.display());
    }

    // 选择 PDF B
    let (b_pdf, mut local_opts, password_used) = match cli.mode {
        Mode::SelfCompare => {
            // c-output.pdf vs c-output.pdf
            (c_pdf.clone(), opts.clone(), false)
        }
        Mode::RustVsC => {
            // M5 未到位前 stub：如果 fixture 旁有 rs-output.pdf 就用，否则 bail
            let rs_pdf = fixture_dir.join("rs-output.pdf");
            if !rs_pdf.is_file() {
                bail!(
                    "rust-vs-c 模式需要 {}, 当前缺失（M5 起 Rust 端 PDF 输出落地后生成）",
                    rs_pdf.display()
                );
            }
            (rs_pdf, opts.clone(), false)
        }
    };

    // 加密 fixture 推迟密码注入到子进程
    if name == "encrypted" {
        local_opts.password = Some("test".to_string());
        // password_used 在下面再标
    }

    let work_dir = out_root.join(format!("work-{name}"));
    let report = compare_pdfs(&c_pdf, &b_pdf, &work_dir, &local_opts)?;

    let json_out = out_root.join(format!("{name}.json"));
    let html_out = out_root.join(format!("{name}.html"));
    write_json_report(&report, &json_out)?;
    write_html_report(&report, &html_out)?;

    // 清理工作目录（保留报告，丢弃中间 PNG）
    let _ = std::fs::remove_dir_all(&work_dir);

    let pw = local_opts.password.is_some() || password_used;
    Ok((report, pw))
}

/// 已知边界 fixture：失败时算 skip 而非 fail。
fn is_known_skip(name: &str, msg: &str) -> bool {
    // blank-page.pdf 经 k2pdfopt 处理产 0 页输出，mutool 渲染会失败 — 已记 Step 2.4 Open Question
    if name == "blank-page" && msg.contains("mutool") {
        return true;
    }
    false
}

fn write_index_html(out_dir: &Path, summary: &RegressionSummary) -> Result<()> {
    let mut html = String::new();
    html.push_str("<!doctype html>\n<html><head><meta charset=\"utf-8\">");
    html.push_str("<title>regression summary</title>");
    html.push_str("<style>body{font-family:sans-serif;margin:1em}");
    html.push_str("table{border-collapse:collapse}th,td{border:1px solid #ccc;padding:.3em .6em}");
    html.push_str(".ok{color:#080}.fail{color:#c00}.skip{color:#888}.err{color:#a60}");
    html.push_str("</style></head><body>");
    html.push_str(&format!(
        "<h1>regression summary</h1><p>mode: {} | dpi: {} | min_ssim: {} | overall_pass: <b>{}</b></p>",
        summary.mode, summary.dpi, summary.min_ssim, summary.overall_pass
    ));
    html.push_str("<table><thead><tr><th>fixture</th><th>status</th><th>overall SSIM</th><th>pages</th><th>error</th></tr></thead><tbody>");
    for f in &summary.fixtures {
        let cls = match f.status.as_str() {
            "ok" => "ok",
            "fail" => "fail",
            "skipped" => "skip",
            _ => "err",
        };
        let err = f.error.clone().unwrap_or_default();
        html.push_str(&format!(
            "<tr><td><a href=\"{0}.html\">{0}</a></td><td class=\"{1}\">{2}</td><td>{3:.4}</td><td>{4}</td><td>{5}</td></tr>",
            f.name, cls, f.status, f.overall_ssim, f.pages_compared, err
        ));
    }
    html.push_str("</tbody></table></body></html>");
    let path = out_dir.join("index.html");
    std::fs::write(&path, html)
        .with_context(|| format!("写 index.html 失败: {}", path.display()))?;
    Ok(())
}
