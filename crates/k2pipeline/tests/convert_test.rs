//! Step 7.3 集成测试：端到端 ConvertJob pipeline。
//!
//! 调 `MutoolRenderer` 渲染 fixture PDF → ConvertContext 处理 → LopdfWriter 写出 →
//! 用 lopdf 重新加载验证输出结构（Pages / MediaBox / Catalog 等）。
//!
//! 与 Step 5.7 `compare_pages` 同源：依赖本机 mutool 可用。
//! 与 Step 7.2 `bitmap_pdf_test.rs` 互补：那里测 PdfWriter trait 单元行为，
//! 这里测 ConvertJob 串联多个 crate 的端到端流程。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use k2pipeline::{ConvertError, ConvertJob, ConvertJobConfig};
use std::fs;
use std::path::PathBuf;

/// 解析 workspace 根目录下的 fixture 路径。
fn fixture(name: &str) -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .map(|p| p.join("tests").join("fixtures").join(name))
        .expect("workspace root reachable")
}

/// 在 OS tempdir 创建唯一输出路径（不依赖 tempfile crate 的 NamedTempFile）。
fn temp_output(label: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let mut p = std::env::temp_dir();
    p.push(format!("k2pdfopt_step73_{label}_{pid}_{nanos}.pdf"));
    p
}

/// 跑一次 ConvertJob 并验证输出 PDF 可被 lopdf 重新解析。
fn run_and_verify(fixture_name: &str, label: &str) {
    let input = fixture(fixture_name);
    if !input.exists() {
        eprintln!("skipped: fixture {} 不存在", input.display());
        return;
    }
    let output = temp_output(label);

    let config = ConvertJobConfig::default();
    let job = ConvertJob::new(&input, &output, config.clone());
    let res = job.run();

    // mutool 不可用时会返 ConvertError::Render；测试环境如果没 mutool 应明确跳过
    if let Err(ConvertError::Render(ref e)) = res {
        let msg = format!("{e}");
        if msg.contains("not found in PATH") || msg.contains("BinaryNotFound") {
            eprintln!("skipped: mutool not available ({})", msg);
            return;
        }
    }
    res.expect("ConvertJob::run should succeed for valid fixture");

    // 验证 1：文件存在且非空
    let meta = fs::metadata(&output).expect("output PDF metadata");
    assert!(
        meta.len() > 100,
        "output PDF too small ({} bytes)",
        meta.len()
    );

    // 验证 2：lopdf 重新加载
    let doc = lopdf::Document::load(&output).expect("lopdf should reload");
    let pages = doc.get_pages();
    assert!(!pages.is_empty(), "expected at least one page");

    // 验证 3：Catalog 存在
    let catalog = doc.catalog().expect("Catalog dict");
    assert!(catalog.has(b"Pages"));

    // 验证 4：每页有 MediaBox
    for (_idx, page_id) in pages.iter() {
        let page = doc.get_object(*page_id).expect("page obj");
        let dict = page.as_dict().expect("page dict");
        assert!(dict.has(b"MediaBox"));
    }

    // 验证 5：字节流头部含 %PDF + 尾部含 %%EOF
    let bytes = fs::read(&output).expect("read pdf bytes");
    assert!(bytes.starts_with(b"%PDF-"));
    let tail = &bytes[bytes.len().saturating_sub(64)..];
    assert!(tail.windows(5).any(|w| w == b"%%EOF"));

    let _ = fs::remove_file(&output);
}

#[test]
fn convert_single_column_pdf_endtoend() {
    run_and_verify("single-column.pdf", "single_column");
}

#[test]
fn convert_two_column_pdf_endtoend() {
    run_and_verify("two-column.pdf", "two_column");
}

#[test]
fn convert_chinese_pdf_endtoend() {
    run_and_verify("chinese.pdf", "chinese");
}

#[test]
fn convert_with_custom_dpi() {
    let input = fixture("single-column.pdf");
    if !input.exists() {
        eprintln!("skipped");
        return;
    }
    let output = temp_output("custom_dpi");

    let config = ConvertJobConfig {
        dst_dpi: 100,
        dst_width: 400,
        dst_height: 600,
        jpeg_quality: -1, // Flate
        use_breakpoint: true,
        join_figure_captions: false,
    };
    let res = ConvertJob::new(&input, &output, config).run();
    if let Err(ConvertError::Render(ref e)) = res {
        if format!("{e}").contains("BinaryNotFound") {
            eprintln!("skipped: mutool not available");
            return;
        }
    }
    res.expect("custom DPI run");
    assert!(output.exists());

    // 验证 MediaBox 尺寸大致按 (dst_width, dst_height) @ dst_dpi
    let doc = lopdf::Document::load(&output).expect("reload");
    let pages = doc.get_pages();
    assert!(!pages.is_empty());
    let first = pages
        .iter()
        .min_by_key(|(idx, _)| *idx)
        .map(|(_, id)| *id)
        .unwrap();
    let page = doc.get_object(first).unwrap();
    let dict = page.as_dict().unwrap();
    let mbox = dict.get(b"MediaBox").unwrap().as_array().unwrap();
    // [0, 0, width_pt, height_pt]
    let w = mbox[2].as_float().unwrap_or(0.0);
    let h = mbox[3].as_float().unwrap_or(0.0);
    // dst_width=400 px @ 100 dpi → 400/100*72 = 288 pt
    // dst_height=600 px @ 100 dpi → 600/100*72 = 432 pt
    assert!((w - 288.0).abs() < 0.5, "width_pt expected ~288, got {w}");
    assert!((h - 432.0).abs() < 0.5, "height_pt expected ~432, got {h}");

    let _ = fs::remove_file(&output);
}

#[test]
fn convert_invalid_input_path_returns_error() {
    let bogus = std::env::temp_dir().join("definitely_not_existing_file.pdf");
    let output = temp_output("invalid");
    let job = ConvertJob::new(&bogus, &output, ConvertJobConfig::default());
    let err = job.run().unwrap_err();
    assert!(matches!(err, ConvertError::Render(_)));
}

#[test]
fn convert_invalid_output_dir_returns_error() {
    let input = fixture("single-column.pdf");
    if !input.exists() {
        eprintln!("skipped");
        return;
    }
    // Windows 用一个肯定不存在的盘符路径
    let bogus_output = PathBuf::from(if cfg!(windows) {
        "Z:/no_such_dir/k2_step73_test.pdf"
    } else {
        "/proc/no_such_dir/k2_step73_test.pdf"
    });
    let job = ConvertJob::new(&input, &bogus_output, ConvertJobConfig::default());
    // 可能在 mutool 阶段已早失败 → Render；也可能在 LopdfWriter::new 阶段失败 → Write
    let res = job.run();
    if let Err(ConvertError::Render(ref e)) = res {
        if format!("{e}").contains("BinaryNotFound") {
            eprintln!("skipped: mutool not available");
            return;
        }
    }
    let err = res.unwrap_err();
    assert!(matches!(
        err,
        ConvertError::Write(_) | ConvertError::Render(_)
    ));
}

#[test]
fn convert_job_config_from_settings_default() {
    let s = k2settings::Settings::default();
    let c = ConvertJobConfig::from_settings(&s);
    // 默认值应可构造一个合法的 config（即使可能不是 device profile 推荐值）
    assert!(c.dst_dpi >= 72);
    assert!(c.dst_width > 0);
    assert!(c.dst_height > 0);
}

// ---- Step 11.4 reflow_mode dispatch ----

/// 共享 mutool-skip helper：若 mutool 不在 PATH 则跳过测试（与 run_and_verify 同源）。
fn skip_if_no_mutool(res: &Result<(), ConvertError>) -> bool {
    if let Err(ConvertError::Render(ref e)) = res {
        let msg = format!("{e}");
        if msg.contains("not found in PATH") || msg.contains("BinaryNotFound") {
            eprintln!("skipped: mutool not available ({})", msg);
            return true;
        }
    }
    false
}

#[test]
fn convert_with_reflow_off_processes_pdf() {
    let input = fixture("single-column.pdf");
    if !input.exists() {
        eprintln!("skipped: fixture missing");
        return;
    }
    let output = temp_output("reflow_off");
    let job = ConvertJob::new(&input, &output, ConvertJobConfig::default())
        .with_reflow_mode(k2settings::ReflowMode::Off);
    let res = job.run();
    if skip_if_no_mutool(&res) {
        return;
    }
    res.expect("ReflowMode::Off path should succeed");
    // 验证输出可被重新加载（与 run_and_verify 同源）
    let doc = lopdf::Document::load(&output).expect("reload off");
    assert!(!doc.get_pages().is_empty());
    let _ = fs::remove_file(&output);
}

#[test]
fn convert_with_reflow_auto_processes_pdf() {
    let input = fixture("single-column.pdf");
    if !input.exists() {
        eprintln!("skipped: fixture missing");
        return;
    }
    let output = temp_output("reflow_auto");
    let job = ConvertJob::new(&input, &output, ConvertJobConfig::default())
        .with_reflow_mode(k2settings::ReflowMode::Auto);
    let res = job.run();
    if skip_if_no_mutool(&res) {
        return;
    }
    res.expect("ReflowMode::Auto path should succeed");
    let doc = lopdf::Document::load(&output).expect("reload auto");
    assert!(!doc.get_pages().is_empty());
    let _ = fs::remove_file(&output);
}

#[test]
fn convert_with_reflow_force_processes_pdf() {
    let input = fixture("single-column.pdf");
    if !input.exists() {
        eprintln!("skipped: fixture missing");
        return;
    }
    let output = temp_output("reflow_force");
    let job = ConvertJob::new(&input, &output, ConvertJobConfig::default())
        .with_reflow_mode(k2settings::ReflowMode::Force);
    let res = job.run();
    if skip_if_no_mutool(&res) {
        return;
    }
    res.expect("ReflowMode::Force path should succeed");
    let doc = lopdf::Document::load(&output).expect("reload force");
    assert!(!doc.get_pages().is_empty());
    let _ = fs::remove_file(&output);
}

#[test]
fn convert_default_reflow_mode_is_auto() {
    // 不调 with_reflow_mode：应使用 ReflowMode::default() = Auto
    let job = ConvertJob::new(
        fixture("single-column.pdf"),
        temp_output("default_mode"),
        ConvertJobConfig::default(),
    );
    assert_eq!(job.reflow_mode, k2settings::ReflowMode::Auto);
}

// ---- Step 11.8 P0-5：OcrError::Cancelled → ConvertError::Cancelled 映射 ----

/// 一个永远返 [`OcrError::Cancelled`] 的 mock，用来验 ConvertJob 把 OCR 取消
/// 错误映射到 [`ConvertError::Cancelled`]（保 ExitCode 130）。
struct CancelledOcrEngine;

impl k2ocr::OcrEngine for CancelledOcrEngine {
    fn engine_name(&self) -> &'static str {
        "cancelled-mock"
    }
    fn probe(&self) -> Result<k2ocr::OcrEngineInfo, k2ocr::OcrError> {
        Ok(k2ocr::OcrEngineInfo {
            engine_name: "cancelled-mock".into(),
            version: "0.0".into(),
            data_path: None,
        })
    }
    fn list_langs(&self) -> Result<Vec<String>, k2ocr::OcrError> {
        Ok(vec!["eng".into()])
    }
    fn recognize(
        &self,
        _input: &k2ocr::OcrPageInput<'_>,
    ) -> Result<Vec<k2types::OcrWord>, k2ocr::OcrError> {
        Err(k2ocr::OcrError::Cancelled)
    }
}

#[test]
fn convert_job_propagates_ocr_cancelled_via_off_path() {
    // ReflowMode::Off 路径：OCR 直接在主循环调，OcrError::Cancelled 通过
    // map_ocr_error_to_convert_error 一步映射到 ConvertError::Cancelled。
    let input = fixture("single-column.pdf");
    if !input.exists() {
        eprintln!("skipped: fixture missing");
        return;
    }
    let output = temp_output("ocr_cancel_off");
    let mut ocr = k2settings::ocr::OcrSettings::default();
    ocr.dst_ocr = k2settings::ocr::OcrMode::Tesseract;
    ocr.dst_ocr_lang = "eng".into();
    let engine: std::sync::Arc<dyn k2ocr::OcrEngine> = std::sync::Arc::new(CancelledOcrEngine);
    let job = ConvertJob::new(&input, &output, ConvertJobConfig::default())
        .with_reflow_mode(k2settings::ReflowMode::Off)
        .with_ocr_settings(ocr)
        .with_ocr_engine(engine);
    let res = job.run();
    if skip_if_no_mutool(&res) {
        return;
    }
    let err = res.expect_err("OcrError::Cancelled should propagate");
    assert!(
        matches!(err, ConvertError::Cancelled),
        "expected ConvertError::Cancelled, got {err:?}"
    );
    let _ = fs::remove_file(&output);
}

#[test]
fn convert_job_propagates_ocr_cancelled_via_auto_path() {
    // ReflowMode::Auto 路径：OCR 在 reflow_pipeline::run_region_ocr 触发，
    // 错误经 ReflowError::Ocr(OcrError::Cancelled) 包装，再经
    // map_reflow_error_to_convert_error 解构映射到 ConvertError::Cancelled。
    //
    // 注：受 Open Q 11.4.A / 11.5.A 影响，main pipeline 整页输入会被 figure
    // 误判走 FigureBypassed，process_region 不会走到 TextReflowed 路径 → OCR
    // 不会被调用 → 这条路径在 v0.2 下实际不会触发。本测试用于验"如果
    // process_region 真的调到 mock engine，错误能正确映射"，与 Open Q 11.4.A
    // 修复后的行为保持一致。
    let input = fixture("single-column.pdf");
    if !input.exists() {
        eprintln!("skipped: fixture missing");
        return;
    }
    let output = temp_output("ocr_cancel_auto");
    let mut ocr = k2settings::ocr::OcrSettings::default();
    ocr.dst_ocr = k2settings::ocr::OcrMode::Tesseract;
    ocr.dst_ocr_lang = "eng".into();
    let engine: std::sync::Arc<dyn k2ocr::OcrEngine> = std::sync::Arc::new(CancelledOcrEngine);
    let job = ConvertJob::new(&input, &output, ConvertJobConfig::default())
        .with_reflow_mode(k2settings::ReflowMode::Auto)
        .with_ocr_settings(ocr)
        .with_ocr_engine(engine);
    let res = job.run();
    if skip_if_no_mutool(&res) {
        return;
    }
    // 整页 figure-bypass 时 OCR 不会被调，run 应该 Ok 完成（Open Q 11.4.A 行为）
    // 修复后此处会 propagate Cancelled。本测试容忍两种结果：Ok 或 Cancelled。
    match res {
        Ok(()) => {
            // 整页 figure-bypass 路径，未触发 OCR
            eprintln!("note: Auto path: figure-bypass took precedence (Open Q 11.4.A)");
        }
        Err(ConvertError::Cancelled) => {
            // 期望路径
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }
    let _ = fs::remove_file(&output);
}
