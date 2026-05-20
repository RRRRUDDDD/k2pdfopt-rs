//! `PdfInfo` 端到端集成测试 - 用 fixture 与 golden PDF 跑 `mutool info` 全链路。
//!
//! 本机若无 mutool 二进制，相关测试自动 skip（不算 fail）。详见 Step 4.2。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use k2render::{PdfInfo, PdfInfoOptions, RenderError};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../tests/fixtures");
    p.push(name);
    p
}

fn golden(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../tests/golden");
    p.push(name);
    p.push("c-output.pdf");
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
fn pdfinfo_from_path_basic_single_column() {
    require_mutool!();
    let info = PdfInfo::from_path(fixture("single-column.pdf")).unwrap();
    assert!(info.page_count >= 1, "page_count should be >=1");
    assert!(!info.encrypted, "fixture is not encrypted");
    assert!(
        info.pdf_version.is_some(),
        "pdf_version should be parsed: {:?}",
        info.pdf_version
    );
    // synthetic fixture 没有 Info 字典
    assert!(
        info.title.is_none() && info.producer.is_none() && info.author.is_none(),
        "synthetic fixture should have no metadata"
    );
    // 单页 A4 类 mediabox
    assert!(
        !info.mediaboxes_pt.is_empty(),
        "at least one mediabox expected"
    );
    let (w, h) = info.mediaboxes_pt[0];
    assert!(w > 100.0 && w < 2000.0);
    assert!(h > 100.0 && h < 2000.0);
}

#[test]
fn pdfinfo_two_column_page_count() {
    require_mutool!();
    let info = PdfInfo::from_path(fixture("two-column.pdf")).unwrap();
    assert_eq!(info.page_count, 2, "two-column.pdf fixture has 2 pages");
    assert!(!info.encrypted);
}

#[test]
fn pdfinfo_encrypted_no_password_returns_typed_error() {
    require_mutool!();
    let err = PdfInfo::from_path(fixture("encrypted.pdf")).unwrap_err();
    let typed = err
        .downcast_ref::<RenderError>()
        .expect("err should be typed RenderError");
    assert!(
        matches!(typed, RenderError::Encrypted { .. }),
        "expected Encrypted, got {typed:?}"
    );
}

#[test]
fn pdfinfo_encrypted_with_correct_password_succeeds() {
    require_mutool!();
    let opts = PdfInfoOptions {
        password: Some("test".to_string()),
        ..Default::default()
    };
    let info = PdfInfo::from_path_with_options(fixture("encrypted.pdf"), opts).unwrap();
    assert_eq!(info.page_count, 1);
    // 解密后仍能在 stdout 看到 Encryption object 节（mutool 保留）
    assert!(
        info.encrypted,
        "Encryption object section should still be present even after auth"
    );
    assert!(info.pdf_version.is_some());
    assert!(!info.mediaboxes_pt.is_empty());
}

#[test]
fn pdfinfo_golden_single_column_has_producer() {
    require_mutool!();
    let path = golden("single-column");
    if !path.exists() {
        eprintln!("[skip] golden PDF not present: {}", path.display());
        return;
    }
    let info = PdfInfo::from_path(&path).unwrap();
    assert!(!info.encrypted);
    assert!(info.page_count >= 1);
    assert_eq!(
        info.producer.as_deref(),
        Some("K2pdfopt v2.55"),
        "C version golden PDFs should advertise K2pdfopt v2.55 as Producer"
    );
    assert!(
        info.title.is_some(),
        "golden PDF should have a Title (typically `c-output.pdf`)"
    );
    assert!(
        info.creation_date.is_some(),
        "golden PDF should have CreationDate"
    );
    assert!(info.mod_date.is_some(), "golden PDF should have ModDate");
}

#[test]
fn pdfinfo_golden_two_column_metadata_consistent() {
    require_mutool!();
    let path = golden("two-column");
    if !path.exists() {
        eprintln!("[skip] golden PDF not present: {}", path.display());
        return;
    }
    let info = PdfInfo::from_path(&path).unwrap();
    assert!(info.page_count >= 1, "golden two-column has >=1 page");
    assert_eq!(info.producer.as_deref(), Some("K2pdfopt v2.55"));
    assert!(info.pdf_version.is_some());
}

#[test]
fn pdfinfo_missing_pdf_file_errors() {
    let err = PdfInfo::from_path(fixture("does-not-exist.pdf")).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("not found"), "should mention not-found: {msg}");
}

#[test]
fn pdfinfo_missing_binary_returns_binary_not_found() {
    let opts = PdfInfoOptions {
        password: None,
        binary: PathBuf::from("definitely-not-on-path-xyz-k2render-pdfinfo-test"),
    };
    let err = PdfInfo::from_path_with_options(fixture("single-column.pdf"), opts).unwrap_err();
    let typed = err
        .downcast_ref::<RenderError>()
        .expect("err should be typed RenderError");
    assert!(
        matches!(typed, RenderError::BinaryNotFound(_)),
        "expected BinaryNotFound, got {typed:?}"
    );
}

#[test]
fn pdfinfo_multiple_fixtures_smoke() {
    require_mutool!();
    let names = [
        "single-column.pdf",
        "two-column.pdf",
        "three-column.pdf",
        "scanned.pdf",
        "blank-page.pdf",
        "chinese.pdf",
        "cover.pdf",
        "formula.pdf",
        "complex-layout.pdf",
        "mixed-text-image.pdf",
        "skewed-scan.pdf",
    ];
    for n in names {
        let path = fixture(n);
        if !path.exists() {
            eprintln!("[skip] fixture not present: {n}");
            continue;
        }
        let info =
            PdfInfo::from_path(&path).unwrap_or_else(|e| panic!("from_path({n}) failed: {e}"));
        assert!(
            info.pdf_version.is_some(),
            "fixture {n} should have a pdf_version"
        );
        // blank-page 可能仍是 1 页（一张空白页），不能强制 page_count > 0
        // 但所有 fixture 至少应能解析出 Pages 行
    }
}

#[test]
fn pdfinfo_idempotent_multiple_calls() {
    require_mutool!();
    let path = fixture("single-column.pdf");
    let a = PdfInfo::from_path(&path).unwrap();
    let b = PdfInfo::from_path(&path).unwrap();
    assert_eq!(a, b, "PdfInfo from same file should be equal across calls");
}

#[test]
fn pdfinfo_struct_supports_clone_and_debug() {
    let opts = PdfInfoOptions::default();
    let cloned = opts.clone();
    assert_eq!(opts.binary, cloned.binary);
    assert!(format!("{opts:?}").contains("PdfInfoOptions"));
}

#[test]
fn pdfinfo_encrypted_wrong_password_returns_encrypted_error() {
    require_mutool!();
    let opts = PdfInfoOptions {
        password: Some("wrong-password".to_string()),
        ..Default::default()
    };
    let err = PdfInfo::from_path_with_options(fixture("encrypted.pdf"), opts).unwrap_err();
    let typed = err
        .downcast_ref::<RenderError>()
        .expect("err should be typed RenderError");
    assert!(
        matches!(typed, RenderError::Encrypted { .. }),
        "wrong password should still surface Encrypted, got {typed:?}"
    );
}
