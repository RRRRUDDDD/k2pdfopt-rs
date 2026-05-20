//! mutool 后端端到端集成测试 - 用 fixture single-column.pdf 等真实 PDF 跑全链路。
//!
//! 本机若无 mutool 二进制，相关测试会自动 skip（不算 fail）。详见 Step 4.1。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use k2render::{DocumentRenderer, MutoolOptions, MutoolRenderer, RenderError};
use k2types::PixelFormat;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../tests/fixtures");
    p.push(name);
    p
}

fn mutool_available() -> bool {
    std::process::Command::new("mutool")
        .arg("-v")
        .output()
        .is_ok()
}

macro_rules! require_mutool {
    () => {
        if !mutool_available() {
            eprintln!("[skip] mutool not in PATH");
            return;
        }
    };
}

#[test]
fn renderer_construct_single_column() {
    require_mutool!();
    let r = MutoolRenderer::new(fixture("single-column.pdf")).unwrap();
    let n = r.page_count().unwrap();
    assert!(n >= 1, "expected >=1 page, got {n}");
}

#[test]
fn page_count_consistent_across_calls() {
    require_mutool!();
    let r = MutoolRenderer::new(fixture("single-column.pdf")).unwrap();
    let a = r.page_count().unwrap();
    let b = r.page_count().unwrap();
    assert_eq!(a, b);
}

#[test]
fn page_size_a4_like_bounds() {
    require_mutool!();
    let r = MutoolRenderer::new(fixture("single-column.pdf")).unwrap();
    let (w_pt, h_pt) = r.page_size(0).unwrap();
    // fixture 是合成的 A4 类页面；用宽松上下界确保确实拿到了 mediabox
    assert!(
        w_pt > 100.0 && w_pt < 2000.0,
        "unexpected page width {w_pt}pt"
    );
    assert!(
        h_pt > 100.0 && h_pt < 2000.0,
        "unexpected page height {h_pt}pt"
    );
}

#[test]
fn render_page0_low_dpi_produces_bitmap() {
    require_mutool!();
    let r = MutoolRenderer::new(fixture("single-column.pdf")).unwrap();
    let page = r.render_page(0, 50.0).unwrap();
    assert_eq!(page.page_index, 0);
    assert!((page.source_dpi - 50.0).abs() < 1e-3);
    assert!(page.bitmap.width > 0, "width should be positive");
    assert!(page.bitmap.height > 0, "height should be positive");
    assert!(matches!(
        page.bitmap.format,
        PixelFormat::Rgba8 | PixelFormat::Rgb8 | PixelFormat::Gray8
    ));
    // 50 DPI 下 A4 大致 414x585，给宽松上界避免过严
    assert!(
        page.bitmap.width <= 4000,
        "bitmap width too large: {}",
        page.bitmap.width
    );
    assert!(
        page.bitmap.height <= 4000,
        "bitmap height too large: {}",
        page.bitmap.height
    );
    let expected_len = page.bitmap.bytes_per_row() * (page.bitmap.height as usize);
    assert_eq!(
        page.bitmap.pixels.len(),
        expected_len,
        "pixels.len() must match width * height * bpp"
    );
    // mediabox 应给出非零的物理尺寸
    let (w_pt, h_pt) = page.source_size_pt;
    assert!(w_pt > 0.0 && h_pt > 0.0);
}

#[test]
fn render_page_with_two_columns_fixture() {
    require_mutool!();
    let r = MutoolRenderer::new(fixture("two-column.pdf")).unwrap();
    let total = r.page_count().unwrap();
    assert!(total >= 1);
    let page = r.render_page(0, 72.0).unwrap();
    // 72 DPI = 1 px / pt，bitmap 尺寸应与 mediabox 数量级一致
    let (w_pt, h_pt) = page.source_size_pt;
    let w_px = page.bitmap.width as f32;
    let h_px = page.bitmap.height as f32;
    // 给 ±2 像素的舍入容差（mutool 内部 round-half 行为）
    assert!(
        (w_px - w_pt).abs() < 3.0,
        "expected w_px≈w_pt at 72dpi; got w_px={w_px}, w_pt={w_pt}"
    );
    assert!(
        (h_px - h_pt).abs() < 3.0,
        "expected h_px≈h_pt at 72dpi; got h_px={h_px}, h_pt={h_pt}"
    );
}

#[test]
fn render_page_out_of_range_typed_error() {
    require_mutool!();
    let r = MutoolRenderer::new(fixture("single-column.pdf")).unwrap();
    let total = r.page_count().unwrap();
    let err = r.render_page(total + 100, 100.0).unwrap_err();
    let typed = err.downcast_ref::<RenderError>().unwrap();
    assert!(
        matches!(typed, RenderError::PageOutOfRange { .. }),
        "expected PageOutOfRange, got {typed:?}"
    );
}

#[test]
fn page_size_out_of_range_typed_error() {
    require_mutool!();
    let r = MutoolRenderer::new(fixture("single-column.pdf")).unwrap();
    let total = r.page_count().unwrap();
    let err = r.page_size(total + 5).unwrap_err();
    let typed = err.downcast_ref::<RenderError>().unwrap();
    assert!(matches!(typed, RenderError::PageOutOfRange { .. }));
}

#[test]
fn encrypted_pdf_no_password_returns_encrypted_error() {
    require_mutool!();
    let path = fixture("encrypted.pdf");
    let err = MutoolRenderer::new(&path).unwrap_err();
    let typed = err
        .downcast_ref::<RenderError>()
        .expect("error should be typed RenderError");
    assert!(
        matches!(typed, RenderError::Encrypted { .. }),
        "expected Encrypted, got {typed:?}"
    );
}

#[test]
fn encrypted_pdf_with_correct_password_works() {
    require_mutool!();
    let path = fixture("encrypted.pdf");
    let opts = MutoolOptions {
        password: Some("test".to_string()),
        ..Default::default()
    };
    let r = MutoolRenderer::with_options(&path, opts).unwrap();
    let n = r.page_count().unwrap();
    assert!(n >= 1, "encrypted fixture should have >=1 page after auth");
    // 进一步渲染验证 PAM 解析路径在密码场景下也工作
    let page = r.render_page(0, 50.0).unwrap();
    assert!(page.bitmap.width > 0 && page.bitmap.height > 0);
}

#[test]
fn missing_binary_returns_binary_not_found() {
    // 这条测试不依赖系统 mutool，传一个绝对不存在的二进制路径
    let opts = MutoolOptions {
        password: None,
        binary: PathBuf::from("definitely-not-on-path-xyz-k2render-test"),
    };
    let err = MutoolRenderer::with_options(fixture("single-column.pdf"), opts).unwrap_err();
    let typed = err.downcast_ref::<RenderError>().unwrap();
    assert!(
        matches!(typed, RenderError::BinaryNotFound(_)),
        "expected BinaryNotFound, got {typed:?}"
    );
}

#[test]
fn missing_pdf_file_returns_error() {
    let err = MutoolRenderer::new(fixture("does-not-exist.pdf")).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("not found"),
        "msg should mention not-found: {msg}"
    );
}

#[test]
fn invalid_dpi_rejected() {
    require_mutool!();
    let r = MutoolRenderer::new(fixture("single-column.pdf")).unwrap();
    let err = r.render_page(0, -1.0).unwrap_err();
    assert!(
        format!("{err}").to_lowercase().contains("dpi"),
        "expected error to mention dpi: {err}"
    );
    let err = r.render_page(0, 0.0).unwrap_err();
    assert!(format!("{err}").to_lowercase().contains("dpi"));
    let err = r.render_page(0, f32::NAN).unwrap_err();
    assert!(format!("{err}").to_lowercase().contains("dpi"));
    let err = r.render_page(0, f32::INFINITY).unwrap_err();
    assert!(format!("{err}").to_lowercase().contains("dpi"));
}

#[test]
fn render_multiple_pages_when_available() {
    require_mutool!();
    let r = MutoolRenderer::new(fixture("two-column.pdf")).unwrap();
    let total = r.page_count().unwrap();
    let cap = total.min(3);
    for i in 0..cap {
        let page = r.render_page(i, 50.0).unwrap();
        assert_eq!(page.page_index, i);
        assert!(page.bitmap.pixels.len() >= 4);
    }
}
