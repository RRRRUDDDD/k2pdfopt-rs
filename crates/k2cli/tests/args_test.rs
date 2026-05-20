//! Integration tests for k2cli::CliArgs — 20 high-frequency M1 parameters.
//!
//! Each test parses CLI args via clap, converts to Settings, and verifies
//! the expected Settings diff against default.

use clap::Parser;
use k2cli::CliArgs;
use k2settings::source::MarginUnit;
use k2settings::Settings;

fn parse(args: &[&str]) -> CliArgs {
    CliArgs::parse_from(args)
}

fn settings_from(args: &[&str]) -> Settings {
    Settings::from(parse(args))
}

// ── 1. --dev ────────────────────────────────────────────────────

#[test]
fn dev_applies_device_profile() {
    let s = settings_from(&["k2pdfopt", "--dev", "kpw"]);
    let default = Settings::default();
    // kpw: 658x889, dpi=212, color=0, mark_corners=1, padding=[0,0,3,4]
    assert_eq!(s.destination.dst_userwidth, 658.0);
    assert_eq!(s.destination.dst_userheight, 889.0);
    assert_eq!(s.destination.dst_dpi, 212);
    assert_eq!(s.destination.dst_userdpi, 212);
    assert_eq!(s.destination.devsize_set, 1);
    assert_eq!(s.output.dst_color, 0);
    // Verify non-device fields are still at defaults
    assert_eq!(s.behavior.verbose, default.behavior.verbose);
}

#[test]
fn dev_unknown_profile_keeps_defaults() {
    let s = settings_from(&["k2pdfopt", "--dev", "nonexistent_xyz"]);
    let default = Settings::default();
    // Unknown profile → settings unchanged
    assert_eq!(
        s.destination.dst_userwidth,
        default.destination.dst_userwidth
    );
}

// ── 2. -o / --output ────────────────────────────────────────────

#[test]
fn output_sets_opname_format() {
    let s = settings_from(&["k2pdfopt", "-o", "out_%s.pdf"]);
    assert_eq!(s.output.dst_opname_format, "out_%s.pdf");
}

// ── 3. -p / --pages ─────────────────────────────────────────────

#[test]
fn pages_sets_pagelist() {
    let s = settings_from(&["k2pdfopt", "-p", "1-10"]);
    assert_eq!(s.behavior.pagelist, "1-10");
}

#[test]
fn pages_long_form() {
    let s = settings_from(&["k2pdfopt", "--pages", "even"]);
    assert_eq!(s.behavior.pagelist, "even");
}

// ── 4. --px ─────────────────────────────────────────────────────

#[test]
fn px_sets_pagexlist() {
    let s = settings_from(&["k2pdfopt", "--px", "1,3"]);
    assert_eq!(s.behavior.pagexlist, "1,3");
}

// ── 5. -m / --margins ──────────────────────────────────────────

#[test]
fn margins_single_value() {
    let s = settings_from(&["k2pdfopt", "-m", "0.5"]);
    // Single value fills all four
    assert!((s.source.srccropmargins.box_vals[0] - 0.5).abs() < f64::EPSILON);
    assert!((s.source.srccropmargins.box_vals[3] - 0.5).abs() < f64::EPSILON);
}

#[test]
fn margins_four_values() {
    let s = settings_from(&["k2pdfopt", "-m", "0.1,0.2,0.3,0.4"]);
    assert!((s.source.srccropmargins.box_vals[0] - 0.1).abs() < f64::EPSILON);
    assert!((s.source.srccropmargins.box_vals[3] - 0.4).abs() < f64::EPSILON);
}

#[test]
fn margins_negative_source_units() {
    let s = settings_from(&["k2pdfopt", "-m", "-1"]);
    assert!((s.source.srccropmargins.box_vals[0] - 1.0).abs() < f64::EPSILON);
    assert_eq!(s.source.srccropmargins.units[0], MarginUnit::Source);
}

#[test]
fn margins_with_unit_suffix() {
    let s = settings_from(&["k2pdfopt", "-m", "1cm"]);
    assert!((s.source.srccropmargins.box_vals[0] - 1.0).abs() < f64::EPSILON);
    assert_eq!(s.source.srccropmargins.units[0], MarginUnit::Cm);
}

#[test]
fn output_margins_om() {
    let s = settings_from(&["k2pdfopt", "--om", "0.1,0.2,0.3,0.4"]);
    assert!((s.destination.dstmargins.box_vals[0] - 0.1).abs() < f64::EPSILON);
    assert!((s.destination.dstmargins.box_vals[3] - 0.4).abs() < f64::EPSILON);
}

// ── 6. --c / --no-c ─────────────────────────────────────────────

#[test]
fn color_flag() {
    let s = settings_from(&["k2pdfopt", "--c"]);
    assert_eq!(s.output.dst_color, 1);
}

#[test]
fn no_color_flag() {
    let s = settings_from(&["k2pdfopt", "--no-c"]);
    assert_eq!(s.output.dst_color, 0);
}

// ── 7. -t / --no-t ──────────────────────────────────────────────

#[test]
fn trim_flag() {
    let s = settings_from(&["k2pdfopt", "--no-t"]);
    assert!(!s.source.src_trim);
}

#[test]
fn trim_enable() {
    let default = Settings::default();
    let s = settings_from(&["k2pdfopt", "-t"]);
    // Default is already true, but explicit flag also sets it
    assert!(s.source.src_trim);
    assert_eq!(s.source.src_trim, default.source.src_trim);
}

// ── 8. --fc / --no-fc ───────────────────────────────────────────

#[test]
fn fit_columns_flag() {
    let s = settings_from(&["k2pdfopt", "--fc"]);
    assert!(s.layout.fit_columns);
}

#[test]
fn no_fit_columns_flag() {
    let s = settings_from(&["k2pdfopt", "--no-fc"]);
    assert!(!s.layout.fit_columns);
}

// ── 9. --wrap / --wrap-extra / --no-wrap ────────────────────────

#[test]
fn wrap_flag() {
    let s = settings_from(&["k2pdfopt", "--wrap"]);
    assert_eq!(s.layout.text_wrap, k2settings::TextWrap::On);
}

#[test]
fn wrap_extra_flag() {
    let s = settings_from(&["k2pdfopt", "--wrap-extra"]);
    assert_eq!(s.layout.text_wrap, k2settings::TextWrap::Extra);
}

#[test]
fn no_wrap_flag() {
    let s = settings_from(&["k2pdfopt", "--no-wrap"]);
    assert_eq!(s.layout.text_wrap, k2settings::TextWrap::Off);
}

// ── 10. --ls / --ls-pages / --no-ls ─────────────────────────────

#[test]
fn ls_flag() {
    let s = settings_from(&["k2pdfopt", "--ls"]);
    assert!(s.destination.dst_landscape);
    assert!(s.destination.dst_landscape_pages.is_empty());
}

#[test]
fn ls_with_pages() {
    let s = settings_from(&["k2pdfopt", "--ls-pages", "3-5"]);
    assert!(s.destination.dst_landscape);
    assert_eq!(s.destination.dst_landscape_pages, "3-5");
}

#[test]
fn no_ls_flag() {
    let s = settings_from(&["k2pdfopt", "--no-ls"]);
    assert!(!s.destination.dst_landscape);
}

// ── 11. -j / --justify ─────────────────────────────────────────

#[test]
fn justify_center() {
    let s = settings_from(&["k2pdfopt", "-j", "1"]);
    assert_eq!(s.output.dst_justify, 1);
}

#[test]
fn justify_with_full() {
    let s = settings_from(&["k2pdfopt", "-j", "0+"]);
    assert_eq!(s.output.dst_justify, 0);
    assert_eq!(s.output.dst_fulljustify, 1);
}

// ── 12. --dpi ───────────────────────────────────────────────────

#[test]
fn dpi_sets_both() {
    let s = settings_from(&["k2pdfopt", "--dpi", "300"]);
    assert_eq!(s.destination.dst_userdpi, 300);
    assert_eq!(s.destination.dst_dpi, 300);
    assert_eq!(s.behavior.user_mag & 1, 1);
}

// ── 13. --odpi ──────────────────────────────────────────────────

#[test]
fn odpi_sets_output_dpi() {
    let s = settings_from(&["k2pdfopt", "--odpi", "200"]);
    assert_eq!(s.destination.dst_userdpi, 200);
    assert_eq!(s.destination.dst_dpi, 200);
}

// ── 14. -w / -h ─────────────────────────────────────────────────

#[test]
fn width_pixels() {
    let s = settings_from(&["k2pdfopt", "-w", "600"]);
    assert!((s.destination.dst_userwidth - 600.0).abs() < f64::EPSILON);
    assert_eq!(s.destination.dst_userwidth_units, 0); // Pixels
}

#[test]
fn width_inches() {
    let s = settings_from(&["k2pdfopt", "-w", "6in"]);
    assert!((s.destination.dst_userwidth - 6.0).abs() < f64::EPSILON);
    assert_eq!(s.destination.dst_userwidth_units, 1); // Inches
}

#[test]
fn height_cm() {
    let s = settings_from(&["k2pdfopt", "--height", "15.24cm"]);
    assert!((s.destination.dst_userheight - 15.24).abs() < f64::EPSILON);
    assert_eq!(s.destination.dst_userheight_units, 2); // Cm
}

// ── 15. -x / --no-x ────────────────────────────────────────────

#[test]
fn exit_on_complete() {
    let s = settings_from(&["k2pdfopt", "-x"]);
    assert_eq!(s.behavior.exit_on_complete, 1);
}

#[test]
fn no_exit() {
    let s = settings_from(&["k2pdfopt", "--no-x"]);
    assert_eq!(s.behavior.exit_on_complete, 0);
}

// ── 16. -y / --no-y ─────────────────────────────────────────────

#[test]
fn assume_yes() {
    let s = settings_from(&["k2pdfopt", "-y"]);
    assert!(s.behavior.assume_yes);
}

#[test]
fn no_assume_yes() {
    let s = settings_from(&["k2pdfopt", "--no-y"]);
    assert!(!s.behavior.assume_yes);
}

// ── 17. -v ──────────────────────────────────────────────────────

#[test]
fn verbose_single() {
    let s = settings_from(&["k2pdfopt", "-v"]);
    assert_eq!(s.behavior.verbose, 1);
}

#[test]
fn verbose_double() {
    let s = settings_from(&["k2pdfopt", "-vv"]);
    assert_eq!(s.behavior.verbose, 2);
}

#[test]
fn verbose_triple() {
    let s = settings_from(&["k2pdfopt", "-vvv"]);
    assert_eq!(s.behavior.verbose, 3);
}

// ── 18. --ui- / --ui ────────────────────────────────────────────

#[test]
fn non_interactive() {
    let s = settings_from(&["k2pdfopt", "--ui-"]);
    assert_eq!(s.behavior.query_user, 0);
    assert!(!s.behavior.query_user_explicit);
}

#[test]
fn interactive() {
    let s = settings_from(&["k2pdfopt", "--ui"]);
    assert_eq!(s.behavior.query_user, 1);
    assert!(s.behavior.query_user_explicit);
}

// ── 19. Meta flags ──────────────────────────────────────────────

#[test]
fn list_devices_flag_parses() {
    let args = parse(&["k2pdfopt", "--list-devices"]);
    assert!(args.list_devices);
}

#[test]
fn echo_cmd_flag_parses() {
    let args = parse(&["k2pdfopt", "--echo-cmd"]);
    assert!(args.echo_cmd);
}

#[test]
fn dry_run_flag_parses() {
    let args = parse(&["k2pdfopt", "--dry-run"]);
    assert!(args.dry_run);
}

#[test]
fn compat_report_flag_parses() {
    let args = parse(&["k2pdfopt", "--compat-report"]);
    assert!(args.compat_report);
}

// ── 20. Combined / order-independence ───────────────────────────

#[test]
fn dev_then_overrides() {
    let s = settings_from(&["k2pdfopt", "--dev", "kpw", "--dpi", "300"]);
    // kpw sets dpi=212, then --dpi overrides to 300
    assert_eq!(s.destination.dst_dpi, 300);
    // But kpw's width/height should still be applied
    assert_eq!(s.destination.dst_userwidth, 658.0);
}

#[test]
fn multiple_overrides() {
    let s = settings_from(&[
        "k2pdfopt", "--dev", "kv", "--c", "-w", "800", "--height", "1000", "--wrap", "-v",
    ]);
    assert_eq!(s.destination.devsize_set, 1);
    assert_eq!(s.output.dst_color, 1);
    assert!((s.destination.dst_userwidth - 800.0).abs() < f64::EPSILON);
    assert!((s.destination.dst_userheight - 1000.0).abs() < f64::EPSILON);
    assert_eq!(s.layout.text_wrap, k2settings::TextWrap::On);
    assert_eq!(s.behavior.verbose, 1);
}

#[test]
fn trailing_files() {
    let args = parse(&["k2pdfopt", "--dev", "kv", "input.pdf", "second.pdf"]);
    assert_eq!(args.files, vec!["input.pdf", "second.pdf"]);
}

#[test]
fn default_settings_unchanged() {
    let s = settings_from(&["k2pdfopt"]);
    let default = Settings::default();
    assert_eq!(s, default);
}

// ---- Step 9.3 OCR flag ----

#[test]
fn ocr_none_keeps_default_mupdf_mode() {
    // 默认 OcrSettings::default() 是 OcrMode::Mupdf
    let s = settings_from(&["k2pdfopt"]);
    assert_eq!(s.ocr.dst_ocr, k2settings::ocr::OcrMode::Mupdf);
    assert_eq!(s.ocr.dst_ocr_lang, "");
}

#[test]
fn ocr_eng_sets_tesseract_and_lang() {
    let s = settings_from(&["k2pdfopt", "--ocr", "eng"]);
    assert_eq!(s.ocr.dst_ocr, k2settings::ocr::OcrMode::Tesseract);
    assert_eq!(s.ocr.dst_ocr_lang, "eng");
}

#[test]
fn ocr_compound_lang_passes_through() {
    let s = settings_from(&["k2pdfopt", "--ocr", "chi_sim+eng"]);
    assert_eq!(s.ocr.dst_ocr, k2settings::ocr::OcrMode::Tesseract);
    assert_eq!(s.ocr.dst_ocr_lang, "chi_sim+eng");
}

#[test]
fn ocr_off_disables() {
    let s = settings_from(&["k2pdfopt", "--ocr", "off"]);
    assert_eq!(s.ocr.dst_ocr, k2settings::ocr::OcrMode::Off);
    assert_eq!(s.ocr.dst_ocr_lang, "");
}

#[test]
fn ocr_off_case_insensitive() {
    let s = settings_from(&["k2pdfopt", "--ocr", "OFF"]);
    assert_eq!(s.ocr.dst_ocr, k2settings::ocr::OcrMode::Off);
}

#[test]
fn ocr_arg_parsed_into_args_field() {
    let args = parse(&["k2pdfopt", "--ocr", "eng"]);
    assert_eq!(args.ocr.as_deref(), Some("eng"));
}

// ---- Step 11.4 --reflow flag ----

#[test]
fn reflow_default_is_auto() {
    let s = settings_from(&["k2pdfopt"]);
    assert_eq!(s.layout.reflow_mode, k2settings::ReflowMode::Auto);
}

#[test]
fn reflow_off_sets_layout_mode() {
    let s = settings_from(&["k2pdfopt", "--reflow", "off"]);
    assert_eq!(s.layout.reflow_mode, k2settings::ReflowMode::Off);
}

#[test]
fn reflow_auto_sets_layout_mode() {
    let s = settings_from(&["k2pdfopt", "--reflow", "auto"]);
    assert_eq!(s.layout.reflow_mode, k2settings::ReflowMode::Auto);
}

#[test]
fn reflow_force_sets_layout_mode() {
    let s = settings_from(&["k2pdfopt", "--reflow", "force"]);
    assert_eq!(s.layout.reflow_mode, k2settings::ReflowMode::Force);
}

#[test]
fn reflow_arg_parsed_into_args_field() {
    let args = parse(&["k2pdfopt", "--reflow", "force"]);
    assert_eq!(args.reflow.as_deref(), Some("force"));
}

#[test]
fn reflow_invalid_value_rejected_by_clap() {
    // clap value_parser = ["off","auto","force"] 应该拒绝其他值
    let result = CliArgs::try_parse_from(["k2pdfopt", "--reflow", "turbo"]);
    assert!(result.is_err(), "clap 应拒绝未知 --reflow 值");
}
