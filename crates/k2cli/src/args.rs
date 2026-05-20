//! CLI argument definitions using clap derive, and `From<CliArgs> for Settings` conversion.
//!
//! Source C: `k2parsecmd.c` (1628 lines) — M1 high-frequency parameters only.
//! See `rust-rewrite-plan.md` v2.1 Appendix A.

use clap::Parser;
use k2settings::device::find_by_alias;
use k2settings::source::MarginUnit;
use k2settings::Settings;

/// Reflow PDFs and images for e-reader screens.
#[derive(Parser, Debug, Clone)]
#[command(name = "k2pdfopt", version, about)]
#[command(after_long_help = crate::help::long_help())]
pub struct CliArgs {
    // ── Device & output ──────────────────────────────────────────
    /// Device profile name or alias (e.g. kv, kpw, ko2, kobo)
    #[arg(long = "dev", value_name = "PROFILE")]
    pub dev: Option<String>,

    /// Output file name format (%s = source name)
    #[arg(short = 'o', long = "output", value_name = "FMT")]
    pub output: Option<String>,

    // ── Page selection ───────────────────────────────────────────
    /// Page range to process (e.g. 1-10, 1,3,5, even, odd)
    #[arg(short = 'p', long = "pages", value_name = "RANGE")]
    pub pages: Option<String>,

    /// Page range to exclude
    #[arg(long = "px", value_name = "RANGE")]
    pub pages_exclude: Option<String>,

    // ── Margins ──────────────────────────────────────────────────
    /// Source crop margins: comma-separated L,T,R,B values (inches default).
    /// Single value applies to all four. Suffix: s=source, t=trimmed.
    #[arg(short = 'm', long = "margins", value_name = "M")]
    pub margins: Option<String>,

    /// Output margins (same format as -m)
    #[arg(long = "om", value_name = "M")]
    pub output_margins: Option<String>,

    // ── Boolean toggles ──────────────────────────────────────────
    /// Color output
    #[arg(long = "c")]
    pub color: bool,

    /// Disable color output
    #[arg(long = "no-c", conflicts_with = "color")]
    pub no_color: bool,

    /// Trim source margins
    #[arg(short = 't', long = "trim")]
    pub trim: bool,

    /// Disable trimming
    #[arg(long = "no-t", conflicts_with = "trim")]
    pub no_trim: bool,

    /// Fit columns to screen width
    #[arg(long = "fc")]
    pub fit_columns: bool,

    /// Disable fit-columns
    #[arg(long = "no-fc", conflicts_with = "fit_columns")]
    pub no_fit_columns: bool,

    // ── Text wrapping ────────────────────────────────────────────
    /// Enable text wrapping
    #[arg(long = "wrap")]
    pub wrap: bool,

    /// Extra text wrapping (C's -wrap+)
    #[arg(long = "wrap-extra")]
    pub wrap_extra: bool,

    /// Disable text wrapping
    #[arg(
        long = "no-wrap",
        conflicts_with_all = ["wrap", "wrap_extra"]
    )]
    pub no_wrap: bool,

    // ── Landscape ───────────────────────────────────────────────
    /// Landscape orientation
    #[arg(long = "ls")]
    pub ls: bool,

    /// Landscape mode for specific pages
    #[arg(long = "ls-pages", value_name = "PAGELIST")]
    pub ls_pages: Option<String>,

    /// Disable landscape
    #[arg(
        long = "no-ls",
        conflicts_with_all = ["ls", "ls_pages"]
    )]
    pub no_ls: bool,

    // ── Dimensions & DPI ────────────────────────────────────────
    /// Justification: 0=left, 1=center. Suffix + for full-justify, - for no.
    #[arg(short = 'j', long = "justify", value_name = "MODE")]
    pub justify: Option<String>,

    /// Output DPI (also sets input DPI)
    #[arg(long = "dpi", value_name = "N")]
    pub dpi: Option<i32>,

    /// Output DPI only
    #[arg(long = "odpi", value_name = "N")]
    pub odpi: Option<i32>,

    /// Output width with optional unit suffix (px/in/cm/s/t)
    #[arg(short = 'w', long = "width", value_name = "W")]
    pub width: Option<String>,

    /// Output height with optional unit suffix (px/in/cm/s/t)
    #[arg(long = "height", value_name = "H")]
    pub height: Option<String>,

    // ── Behavior ─────────────────────────────────────────────────
    /// Exit on complete (C's -x)
    #[arg(short = 'x', long = "exit")]
    pub exit: bool,

    /// Don't exit on complete
    #[arg(long = "no-x", conflicts_with = "exit")]
    pub no_exit: bool,

    /// Assume yes to all prompts
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,

    /// Don't assume yes
    #[arg(long = "no-y", conflicts_with = "yes")]
    pub no_yes: bool,

    /// Verbose output (repeat for more: -v, -vv, -vvv)
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Non-interactive mode (C's -ui-)
    #[arg(long = "ui-")]
    pub no_interactive: bool,

    /// Interactive mode (C's -ui)
    #[arg(long = "ui", conflicts_with = "no_interactive")]
    pub interactive: bool,

    // ── OCR (Step 9.3) ───────────────────────────────────────────
    /// OCR language(s) for tesseract (e.g. eng, chi_sim, chi_sim+eng).
    /// Use "off" to explicitly disable. When omitted, OCR stays off (default).
    #[arg(long = "ocr", value_name = "LANG")]
    pub ocr: Option<String>,

    // ── OCR strict mode (Step 11.9 P0-6) ─────────────────────────
    /// OCR 缺语言策略：strict=缺即报错 / partial=丢失保留命中 / fallback=自动落 eng
    /// (默认 = v0.1.0 行为)。
    #[arg(
        long = "ocr-mode",
        value_name = "MODE",
        value_parser = ["strict", "partial", "fallback"]
    )]
    pub ocr_mode: Option<String>,

    // ── OCR visibility / confidence (Step 11.11 P1-2 / P1-4) ─────
    /// OCR 输出可见性 bit mask（C `dst_ocr_visibility_flags`）。
    /// bit 0x01=show source bitmap / 0x02=show OCR text (Tr 3 invisible) /
    /// 0x04=show boxes / 0x08=use spaces / 0x10=optimized spaces。
    /// 默认 1 = SHOW_SOURCE。常用值：3 = source+text, 5 = source+boxes,
    /// 7 = source+text+boxes（端到端验收命令）。
    #[arg(
        long = "ocr-visibility-flags",
        value_name = "MASK",
        value_parser = clap::value_parser!(u8).range(0..=31)
    )]
    pub ocr_visibility_flags: Option<u8>,

    /// OCR word 置信度过滤阈值 [0.0, 1.0]。低于此值的 word 被丢弃。
    /// 默认 0.0 = 不过滤（与 v0.1.0 行为一致）。
    #[arg(long = "ocr-min-confidence", value_name = "F")]
    pub ocr_min_confidence: Option<f32>,

    // ── Reflow pipeline (Step 11.4) ──────────────────────────────
    /// Reflow pipeline mode: off=v0.1.0 直通 / auto=完整 figure+text reflow
    /// (默认) / force=即使是单列也跑完整 reflow.
    #[arg(
        long = "reflow",
        value_name = "MODE",
        value_parser = ["off", "auto", "force"]
    )]
    pub reflow: Option<String>,

    // ── Meta flags ───────────────────────────────────────────────
    /// List all device profiles
    #[arg(long = "list-devices")]
    pub list_devices: bool,

    /// Echo the equivalent command line
    #[arg(long = "echo-cmd")]
    pub echo_cmd: bool,

    /// Show conversion plan without processing
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Show compatibility report vs. C version
    #[arg(long = "compat-report")]
    pub compat_report: bool,

    // ── Input files ─────────────────────────────────────────────
    /// Input PDF/image files to process
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub files: Vec<String>,
}

/// Parse a dimension value with optional unit suffix.
///
/// Accepts: "600" (pixels), "6in" (inches), "15.24cm" (cm),
/// "0.5s" (source-relative), "0.5t" (trimmed-relative).
/// Returns (numeric_value, unit).
fn parse_value_with_units(input: &str) -> (f64, MarginUnit) {
    let input = input.trim();
    // Try to split at the boundary of digits/dots and letters
    let split_pos = input
        .char_indices()
        .find(|(_, c)| c.is_ascii_alphabetic())
        .map_or(input.len(), |(i, _)| i);

    let num_str = &input[..split_pos];
    let unit_str = &input[split_pos..];

    let value = num_str.parse::<f64>().unwrap_or(0.0);
    let unit = match unit_str.to_ascii_lowercase().as_str() {
        "in" | "i" => MarginUnit::Inches,
        "cm" | "c" => MarginUnit::Cm,
        "s" => MarginUnit::Source,
        "t" => MarginUnit::Trimmed,
        _ => MarginUnit::Pixels,
    };
    (value, unit)
}

/// Apply comma-separated margin values to a CropBox.
///
/// Format: "L,T,R,B" or "V" (applied to all four).
/// Each value can have a unit suffix (in/cm/s/t). Default unit is Inches.
/// For source margins, negative values (without explicit unit) mean Source units.
fn apply_margins(
    box_vals: &mut [f64; 4],
    units: &mut [MarginUnit; 4],
    input: &str,
    is_source: bool,
) {
    let parts: Vec<&str> = input.split(',').collect();
    for (i, part) in parts.iter().enumerate().take(4) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // Negative value without explicit unit → source-relative (C behavior)
        let (val, unit) = parse_value_with_units(part);
        if is_source && val < 0.0 && !part.contains(|c: char| c.is_ascii_alphabetic()) {
            box_vals[i] = val.abs();
            units[i] = MarginUnit::Source;
        } else {
            box_vals[i] = val;
            units[i] = unit;
        }
        // Fill remaining slots with the same value (C behavior: short form)
        for j in (i + 1)..4 {
            box_vals[j] = box_vals[i];
            units[j] = units[i];
        }
    }
}

/// Apply justification string to output settings.
///
/// C behavior: `-j 0` left, `-j 1` center, `-j -1` default.
/// Suffix `+` → full justify on, suffix `-` → full justify off.
fn apply_justify(settings: &mut Settings, input: &str) {
    let has_plus = input.contains('+');
    let has_minus = input.len() > 1 && input[1..].contains('-');

    // Strip trailing +/- modifiers before parsing the numeric value
    let num_str: String = input.trim_end_matches(['+', '-']).to_string();

    settings.output.dst_justify = num_str.parse::<i32>().unwrap_or(-1);

    if has_plus {
        settings.output.dst_fulljustify = 1;
    } else if has_minus {
        settings.output.dst_fulljustify = 0;
    }
}

impl From<CliArgs> for Settings {
    fn from(args: CliArgs) -> Self {
        let mut settings = Settings::default();

        // 1. Device profile — applied first so individual overrides can fine-tune
        if let Some(ref profile) = args.dev {
            if let Some(device) = find_by_alias(profile) {
                settings.apply_device_profile(device);
                settings.destination.devsize_set = 1;
                settings.destination.device_alias = Some(device.alias.to_string());
            }
        }

        // 2. Output format
        if let Some(ref output) = args.output {
            settings.output.dst_opname_format = output.clone();
        }

        // 3. Page selection
        if let Some(ref pages) = args.pages {
            settings.behavior.pagelist = pages.clone();
        }
        if let Some(ref px) = args.pages_exclude {
            settings.behavior.pagexlist = px.clone();
        }

        // 4. Margins
        if let Some(ref margins) = args.margins {
            apply_margins(
                &mut settings.source.srccropmargins.box_vals,
                &mut settings.source.srccropmargins.units,
                margins,
                true,
            );
        }
        if let Some(ref om) = args.output_margins {
            apply_margins(
                &mut settings.destination.dstmargins.box_vals,
                &mut settings.destination.dstmargins.units,
                om,
                false,
            );
        }

        // 5. Color toggle
        if args.no_color {
            settings.output.dst_color = 0;
        } else if args.color {
            settings.output.dst_color = 1;
        }

        // 6. Trim toggle
        if args.no_trim {
            settings.source.src_trim = false;
        } else if args.trim {
            settings.source.src_trim = true;
        }

        // 7. Fit columns toggle
        if args.no_fit_columns {
            settings.layout.fit_columns = false;
        } else if args.fit_columns {
            settings.layout.fit_columns = true;
        }

        // 8. Text wrapping (Off/On/Extra)
        if args.no_wrap {
            settings.layout.text_wrap = k2settings::TextWrap::Off;
        } else if args.wrap_extra {
            settings.layout.text_wrap = k2settings::TextWrap::Extra;
        } else if args.wrap {
            settings.layout.text_wrap = k2settings::TextWrap::On;
        }

        // 9. Landscape
        if args.no_ls {
            settings.destination.dst_landscape = false;
        } else if args.ls || args.ls_pages.is_some() {
            settings.destination.dst_landscape = true;
            if let Some(ref pages) = args.ls_pages {
                settings.destination.dst_landscape_pages = pages.clone();
            }
        }

        // 10. Justification
        if let Some(ref justify) = args.justify {
            apply_justify(&mut settings, justify);
        }

        // 11. DPI (both -dpi and -odpi set dst_userdpi + dst_dpi)
        if let Some(dpi) = args.dpi {
            settings.destination.dst_userdpi = dpi;
            settings.destination.dst_dpi = dpi;
            settings.behavior.user_mag |= 1;
        }
        if let Some(odpi) = args.odpi {
            settings.destination.dst_userdpi = odpi;
            settings.destination.dst_dpi = odpi;
            settings.behavior.user_mag |= 1;
        }

        // 12. Width / Height with unit parsing
        if let Some(ref width) = args.width {
            let (val, unit) = parse_value_with_units(width);
            settings.destination.dst_userwidth = val;
            settings.destination.dst_userwidth_units = unit as i32;
        }
        if let Some(ref height) = args.height {
            let (val, unit) = parse_value_with_units(height);
            settings.destination.dst_userheight = val;
            settings.destination.dst_userheight_units = unit as i32;
        }

        // 13. Exit on complete
        if args.no_exit {
            settings.behavior.exit_on_complete = 0;
        } else if args.exit {
            settings.behavior.exit_on_complete = 1;
        }

        // 14. Assume yes
        if args.no_yes {
            settings.behavior.assume_yes = false;
        } else if args.yes {
            settings.behavior.assume_yes = true;
        }

        // 15. Verbose (count: -v=1, -vv=2, -vvv=3, -vvvv=4)
        if args.verbose > 0 {
            settings.behavior.verbose = i32::from(args.verbose);
        }

        // 16. UI mode
        if args.no_interactive {
            settings.behavior.query_user = 0;
            settings.behavior.query_user_explicit = false;
        } else if args.interactive {
            settings.behavior.query_user = 1;
            settings.behavior.query_user_explicit = true;
        }

        // 17. OCR (Step 9.3) — `--ocr off` → Off；`--ocr <lang>` → Tesseract + lang
        if let Some(ref ocr) = args.ocr {
            let lower = ocr.to_ascii_lowercase();
            if lower == "off" || lower.is_empty() {
                settings.ocr.dst_ocr = k2settings::ocr::OcrMode::Off;
                settings.ocr.dst_ocr_lang.clear();
            } else {
                settings.ocr.dst_ocr = k2settings::ocr::OcrMode::Tesseract;
                settings.ocr.dst_ocr_lang = ocr.clone();
            }
        }

        // 17b. OCR strict mode (Step 11.9 P0-6) — `--ocr-mode strict|partial|fallback`
        // clap value_parser 已限定枚举值，from_arg 仅按字面构造，未知值不会到此（兜底回退默认）。
        if let Some(ref mode) = args.ocr_mode {
            if let Some(parsed) = k2settings::OcrStrictMode::from_arg(mode) {
                settings.ocr.ocr_strict_mode = parsed;
            }
        }

        // 17c. OCR visibility flags (Step 11.11 P1-2) — `--ocr-visibility-flags <MASK>`
        // clap value_parser 已限定到 0..=31（OcrVisibility::ALL_BITS_MAX），直接 from_bits。
        if let Some(bits) = args.ocr_visibility_flags {
            settings.ocr.dst_ocr_visibility_flags = k2settings::OcrVisibility::from_bits(bits);
        }

        // 17d. OCR min confidence (Step 11.11 P1-4) — `--ocr-min-confidence <F>`
        // 不做范围 clamp，clap 通过 value_parser 不限定（OcrPageInput::min_confidence
        // 内部 `(v * 100).clamp(0, 100)` 已兜底）。
        if let Some(min_conf) = args.ocr_min_confidence {
            settings.ocr.ocr_min_confidence = min_conf;
        }

        // 18. Reflow mode (Step 11.4) — `--reflow off|auto|force`
        if let Some(ref mode) = args.reflow {
            if let Some(parsed) = k2settings::ReflowMode::from_arg(mode) {
                settings.layout.reflow_mode = parsed;
            }
        }

        settings
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_units_pixels() {
        let (v, u) = parse_value_with_units("600");
        assert!((v - 600.0).abs() < f64::EPSILON);
        assert_eq!(u, MarginUnit::Pixels);
    }

    #[test]
    fn parse_units_inches() {
        let (v, u) = parse_value_with_units("0.5in");
        assert!((v - 0.5).abs() < f64::EPSILON);
        assert_eq!(u, MarginUnit::Inches);
    }

    #[test]
    fn parse_units_cm() {
        let (v, u) = parse_value_with_units("2.54cm");
        assert!((v - 2.54).abs() < f64::EPSILON);
        assert_eq!(u, MarginUnit::Cm);
    }

    #[test]
    fn parse_units_source() {
        let (v, u) = parse_value_with_units("0.5s");
        assert!((v - 0.5).abs() < f64::EPSILON);
        assert_eq!(u, MarginUnit::Source);
    }

    #[test]
    fn apply_margins_single_value() {
        let mut box_vals = [0.0; 4];
        let mut units = [MarginUnit::Inches; 4];
        apply_margins(&mut box_vals, &mut units, "0.5", true);
        assert!((box_vals[0] - 0.5).abs() < f64::EPSILON);
        // C behavior: single value fills all remaining
        assert!((box_vals[3] - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn apply_margins_negative_source() {
        let mut box_vals = [0.0; 4];
        let mut units = [MarginUnit::Inches; 4];
        apply_margins(&mut box_vals, &mut units, "-1", true);
        assert!((box_vals[0] - 1.0).abs() < f64::EPSILON);
        assert_eq!(units[0], MarginUnit::Source);
    }

    #[test]
    fn apply_margins_four_values() {
        let mut box_vals = [0.0; 4];
        let mut units = [MarginUnit::Inches; 4];
        apply_margins(&mut box_vals, &mut units, "0.1,0.2,0.3,0.4", false);
        assert!((box_vals[0] - 0.1).abs() < f64::EPSILON);
        assert!((box_vals[1] - 0.2).abs() < f64::EPSILON);
        assert!((box_vals[2] - 0.3).abs() < f64::EPSILON);
        assert!((box_vals[3] - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn justify_simple() {
        let mut s = Settings::default();
        apply_justify(&mut s, "0");
        assert_eq!(s.output.dst_justify, 0);
    }

    #[test]
    fn justify_with_plus() {
        let mut s = Settings::default();
        apply_justify(&mut s, "0+");
        assert_eq!(s.output.dst_justify, 0);
        assert_eq!(s.output.dst_fulljustify, 1);
    }

    #[test]
    fn justify_with_minus() {
        let mut s = Settings::default();
        apply_justify(&mut s, "1-");
        assert_eq!(s.output.dst_justify, 1);
        assert_eq!(s.output.dst_fulljustify, 0);
    }
}
