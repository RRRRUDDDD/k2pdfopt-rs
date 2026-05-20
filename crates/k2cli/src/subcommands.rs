//! Subcommand implementations: --list-devices, --echo-cmd, --dry-run, --compat-report.
//!
//! Each function takes the relevant inputs and returns the output string or
//! performs the side effect. Separated from main.rs for testability.

use k2settings::device::list_devices;
use k2settings::Settings;

/// Execute `--list-devices`: output the device profile table.
pub fn cmd_list_devices() -> String {
    list_devices()
}

/// Execute `--echo-cmd`: serialize Settings back to CLI args.
pub fn cmd_echo_cmd(settings: &Settings) -> String {
    let args = settings.to_args();
    if args.is_empty() {
        "k2pdfopt-rs (default settings)".to_string()
    } else {
        format!("k2pdfopt-rs {}", args.join(" "))
    }
}

/// Execute `--dry-run`: output a ConvertJob JSON describing what would be processed.
///
/// The JSON includes: input files, settings summary, device info, page selection,
/// and a note that the pipeline is not yet implemented.
pub fn cmd_dry_run(settings: &Settings, files: &[String]) -> String {
    let device_info = if let Some(ref alias) = settings.destination.device_alias {
        format!(r#""device_alias": "{}""#, alias)
    } else {
        r#""device_alias": null"#.to_string()
    };

    let files_json: String = files
        .iter()
        .map(|f| format!("    {f:?}"))
        .collect::<Vec<_>>()
        .join(",\n");

    let pagelist = if settings.behavior.pagelist.is_empty() {
        "all".to_string()
    } else {
        settings.behavior.pagelist.clone()
    };

    let pagexlist = if settings.behavior.pagexlist.is_empty() {
        "none".to_string()
    } else {
        settings.behavior.pagexlist.clone()
    };

    let version = env!("CARGO_PKG_VERSION");

    format!(
        r#"{{
  "mode": "dry-run",
  "version": "{version}",
  "pipeline_status": "not_yet_implemented",
  {device_info},
  "output_width_px": {width},
  "output_height_px": {height},
  "output_dpi": {dpi},
  "input_files": [
{files_json}
  ],
  "page_selection": {{
    "pages": "{pagelist}",
    "exclude": "{pagexlist}"
  }},
  "settings_summary": {{
    "color": {color},
    "trim": {trim},
    "fit_columns": {fit_columns},
    "text_wrap": "{text_wrap}",
    "landscape": {landscape},
    "justify": {justify},
    "verbose": {verbose}
  }}
}}"#,
        width = settings.destination.dst_width,
        height = settings.destination.dst_height,
        dpi = settings.destination.dst_dpi,
        color = settings.output.dst_color,
        trim = settings.source.src_trim,
        fit_columns = settings.layout.fit_columns,
        text_wrap = match settings.layout.text_wrap {
            k2settings::TextWrap::Off => "off",
            k2settings::TextWrap::On => "on",
            k2settings::TextWrap::Extra => "extra",
        },
        landscape = settings.destination.dst_landscape,
        justify = settings.output.dst_justify,
        verbose = settings.behavior.verbose,
    )
}

/// Parameter compatibility status for --compat-report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatStatus {
    /// Parameter is supported with the same semantics.
    Supported,
    /// Parameter is not yet implemented.
    Unsupported,
    /// Parameter is supported but with different syntax or behavior.
    Different,
}

/// A single parameter entry in the compatibility matrix.
#[derive(Debug, Clone)]
pub struct CompatEntry {
    /// C version parameter (e.g. `-dev`).
    pub c_param: &'static str,
    /// Rust version parameter (e.g. `--dev`), or empty if unsupported.
    pub rs_param: &'static str,
    /// Compatibility status.
    pub status: CompatStatus,
    /// One-line note about differences.
    pub note: &'static str,
}

/// The full compatibility matrix: C v2.55 params vs. Rust release.
///
/// Listed in the same order as C's `k2usage.c` help output.
pub fn compat_matrix() -> Vec<CompatEntry> {
    vec![
        // ── Device & output ──
        CompatEntry {
            c_param: "-dev",
            rs_param: "--dev",
            status: CompatStatus::Different,
            note: "C uses multi-char short opt; Rust uses long opt only",
        },
        CompatEntry {
            c_param: "-o",
            rs_param: "-o/--output",
            status: CompatStatus::Supported,
            note: "",
        },
        // ── Page selection ──
        CompatEntry {
            c_param: "-p",
            rs_param: "-p/--pages",
            status: CompatStatus::Supported,
            note: "Rust parses e/o as prefix modifiers (improved over C)",
        },
        CompatEntry {
            c_param: "-px",
            rs_param: "--px",
            status: CompatStatus::Supported,
            note: "",
        },
        // ── Margins ──
        CompatEntry {
            c_param: "-m",
            rs_param: "-m/--margins",
            status: CompatStatus::Supported,
            note: "Unit suffix support: in/cm/s/t (same as C)",
        },
        CompatEntry {
            c_param: "-om",
            rs_param: "--om",
            status: CompatStatus::Supported,
            note: "",
        },
        // ── Boolean toggles ──
        CompatEntry {
            c_param: "-c/-c-",
            rs_param: "--c/--no-c",
            status: CompatStatus::Different,
            note: "C uses -flag- negation; Rust uses --no-flag",
        },
        CompatEntry {
            c_param: "-t/-t-",
            rs_param: "-t/--no-t",
            status: CompatStatus::Different,
            note: "C uses -flag- negation; Rust uses --no-flag",
        },
        CompatEntry {
            c_param: "-fc/-fc-",
            rs_param: "--fc/--no-fc",
            status: CompatStatus::Different,
            note: "C uses multi-char short + -flag- negation",
        },
        // ── Text wrapping ──
        CompatEntry {
            c_param: "-wrap/-wrap-/-wrap+",
            rs_param: "--wrap/--no-wrap/--wrap-extra",
            status: CompatStatus::Different,
            note: "C uses -flag+/--flag- syntax; Rust uses --wrap-extra",
        },
        // ── Landscape ──
        CompatEntry {
            c_param: "-ls/-ls-/-ls<pages>",
            rs_param: "--ls/--no-ls/--ls-pages",
            status: CompatStatus::Different,
            note: "C allows -ls3-5 compact syntax; Rust uses --ls-pages 3-5",
        },
        // ── Dimensions & DPI ──
        CompatEntry {
            c_param: "-j",
            rs_param: "-j/--justify",
            status: CompatStatus::Supported,
            note: "+/- full-justify suffix supported",
        },
        CompatEntry {
            c_param: "-dpi",
            rs_param: "--dpi",
            status: CompatStatus::Different,
            note: "C uses multi-char short opt; Rust uses long opt only",
        },
        CompatEntry {
            c_param: "-odpi",
            rs_param: "--odpi",
            status: CompatStatus::Different,
            note: "C uses multi-char short opt; Rust uses long opt only",
        },
        CompatEntry {
            c_param: "-w",
            rs_param: "-w/--width",
            status: CompatStatus::Supported,
            note: "Unit suffix supported: px/in/cm/s/t",
        },
        CompatEntry {
            c_param: "-h",
            rs_param: "--height",
            status: CompatStatus::Different,
            note: "C uses -h (conflicts with --help); Rust uses --height only",
        },
        // ── Behavior ──
        CompatEntry {
            c_param: "-x/-x-",
            rs_param: "-x/--no-x",
            status: CompatStatus::Different,
            note: "C uses -flag- negation; Rust uses --no-flag",
        },
        CompatEntry {
            c_param: "-y/-y-",
            rs_param: "-y/--no-y",
            status: CompatStatus::Different,
            note: "C uses -flag- negation; Rust uses --no-flag",
        },
        CompatEntry {
            c_param: "-v",
            rs_param: "-v/--verbose",
            status: CompatStatus::Supported,
            note: "Rust supports -vvvv (4 levels); C only has 0/1",
        },
        CompatEntry {
            c_param: "-ui-/-ui",
            rs_param: "--ui-/--ui",
            status: CompatStatus::Supported,
            note: "",
        },
        // ── Meta ──
        CompatEntry {
            c_param: "(none)",
            rs_param: "--list-devices",
            status: CompatStatus::Supported,
            note: "C uses -dev ?; Rust has dedicated flag",
        },
        CompatEntry {
            c_param: "(none)",
            rs_param: "--echo-cmd",
            status: CompatStatus::Supported,
            note: "Rust-only feature (C has k2settings2cmd.c internally)",
        },
        CompatEntry {
            c_param: "(none)",
            rs_param: "--dry-run",
            status: CompatStatus::Supported,
            note: "Rust-only feature",
        },
        CompatEntry {
            c_param: "(none)",
            rs_param: "--compat-report",
            status: CompatStatus::Supported,
            note: "Rust-only feature",
        },
        // ── Not yet implemented (M2+) ──
        CompatEntry {
            c_param: "-?[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Pattern search in help text (M2+)",
        },
        CompatEntry {
            c_param: "-a[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "ANSI color toggle (M2+)",
        },
        CompatEntry {
            c_param: "-ac[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Auto crop (M2+)",
        },
        CompatEntry {
            c_param: "-as[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Auto straighten (M2+)",
        },
        CompatEntry {
            c_param: "-author",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "PDF author metadata (M2+)",
        },
        CompatEntry {
            c_param: "-bmp[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Bitmap output (M2+)",
        },
        CompatEntry {
            c_param: "-bp[+|-|--]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Page break control (M2+)",
        },
        CompatEntry {
            c_param: "-bpl",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Page break list (M2+)",
        },
        CompatEntry {
            c_param: "-bpm",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Page break marks (M2+)",
        },
        CompatEntry {
            c_param: "-cbox",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Crop boxes (M2+)",
        },
        CompatEntry {
            c_param: "-cg/-cgmax/-cgr/-ch",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Column gap/height settings (M2+)",
        },
        CompatEntry {
            c_param: "-ci[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Cover image (M2+)",
        },
        CompatEntry {
            c_param: "-col",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Max columns (M2+)",
        },
        CompatEntry {
            c_param: "-colorbg/-colorfg",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Color mapping (M3+)",
        },
        CompatEntry {
            c_param: "-n[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Native PDF output (M3+)",
        },
        CompatEntry {
            c_param: "-ocr[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "OCR via tesseract (M3+)",
        },
        CompatEntry {
            c_param: "-mode",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Processing mode (M2+)",
        },
        CompatEntry {
            c_param: "-toc[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Table of contents (M2+)",
        },
        CompatEntry {
            c_param: "-evl/-ehl",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Erase lines (M2+)",
        },
        CompatEntry {
            c_param: "-er/-de/-g/-cmax",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Image processing filters (M2+)",
        },
        CompatEntry {
            c_param: "-dr/-ds",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Display resolution / doc size (M2+)",
        },
        CompatEntry {
            c_param: "-r",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "RTL column order (M2+)",
        },
        CompatEntry {
            c_param: "-go",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Column display order (M2+)",
        },
        CompatEntry {
            c_param: "-s[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Special region detection (M2+)",
        },
        CompatEntry {
            c_param: "-sm[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Show marked regions (M2+)",
        },
        CompatEntry {
            c_param: "-indent",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Indent detection (M2+)",
        },
        CompatEntry {
            c_param: "-f2p",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Fit-to-page (M2+)",
        },
        CompatEntry {
            c_param: "-odc",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Output doc cuts (M2+)",
        },
        CompatEntry {
            c_param: "-minl/-maxl",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Min/max output lines per page (M2+)",
        },
        CompatEntry {
            c_param: "-bpc",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Bits per color (M2+)",
        },
        CompatEntry {
            c_param: "-d[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Dithering (M2+)",
        },
        CompatEntry {
            c_param: "-jpg[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "JPEG output (M3+)",
        },
        CompatEntry {
            c_param: "-rt[-]",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Rotate source (M2+)",
        },
        CompatEntry {
            c_param: "-sdk",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "GUI SDK mode — not planned for CLI",
        },
        CompatEntry {
            c_param: "-gs",
            rs_param: "",
            status: CompatStatus::Unsupported,
            note: "Ghostscript — replaced by mutool (ADR-015)",
        },
    ]
}

/// Execute `--compat-report`: output parameter compatibility report.
///
/// If `filter` is provided (e.g. from `--dev`), only show entries relevant
/// to the current settings context.
pub fn cmd_compat_report(settings: &Settings) -> String {
    let matrix = compat_matrix();
    let mut supported = Vec::new();
    let mut different = Vec::new();
    let mut unsupported = Vec::new();

    for entry in &matrix {
        match entry.status {
            CompatStatus::Supported => supported.push(entry),
            CompatStatus::Different => different.push(entry),
            CompatStatus::Unsupported => unsupported.push(entry),
        }
    }

    let mut out = String::new();

    out.push_str(&format!(
        "k2pdfopt-rs v{} Compatibility Report (vs. C v2.55)\n",
        env!("CARGO_PKG_VERSION")
    ));
    out.push_str("=====================================================\n\n");

    if let Some(ref alias) = settings.destination.device_alias {
        out.push_str(&format!("Active device: {alias}\n\n"));
    }

    // Supported
    out.push_str(&format!("SUPPORTED ({} params)\n", supported.len()));
    out.push_str("----------------------\n");
    for e in &supported {
        out.push_str(&format!("  {:<28} -> {:<20}\n", e.c_param, e.rs_param));
        if !e.note.is_empty() {
            out.push_str(&format!("    Note: {}\n", e.note));
        }
    }
    out.push('\n');

    // Different
    out.push_str(&format!(
        "DIFFERENT syntax/behavior ({} params)\n",
        different.len()
    ));
    out.push_str("---------------------------------------\n");
    for e in &different {
        out.push_str(&format!("  {:<28} -> {:<20}\n", e.c_param, e.rs_param));
        if !e.note.is_empty() {
            out.push_str(&format!("    Note: {}\n", e.note));
        }
    }
    out.push('\n');

    // Unsupported
    out.push_str(&format!(
        "NOT YET IMPLEMENTED ({} params)\n",
        unsupported.len()
    ));
    out.push_str("---------------------------------\n");
    for e in &unsupported {
        out.push_str(&format!("  {:<28}  ({})\n", e.c_param, e.note));
    }
    out.push('\n');

    // Summary
    let total = matrix.len();
    let pct_supported = (supported.len() as f64 / total as f64 * 100.0).round() as u32;
    let pct_different = (different.len() as f64 / total as f64 * 100.0).round() as u32;
    let pct_unsupported = (unsupported.len() as f64 / total as f64 * 100.0).round() as u32;
    out.push_str(&format!(
        "Summary: {total} params total — {pct_supported}% supported, {pct_different}% different, {pct_unsupported}% not yet implemented\n"
    ));

    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn list_devices_not_empty() {
        let out = cmd_list_devices();
        assert!(out.contains("Kindle Paperwhite"));
        assert!(out.contains("kpw"));
    }

    #[test]
    fn echo_cmd_default() {
        let settings = Settings::default();
        let out = cmd_echo_cmd(&settings);
        assert!(out.contains("default settings"));
    }

    #[test]
    fn echo_cmd_with_dev() {
        let mut settings = Settings::default();
        let dp = k2settings::device::find_by_alias("kpw").unwrap();
        settings.apply_device_profile(dp);
        settings.destination.device_alias = Some("kpw".to_string());
        let out = cmd_echo_cmd(&settings);
        assert!(out.contains("--dev"));
        assert!(out.contains("kpw"));
    }

    #[test]
    fn dry_run_basic() {
        let settings = Settings::default();
        let files = vec!["test.pdf".to_string()];
        let out = cmd_dry_run(&settings, &files);
        assert!(out.contains("dry-run"));
        assert!(out.contains("test.pdf"));
        assert!(out.contains("not_yet_implemented"));
    }

    #[test]
    fn dry_run_with_pages() {
        let mut settings = Settings::default();
        settings.behavior.pagelist = "1-3".to_string();
        let out = cmd_dry_run(&settings, &[]);
        assert!(out.contains("1-3"));
    }

    #[test]
    fn compat_report_counts() {
        let matrix = compat_matrix();
        let supported = matrix
            .iter()
            .filter(|e| e.status == CompatStatus::Supported)
            .count();
        let different = matrix
            .iter()
            .filter(|e| e.status == CompatStatus::Different)
            .count();
        let unsupported = matrix
            .iter()
            .filter(|e| e.status == CompatStatus::Unsupported)
            .count();
        assert!(supported > 0, "must have some supported params");
        assert!(different > 0, "must have some different params");
        assert!(unsupported > supported, "most C params not yet in M1");
        assert_eq!(supported + different + unsupported, matrix.len());
    }

    #[test]
    fn compat_report_output() {
        let settings = Settings::default();
        let out = cmd_compat_report(&settings);
        assert!(out.contains("Compatibility Report"));
        assert!(out.contains("SUPPORTED"));
        assert!(out.contains("DIFFERENT"));
        assert!(out.contains("NOT YET IMPLEMENTED"));
        assert!(out.contains("Summary:"));
    }
}
