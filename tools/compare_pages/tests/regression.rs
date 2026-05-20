//! Step 5.7 端到端回归集成测试。
//!
//! 三类测试：
//! 1. 纯 PNG self-compare：直接对 c-pages/*.png 调 compare_png_pair，验证 SSIM=1.0
//! 2. fixture 覆盖：扫描 tests/golden/* 至少 12 个目录都能加载首页 PNG
//! 3. mutool 可达性 + PDF self-compare：mutool 装则跑端到端；否则 skip
//!
//! 集成测试位置：`tools/compare_pages/tests/regression.rs`
//! 因为 workspace 根 `tests/` 不属任何 crate，cargo 不会自动跑；本文件即
//! execution-plan §5.7 中 "tests/regression.rs" 的实际承载点。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use compare_pages::{compare_pdfs, compare_png_pair, load_png_gray, ssim_mean, CompareOptions};

/// 找到 workspace 根目录（含 tests/golden/）。
fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = .../k2pdfopt-rs/tools/compare_pages
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .map(|p| p.to_path_buf())
        .expect("workspace root not found")
}

fn golden_root() -> PathBuf {
    workspace_root().join("tests").join("golden")
}

fn has_mutool() -> bool {
    Command::new("mutool")
        .arg("-v")
        .output()
        .map(|o| o.status.success() || !o.stderr.is_empty())
        .unwrap_or(false)
}

#[test]
fn golden_root_has_at_least_12_fixtures() {
    let root = golden_root();
    assert!(root.is_dir(), "tests/golden/ 缺失: {}", root.display());
    let count = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && !e.file_name().to_string_lossy().starts_with('_')
        })
        .count();
    assert!(
        count >= 12,
        "tests/golden/ 应至少含 12 个 fixture 目录，实际 {count}"
    );
}

#[test]
fn png_self_compare_yields_ssim_one() {
    // 用 single-column fixture 首页 PNG 与自身比对，期望 SSIM=1.0
    let png = golden_root()
        .join("single-column")
        .join("c-pages")
        .join("page-0001.png");
    if !png.is_file() {
        eprintln!(
            "skip: 缺失 baseline PNG: {} (Step 2.4 应已生成)",
            png.display()
        );
        return;
    }
    let img = load_png_gray(&png).unwrap();
    let s = ssim_mean(&img, &img).unwrap();
    assert!((s - 1.0).abs() < 1e-9, "self SSIM 应为 1.0，实际 {s}");
}

#[test]
fn compare_png_pair_self_sanity() {
    let png = golden_root()
        .join("two-column")
        .join("c-pages")
        .join("page-0001.png");
    if !png.is_file() {
        eprintln!("skip: 缺失 {}", png.display());
        return;
    }
    let pc = compare_png_pair(0, &png, &png).unwrap();
    assert!((pc.ssim_mean - 1.0).abs() < 1e-9);
    assert_eq!(pc.diff.max_abs, 0);
    assert!(!pc.size_mismatch);
}

#[test]
fn pdf_self_compare_via_mutool_yields_high_ssim() {
    if !has_mutool() {
        eprintln!("skip: mutool 不可用");
        return;
    }
    let pdf = golden_root().join("single-column").join("c-output.pdf");
    if !pdf.is_file() {
        eprintln!("skip: 缺失 {}", pdf.display());
        return;
    }
    let work_dir =
        std::env::temp_dir().join(format!("compare_pages-regression-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work_dir);
    let opts = CompareOptions::default();
    let report = compare_pdfs(&pdf, &pdf, &work_dir, &opts).unwrap();
    let _ = std::fs::remove_dir_all(&work_dir);
    assert!(report.pages_compared > 0, "至少应比对 1 页");
    assert!(
        report.overall_ssim > 0.999,
        "self-compare overall_SSIM 应接近 1.0，实际 {}",
        report.overall_ssim
    );
    for p in &report.pages {
        assert!(
            p.ssim_mean > 0.999,
            "page {} SSIM={:.6} 应接近 1.0",
            p.page_index,
            p.ssim_mean
        );
        assert_eq!(p.diff.max_abs, 0);
    }
}

#[test]
fn fixture_dirs_have_metadata_or_baseline() {
    let root = golden_root();
    if !root.is_dir() {
        eprintln!("skip: {} 不存在", root.display());
        return;
    }
    let mut covered = 0;
    for entry in std::fs::read_dir(&root).unwrap().flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('_') {
            continue;
        }
        let dir = entry.path();
        let meta = dir.join("metadata.json");
        let c_output = dir.join("c-output.pdf");
        // 至少应有 metadata.json 或 c-output.pdf
        if meta.is_file() || c_output.is_file() {
            covered += 1;
        }
    }
    assert!(
        covered >= 12,
        "至少 12 fixture 应有 metadata.json 或 c-output.pdf，实际 {covered}"
    );
}

#[test]
fn render_strategy_documented() {
    // 防止未来误删 resize_strategy 描述
    let s = compare_pages::resize_strategy();
    assert!(s.contains("尺寸"));
    assert!(s.contains("size_mismatch"));
}

#[test]
fn ssim_window_constants_match_wikipedia() {
    assert_eq!(compare_pages::SSIM_WINDOW, 11);
    assert!((compare_pages::SSIM_SIGMA - 1.5).abs() < 1e-9);
    assert!((compare_pages::SSIM_K1 - 0.01).abs() < 1e-9);
    assert!((compare_pages::SSIM_K2 - 0.03).abs() < 1e-9);
    assert!((compare_pages::SSIM_L - 255.0).abs() < 1e-9);
}

#[test]
fn workspace_root_resolved() {
    let root: &Path = &workspace_root();
    assert!(root.join("Cargo.toml").is_file());
    assert!(root.join("crates").is_dir());
}
