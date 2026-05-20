//! Step 3.5 验收测试 — Settings::to_args() 输出格式验证。
//!
//! 完整的 Settings → args → Settings roundtrip 测试在 k2cli crate 中执行
//! （因 k2settings 不能依赖 k2cli，否则造成循环依赖）。
//! 本文件验证 to_args() 输出字符串的正确性。
//!
//! 测试名以 `roundtrip_` 前缀，使 `cargo test roundtrip_test` 过滤器能命中。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use k2settings::device::find_by_alias;
use k2settings::layout::TextWrap;
use k2settings::ocr::{OcrMode, OcrStrictMode};
use k2settings::source::{CropBox, MarginUnit};
use k2settings::{OcrVisibility, Settings};

/// Helper: check to_args output contains expected strings.
fn assert_args_contain(settings: &Settings, expected: &[&str]) {
    let args = settings.to_args();
    for exp in expected {
        assert!(
            args.iter().any(|a| a == exp),
            "Expected arg '{exp}' not found in to_args() output: {args:?}"
        );
    }
}

fn assert_args_not_contain(settings: &Settings, unexpected: &[&str]) {
    let args = settings.to_args();
    for u in unexpected {
        assert!(
            !args.iter().any(|a| a == u),
            "Unexpected arg '{u}' found in to_args() output: {args:?}"
        );
    }
}

#[test]
fn roundtrip_default_produces_no_args() {
    let settings = Settings::default();
    let args = settings.to_args();
    assert!(
        args.is_empty(),
        "Default settings should produce no args, got: {args:?}"
    );
}

#[test]
fn roundtrip_device_alias() {
    let mut settings = Settings::default();
    let device = find_by_alias("kpw").unwrap();
    settings.apply_device_profile(device);
    settings.destination.device_alias = Some("kpw".to_string());
    let args = settings.to_args();
    assert_eq!(args[0], "--dev");
    assert_eq!(args[1], "kpw");
    assert_eq!(args.len(), 2, "No overrides expected, got: {args:?}");
}

#[test]
fn roundtrip_device_with_override() {
    let mut settings = Settings::default();
    let device = find_by_alias("kpw").unwrap();
    settings.apply_device_profile(device);
    settings.destination.device_alias = Some("kpw".to_string());
    settings.output.dst_color = 1;
    assert_args_contain(&settings, &["--dev", "kpw", "--c"]);
}

#[test]
fn roundtrip_verbose() {
    let mut settings = Settings::default();
    settings.behavior.verbose = 3;
    let args = settings.to_args();
    let v_count = args.iter().filter(|a| *a == "-v").count();
    assert_eq!(v_count, 3, "Expected 3 -v flags, got {v_count}: {args:?}");
}

#[test]
fn roundtrip_color_on() {
    let mut settings = Settings::default();
    settings.output.dst_color = 1;
    assert_args_contain(&settings, &["--c"]);
}

#[test]
fn roundtrip_color_default_no_flag() {
    let settings = Settings::default();
    assert_args_not_contain(&settings, &["--c", "--no-c"]);
}

#[test]
fn roundtrip_trim_disabled() {
    let mut settings = Settings::default();
    settings.source.src_trim = false;
    assert_args_contain(&settings, &["--no-t"]);
}

#[test]
fn roundtrip_fit_columns_disabled() {
    let mut settings = Settings::default();
    settings.layout.fit_columns = false;
    assert_args_contain(&settings, &["--no-fc"]);
}

#[test]
fn roundtrip_text_wrap_off() {
    let mut settings = Settings::default();
    settings.layout.text_wrap = TextWrap::Off;
    assert_args_contain(&settings, &["--no-wrap"]);
}

#[test]
fn roundtrip_text_wrap_extra() {
    let mut settings = Settings::default();
    settings.layout.text_wrap = TextWrap::Extra;
    assert_args_contain(&settings, &["--wrap-extra"]);
}

#[test]
fn roundtrip_landscape_with_pages() {
    let mut settings = Settings::default();
    settings.destination.dst_landscape = true;
    settings.destination.dst_landscape_pages = "3-5".to_string();
    assert_args_contain(&settings, &["--ls-pages", "3-5"]);
}

#[test]
fn roundtrip_landscape_default_no_flag() {
    let settings = Settings::default();
    assert_args_not_contain(&settings, &["--no-ls"]);
}

#[test]
fn roundtrip_justify_center_full() {
    let mut settings = Settings::default();
    settings.output.dst_justify = 1;
    settings.output.dst_fulljustify = 1;
    assert_args_contain(&settings, &["-j", "1+"]);
}

#[test]
fn roundtrip_justify_left_no_full() {
    let mut settings = Settings::default();
    settings.output.dst_justify = 0;
    settings.output.dst_fulljustify = 0;
    assert_args_contain(&settings, &["-j", "0-"]);
}

#[test]
fn roundtrip_odpi() {
    let mut settings = Settings::default();
    settings.destination.dst_userdpi = 300;
    settings.destination.dst_dpi = 300;
    assert_args_contain(&settings, &["--odpi", "300"]);
}

#[test]
fn roundtrip_width_inches() {
    let mut settings = Settings::default();
    settings.destination.dst_userwidth = 6.0;
    settings.destination.dst_userwidth_units = 1;
    let args = settings.to_args();
    let w_idx = args.iter().position(|a| a == "-w").unwrap();
    assert_eq!(args[w_idx + 1], "6in");
}

#[test]
fn roundtrip_height_pixels() {
    let mut settings = Settings::default();
    settings.destination.dst_userheight = 1024.0;
    settings.destination.dst_userheight_units = 0;
    let args = settings.to_args();
    let h_idx = args.iter().position(|a| a == "--height").unwrap();
    assert_eq!(args[h_idx + 1], "1024");
}

#[test]
fn roundtrip_exit_on_complete() {
    let mut settings = Settings::default();
    settings.behavior.exit_on_complete = 1;
    assert_args_contain(&settings, &["-x"]);
}

#[test]
fn roundtrip_assume_yes() {
    let mut settings = Settings::default();
    settings.behavior.assume_yes = true;
    assert_args_contain(&settings, &["-y"]);
}

#[test]
fn roundtrip_non_interactive() {
    let mut settings = Settings::default();
    settings.behavior.query_user = 0;
    settings.behavior.query_user_explicit = false;
    assert_args_contain(&settings, &["--ui-"]);
}

#[test]
fn roundtrip_page_selection() {
    let mut settings = Settings::default();
    settings.behavior.pagelist = "1-10".to_string();
    assert_args_contain(&settings, &["-p", "1-10"]);
}

#[test]
fn roundtrip_pages_exclude() {
    let mut settings = Settings::default();
    settings.behavior.pagexlist = "3,7".to_string();
    assert_args_contain(&settings, &["--px", "3,7"]);
}

#[test]
fn roundtrip_output_format() {
    let mut settings = Settings::default();
    settings.output.dst_opname_format = "out_%s".to_string();
    assert_args_contain(&settings, &["-o", "out_%s"]);
}

#[test]
fn roundtrip_source_margins_equal() {
    let mut settings = Settings::default();
    settings.source.srccropmargins = CropBox {
        box_vals: [0.5; 4],
        units: [MarginUnit::Inches; 4],
        ..CropBox::default()
    };
    assert_args_contain(&settings, &["-m", "0.5in"]);
}

#[test]
fn roundtrip_source_margins_different() {
    let mut settings = Settings::default();
    settings.source.srccropmargins = CropBox {
        box_vals: [0.1, 0.2, 0.3, 0.4],
        units: [MarginUnit::Inches; 4],
        ..CropBox::default()
    };
    assert_args_contain(&settings, &["-m", "0.1in,0.2in,0.3in,0.4in"]);
}

#[test]
fn roundtrip_output_margins() {
    let mut settings = Settings::default();
    settings.destination.dstmargins = CropBox {
        box_vals: [0.01; 4],
        units: [MarginUnit::Inches; 4],
        ..CropBox::default()
    };
    assert_args_contain(&settings, &["--om", "0.01in"]);
}

#[test]
fn roundtrip_multiple_overrides() {
    let mut settings = Settings::default();
    settings.behavior.verbose = 2;
    settings.source.src_trim = false;
    settings.layout.text_wrap = TextWrap::Extra;
    settings.behavior.pagelist = "5-10".to_string();
    let args = settings.to_args();
    assert_args_contain(&settings, &["--no-t", "--wrap-extra", "-p", "5-10"]);
    let v_count = args.iter().filter(|a| *a == "-v").count();
    assert_eq!(v_count, 2);
}

// ──────────────────────────────────────────────────────────────────────
// Step 11.7 P0-4 OCR roundtrip (consumes Open Q 9.3.D / 9.4.H / 10.3.I)
// ──────────────────────────────────────────────────────────────────────
//
// 验证 serialize.rs::push_overrides 对 OcrSettings 字段的输出行为，按
// execution-plan §11.7 操作清单 #1 字面：仅 Tesseract + 非空 lang 时输出
// `--ocr <LANG>`。`cli_echo_cmd_*` 两测试直接用 `Settings::to_args()` 模拟
// `cmd_echo_cmd`（subcommands.rs:14-22 内部就是 `to_args().join(" ")`），
// 端到端真实 `./target/release/k2pdfopt.exe --echo-cmd` 验收见 §11.7 验收命令 #2.

/// OcrMode::Off → 不输出 --ocr。
#[test]
fn roundtrip_ocr_off_no_output() {
    let mut settings = Settings::default();
    settings.ocr.dst_ocr = OcrMode::Off;
    settings.ocr.dst_ocr_lang.clear();
    assert_args_not_contain(&settings, &["--ocr"]);
}

/// OcrMode::Tesseract + lang="eng" → --ocr eng.
#[test]
fn roundtrip_ocr_tesseract_eng() {
    let mut settings = Settings::default();
    settings.ocr.dst_ocr = OcrMode::Tesseract;
    settings.ocr.dst_ocr_lang = "eng".to_string();
    let args = settings.to_args();
    let idx = args
        .iter()
        .position(|a| a == "--ocr")
        .expect("missing --ocr");
    assert_eq!(args[idx + 1], "eng", "lang mismatch: {args:?}");
}

/// OcrMode::Tesseract + lang="chi_sim+eng" → --ocr chi_sim+eng（多语 join 不 escape）.
#[test]
fn roundtrip_ocr_chi_sim_plus_eng() {
    let mut settings = Settings::default();
    settings.ocr.dst_ocr = OcrMode::Tesseract;
    settings.ocr.dst_ocr_lang = "chi_sim+eng".to_string();
    let args = settings.to_args();
    let idx = args
        .iter()
        .position(|a| a == "--ocr")
        .expect("missing --ocr");
    assert_eq!(args[idx + 1], "chi_sim+eng", "lang mismatch: {args:?}");
}

/// v0.2 OcrMode::Mupdf 等同 off（M8+ 才真正支持 native PDF 文本提取），serialize 不输出 --ocr.
/// 与 docs/migration-from-c.md §6.2 OCR 路径 4 种语义一致.
#[test]
fn roundtrip_ocr_mupdf_no_output_in_v02() {
    let mut settings = Settings::default();
    settings.ocr.dst_ocr = OcrMode::Mupdf;
    settings.ocr.dst_ocr_lang.clear();
    assert_args_not_contain(&settings, &["--ocr"]);
}

/// OcrMode::Tesseract + 空 lang → serialize 层不输出 --ocr（保 minimal 语义）.
/// 运行时 lang::resolve（Step 9.4）会 fallback "eng" + warning, 但 serialize
/// 层不耦合运行时 fallback —— 若用户显式 --ocr eng 写到 settings, lang 已就位 ;
/// 若 Tesseract 状态在 settings 中残留 + 空 lang 则提示是配置错误而非默认状态.
#[test]
fn roundtrip_ocr_lang_empty_falls_back_eng() {
    let mut settings = Settings::default();
    settings.ocr.dst_ocr = OcrMode::Tesseract;
    settings.ocr.dst_ocr_lang.clear();
    assert_args_not_contain(&settings, &["--ocr"]);
}

/// 模拟 CLI `--ocr chi_sim+eng --echo-cmd` 等价: subcommands::cmd_echo_cmd
/// 内部就是 settings.to_args().join(" "), 故构造 Tesseract+lang 状态后调
/// to_args 验证 --ocr 出现.
#[test]
fn cli_echo_cmd_includes_ocr_flag() {
    let mut settings = Settings::default();
    settings.ocr.dst_ocr = OcrMode::Tesseract;
    settings.ocr.dst_ocr_lang = "chi_sim+eng".to_string();
    let args = settings.to_args();
    assert!(
        args.iter().any(|a| a == "--ocr"),
        "echo-cmd missing --ocr: {args:?}"
    );
    let idx = args.iter().position(|a| a == "--ocr").unwrap();
    assert_eq!(args[idx + 1], "chi_sim+eng");
}

/// 模拟 CLI `--ocr off --echo-cmd` 等价: dst_ocr=Off → to_args 不输出 --ocr.
#[test]
fn cli_echo_cmd_excludes_ocr_when_off() {
    let mut settings = Settings::default();
    settings.ocr.dst_ocr = OcrMode::Off;
    settings.ocr.dst_ocr_lang.clear();
    let args = settings.to_args();
    assert!(
        !args.iter().any(|a| a == "--ocr"),
        "Off 不应输出 --ocr: {args:?}"
    );
}

/// OCR 字段输出不应影响其他字段（verbose / text_wrap / pagelist 全保留）.
#[test]
fn roundtrip_does_not_lose_other_fields_with_ocr() {
    let mut settings = Settings::default();
    settings.ocr.dst_ocr = OcrMode::Tesseract;
    settings.ocr.dst_ocr_lang = "eng".to_string();
    settings.behavior.verbose = 2;
    settings.layout.text_wrap = TextWrap::Off;
    settings.behavior.pagelist = "1-5".to_string();
    let args = settings.to_args();
    assert!(args.iter().any(|a| a == "--ocr"), "--ocr missing: {args:?}");
    assert!(args.iter().any(|a| a == "eng"), "lang missing: {args:?}");
    assert!(
        args.iter().any(|a| a == "--no-wrap"),
        "--no-wrap missing: {args:?}"
    );
    assert!(args.iter().any(|a| a == "-p"), "-p missing: {args:?}");
    assert!(
        args.iter().any(|a| a == "1-5"),
        "pagelist missing: {args:?}"
    );
    let v_count = args.iter().filter(|a| *a == "-v").count();
    assert_eq!(v_count, 2, "-v count wrong: {args:?}");
}

// ─────────────────────────────────────────────────────────────────────
// Step 11.9 P0-6 - `--ocr-mode strict|partial|fallback` 反向序列化 (执行计划 #5 单测 4)
//
// 4 case 集中验证 serialize.rs:208-235 OcrStrictMode 输出逻辑：
//   - 默认 Fallback → 不输出（与 v0.1.0 兼容，最小输出）
//   - Strict       → 输出 `--ocr-mode strict`
//   - Partial      → 输出 `--ocr-mode partial`
//   - 与其他字段不互相干扰
// ─────────────────────────────────────────────────────────────────────

/// Default OcrStrictMode (Fallback) 不应触发 --ocr-mode 输出，保持 minimal 输出。
#[test]
fn roundtrip_ocr_mode_default_fallback_does_not_emit() {
    let settings = Settings::default();
    assert_args_not_contain(&settings, &["--ocr-mode"]);
}

/// OcrStrictMode::Strict 应输出 `--ocr-mode strict`。
#[test]
fn roundtrip_ocr_mode_strict_emits_flag() {
    let mut settings = Settings::default();
    settings.ocr.ocr_strict_mode = OcrStrictMode::Strict;
    let args = settings.to_args();
    let idx = args
        .iter()
        .position(|a| a == "--ocr-mode")
        .unwrap_or_else(|| panic!("--ocr-mode 缺失: {args:?}"));
    assert_eq!(
        args.get(idx + 1).map(String::as_str),
        Some("strict"),
        "--ocr-mode 后应紧跟 'strict': {args:?}"
    );
}

/// OcrStrictMode::Partial 应输出 `--ocr-mode partial`。
#[test]
fn roundtrip_ocr_mode_partial_emits_flag() {
    let mut settings = Settings::default();
    settings.ocr.ocr_strict_mode = OcrStrictMode::Partial;
    let args = settings.to_args();
    let idx = args
        .iter()
        .position(|a| a == "--ocr-mode")
        .unwrap_or_else(|| panic!("--ocr-mode 缺失: {args:?}"));
    assert_eq!(
        args.get(idx + 1).map(String::as_str),
        Some("partial"),
        "--ocr-mode 后应紧跟 'partial': {args:?}"
    );
}

/// OcrStrictMode 与 --ocr 共存：strict 模式 + Tesseract lang 同时输出 + 互不污染。
#[test]
fn roundtrip_ocr_mode_combines_with_ocr_lang() {
    let mut settings = Settings::default();
    settings.ocr.dst_ocr = OcrMode::Tesseract;
    settings.ocr.dst_ocr_lang = "chi_sim+eng".to_string();
    settings.ocr.ocr_strict_mode = OcrStrictMode::Strict;
    let args = settings.to_args();
    // 两 flag 都在
    assert!(args.iter().any(|a| a == "--ocr"), "--ocr 缺失: {args:?}");
    assert!(
        args.iter().any(|a| a == "chi_sim+eng"),
        "lang chi_sim+eng 缺失: {args:?}"
    );
    assert!(
        args.iter().any(|a| a == "--ocr-mode"),
        "--ocr-mode 缺失: {args:?}"
    );
    assert!(args.iter().any(|a| a == "strict"), "strict 缺失: {args:?}");
}

// ── Step 11.11 P1-2 / P1-4：visibility flags + min_confidence roundtrip ──

/// 默认 `dst_ocr_visibility_flags = SHOW_SOURCE` 应 **不** 输出 `--ocr-visibility-flags`
/// （minimal 输出 + 与 v0.1.0 行为兼容）。
#[test]
fn roundtrip_ocr_visibility_flags_default_does_not_emit() {
    let settings = Settings::default();
    let args = settings.to_args();
    assert!(
        !args.iter().any(|a| a == "--ocr-visibility-flags"),
        "默认 visibility 不应输出 flag: {args:?}"
    );
}

/// 非默认 `dst_ocr_visibility_flags = 7` (SHOW_SOURCE | SHOW_OCR_TEXT | SHOW_BOXES)
/// 应输出 `--ocr-visibility-flags 7`（与端到端验收命令字面一致）。
#[test]
fn roundtrip_ocr_visibility_flags_seven_emits_mask() {
    let mut settings = Settings::default();
    settings.ocr.dst_ocr_visibility_flags = OcrVisibility::from_bits(7);
    let args = settings.to_args();
    let idx = args
        .iter()
        .position(|a| a == "--ocr-visibility-flags")
        .unwrap_or_else(|| panic!("--ocr-visibility-flags 缺失: {args:?}"));
    assert_eq!(
        args.get(idx + 1).map(String::as_str),
        Some("7"),
        "--ocr-visibility-flags 后应紧跟 '7': {args:?}"
    );
}

/// 默认 `ocr_min_confidence = 0.0` 应 **不** 输出 `--ocr-min-confidence`
/// （minimal 输出 + 与 v0.1.0 行为兼容）。
#[test]
fn roundtrip_ocr_min_confidence_default_does_not_emit() {
    let settings = Settings::default();
    let args = settings.to_args();
    assert!(
        !args.iter().any(|a| a == "--ocr-min-confidence"),
        "默认 min_confidence 不应输出 flag: {args:?}"
    );
}

/// 非默认 `ocr_min_confidence = 0.5` 应输出 `--ocr-min-confidence 0.5`。
#[test]
fn roundtrip_ocr_min_confidence_half_emits_value() {
    let mut settings = Settings::default();
    settings.ocr.ocr_min_confidence = 0.5;
    let args = settings.to_args();
    let idx = args
        .iter()
        .position(|a| a == "--ocr-min-confidence")
        .unwrap_or_else(|| panic!("--ocr-min-confidence 缺失: {args:?}"));
    assert_eq!(
        args.get(idx + 1).map(String::as_str),
        Some("0.5"),
        "--ocr-min-confidence 后应紧跟 '0.5': {args:?}"
    );
}
