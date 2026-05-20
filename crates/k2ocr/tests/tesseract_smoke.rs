//! Tesseract CLI 集成测试 —— 需要本机装 tesseract。
//!
//! **graceful skip 策略**：
//! - 若 `tesseract --version` 失败 → 整文件跳过（与 Step 6.1/6.2/6.3 source_*_smoke 同源）
//! - 若 fixture `test_page.png` 不存在（Spike 目录被清理）→ 单测跳过
//! - chi_sim 等可选语言包未装 → 单测跳过对应 case
//!
//! 测试覆盖：
//! - probe / list_langs 基本流程
//! - 用 spike test_page.png 真实跑 recognize，断言 word 列表非空
//! - ROI 局部识别 + 坐标 offset 正确
//! - 语言包缺失错误正确触发
//! - min_confidence 过滤生效

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

use k2ocr::{OcrEngine, OcrError, OcrPageInput, OcrRoi, TesseractCliEngine};
use k2types::{Bitmap, PixelFormat};

/// `tesseract --version` 能成功 → 引擎可用。
fn tesseract_available() -> bool {
    Command::new("tesseract")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 读 spike 提供的 `test_page.png`（150 DPI 演示幻灯片）。
fn load_test_page() -> Option<Bitmap> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("spikes")
        .join("ocr-cli")
        .join("fixtures")
        .join("test_page.png");
    if !path.exists() {
        return None;
    }
    k2core::bitmap::read_png(&path, 150.0).ok()
}

#[test]
fn smoke_probe_returns_version() {
    if !tesseract_available() {
        eprintln!("skip: tesseract not available");
        return;
    }
    let e = TesseractCliEngine::new();
    let info = e.probe().expect("probe should succeed");
    assert_eq!(info.engine_name, "tesseract-cli");
    assert!(!info.version.is_empty(), "version not empty: {info:?}");
    // 任何 tesseract 都至少 major version 第一个数字
    assert!(
        info.version
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false),
        "version starts with digit: {}",
        info.version
    );
}

#[test]
fn smoke_probe_cached_repeated_calls() {
    if !tesseract_available() {
        return;
    }
    let e = TesseractCliEngine::new();
    let info1 = e.probe().expect("first probe");
    let info2 = e.probe().expect("second probe");
    assert_eq!(info1, info2);
}

#[test]
fn smoke_list_langs_contains_eng() {
    if !tesseract_available() {
        return;
    }
    let e = TesseractCliEngine::new();
    let langs = e.list_langs().expect("list_langs");
    assert!(
        langs.iter().any(|l| l == "eng"),
        "eng must be installed for smoke tests: got {langs:?}"
    );
}

#[test]
fn smoke_list_langs_cached() {
    if !tesseract_available() {
        return;
    }
    let e = TesseractCliEngine::new();
    let a = e.list_langs().expect("first");
    let b = e.list_langs().expect("second");
    assert_eq!(a, b);
}

#[test]
fn smoke_recognize_test_page_returns_words() {
    if !tesseract_available() {
        return;
    }
    let Some(bmp) = load_test_page() else {
        eprintln!("skip: spike test_page.png not found");
        return;
    };
    let e = TesseractCliEngine::new();
    let words = e
        .recognize(&OcrPageInput::new(&bmp, 150.0).with_lang("eng"))
        .expect("recognize should succeed on test_page.png");
    assert!(
        !words.is_empty(),
        "expected non-empty word list from test_page.png"
    );
    // 至少 1 个 word conf > 0.5 才算"OCR 真的工作了"
    let high_conf_count = words.iter().filter(|w| w.confidence >= 0.5).count();
    assert!(
        high_conf_count >= 1,
        "expected at least 1 word with conf>=0.5; got 0 (total={})",
        words.len()
    );
    // confidence 必在 [0, 1] 范围
    for w in &words {
        assert!(
            (0.0..=1.0).contains(&w.confidence),
            "confidence out of range: {} (text='{}')",
            w.confidence,
            w.text
        );
    }
}

#[test]
fn smoke_recognize_with_roi_offsets_coords() {
    if !tesseract_available() {
        return;
    }
    let Some(bmp) = load_test_page() else {
        return;
    };
    let bw = bmp.width;
    let bh = bmp.height;
    // 左上四分之一 ROI
    let roi = OcrRoi::new(0, 0, bw / 2 - 1, bh / 2 - 1);
    let e = TesseractCliEngine::new();
    let words = e
        .recognize(
            &OcrPageInput::new(&bmp, 150.0)
                .with_lang("eng")
                .with_roi(roi),
        )
        .expect("ROI recognize should succeed");
    for w in &words {
        // ROI 限定在 [0, bw/2)，且 word.x 是 ROI offset + local，最小可能 0
        assert!(
            w.x >= 0.0,
            "word x should be >= 0; got {} for '{}'",
            w.x,
            w.text
        );
        // word 在 ROI 内时 x < ROI 宽度边界（允许 word 出 ROI 外，但 OCR 仅看 ROI 内像素 → 大多数 word x+w < bw/2）
        assert!(w.y >= 0.0);
    }
}

#[test]
fn smoke_recognize_roi_with_nonzero_offset() {
    if !tesseract_available() {
        return;
    }
    let Some(bmp) = load_test_page() else {
        return;
    };
    let bw = bmp.width;
    let bh = bmp.height;
    // 右下四分之一 ROI（roi.x0 > 0 验证 offset 真的加进去了）
    let x0 = bw / 2;
    let y0 = bh / 2;
    let roi = OcrRoi::new(x0, y0, bw - 1, bh - 1);
    let e = TesseractCliEngine::new();
    let words = e
        .recognize(
            &OcrPageInput::new(&bmp, 150.0)
                .with_lang("eng")
                .with_roi(roi),
        )
        .expect("ROI recognize should succeed");
    // 如果 ROI 内有 word，那么 word.x >= x0 - small tolerance（TSV 局部 left=0 时即 = x0）
    if !words.is_empty() {
        let any_offset = words
            .iter()
            .any(|w| w.x >= f64::from(x0) - 1.0 && w.y >= f64::from(y0) - 1.0);
        assert!(
            any_offset,
            "at least 1 word x>=x0 && y>=y0 expected after ROI offset; got x0={x0} y0={y0} words={:?}",
            words.iter().take(3).collect::<Vec<_>>()
        );
    }
}

#[test]
fn smoke_language_not_installed_returns_error() {
    if !tesseract_available() {
        return;
    }
    let Some(bmp) = load_test_page() else {
        return;
    };
    let e = TesseractCliEngine::new();
    let r = e.recognize(&OcrPageInput::new(&bmp, 150.0).with_lang("xxx_nonexistent_lang"));
    assert!(
        matches!(r, Err(OcrError::LanguageNotInstalled { .. })),
        "expected LanguageNotInstalled; got {r:?}"
    );
}

#[test]
fn smoke_min_confidence_filter_drops_low_conf_words() {
    if !tesseract_available() {
        return;
    }
    let Some(bmp) = load_test_page() else {
        return;
    };
    let e = TesseractCliEngine::new();
    let all = e
        .recognize(&OcrPageInput::new(&bmp, 150.0).with_lang("eng"))
        .expect("baseline recognize");
    let filtered = e
        .recognize(
            &OcrPageInput::new(&bmp, 150.0)
                .with_lang("eng")
                .with_min_confidence(0.95),
        )
        .expect("filtered recognize");
    assert!(
        filtered.len() <= all.len(),
        "filtered ({}) should be <= all ({})",
        filtered.len(),
        all.len()
    );
    for w in &filtered {
        assert!(
            w.confidence >= 0.95,
            "filtered word should have conf>=0.95; got {} '{}'",
            w.confidence,
            w.text
        );
    }
}

#[test]
fn smoke_recognize_default_lang_is_eng() {
    if !tesseract_available() {
        return;
    }
    let Some(bmp) = load_test_page() else {
        return;
    };
    let e = TesseractCliEngine::new();
    // 空 lang 默认走 eng
    let words = e
        .recognize(&OcrPageInput::new(&bmp, 150.0))
        .expect("default lang recognize");
    // 跟显式 eng 比 — 应一致或非常接近（tesseract 内部为同一调用，但因新进程 cache 可能极小差异）
    let words_eng = e
        .recognize(&OcrPageInput::new(&bmp, 150.0).with_lang("eng"))
        .expect("eng recognize");
    assert_eq!(words.len(), words_eng.len());
}

#[test]
fn smoke_trait_object_dispatch() {
    if !tesseract_available() {
        return;
    }
    let Some(bmp) = load_test_page() else {
        return;
    };
    let e: Box<dyn OcrEngine> = Box::new(TesseractCliEngine::new());
    assert_eq!(e.engine_name(), "tesseract-cli");
    let _info = e.probe().expect("trait probe");
    let langs = e.list_langs().expect("trait list_langs");
    assert!(langs.iter().any(|l| l == "eng"));
    let words = e
        .recognize(&OcrPageInput::new(&bmp, 150.0).with_lang("eng"))
        .expect("trait recognize");
    assert!(!words.is_empty());
}

#[test]
fn smoke_empty_bitmap_skips_engine() {
    // tesseract 不会被调用 - effective_roi 早返 EmptyBitmap
    let bmp = Bitmap::from_raw(0, 100, 300.0, PixelFormat::Gray8, Vec::new());
    let Ok(bmp) = bmp else {
        return; // Bitmap::from_raw 拒空，符合 k2types 行为
    };
    let e = TesseractCliEngine::new();
    let r = e.recognize(&OcrPageInput::new(&bmp, 300.0));
    assert!(matches!(r, Err(OcrError::EmptyBitmap)));
}

#[test]
fn smoke_engine_not_found_path() {
    // 不依赖 tesseract 是否实际可用 - 用一个保证不存在的二进制名
    let e = TesseractCliEngine::new().with_executable("k2ocr_definitely_no_such_program_xyz_42x77");
    let r = e.probe();
    assert!(
        matches!(r, Err(OcrError::EngineNotFound { .. })),
        "expected EngineNotFound; got {r:?}"
    );
}
