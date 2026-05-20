//! Settings → CLI args reverse serialization.
//!
//! Source C: `k2settings2cmd.c` (928 lines) — `k2pdfopt_settings_get_cmdline()`.
//!
//! Only outputs M1 fields that differ from the comparison base.
//! If a device profile was applied (`device_alias` is set), the base is
//! `default + apply_device_profile(alias)`, so `--dev <alias>` is output
//! compactly. Otherwise the base is `Settings::default()`.

use crate::device::find_by_alias;
use crate::layout::TextWrap;
use crate::ocr::{OcrMode, OcrStrictMode};
use crate::source::MarginUnit;
use crate::Settings;

impl Settings {
    /// Convert settings to a CLI args vector, outputting only non-default fields.
    ///
    /// If `device_alias` is set, outputs `--dev <alias>` and compares remaining
    /// fields against the device-adjusted base. Otherwise compares against
    /// `Settings::default()`.
    ///
    /// Covers M1 high-frequency parameters only (Step 3.4 set).
    pub fn to_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        // Determine comparison base
        let device_base;
        let base: &Settings = if let Some(ref alias) = self.destination.device_alias {
            args.push("--dev".to_string());
            args.push(alias.clone());
            device_base = if let Some(device) = find_by_alias(alias) {
                let mut b = Settings::default();
                b.apply_device_profile(device);
                b
            } else {
                Settings::default()
            };
            &device_base
        } else {
            device_base = Settings::default();
            &device_base
        };

        self.push_overrides(&mut args, base);
        args
    }

    /// Push CLI args for fields that differ from `base`.
    fn push_overrides(&self, args: &mut Vec<String>, base: &Settings) {
        // Output format
        if self.output.dst_opname_format != base.output.dst_opname_format {
            args.push("-o".to_string());
            args.push(self.output.dst_opname_format.clone());
        }

        // Page selection
        if self.behavior.pagelist != base.behavior.pagelist {
            args.push("-p".to_string());
            args.push(self.behavior.pagelist.clone());
        }
        if self.behavior.pagexlist != base.behavior.pagexlist {
            args.push("--px".to_string());
            args.push(self.behavior.pagexlist.clone());
        }

        // Source margins
        if self.source.srccropmargins != base.source.srccropmargins {
            args.push("-m".to_string());
            args.push(format_cropbox(&self.source.srccropmargins));
        }

        // Output margins
        if self.destination.dstmargins != base.destination.dstmargins {
            args.push("--om".to_string());
            args.push(format_cropbox(&self.destination.dstmargins));
        }

        // Color
        if self.output.dst_color != base.output.dst_color {
            if self.output.dst_color > 0 {
                args.push("--c".to_string());
            } else {
                args.push("--no-c".to_string());
            }
        }

        // Trim
        if self.source.src_trim != base.source.src_trim {
            if self.source.src_trim {
                args.push("-t".to_string());
            } else {
                args.push("--no-t".to_string());
            }
        }

        // Fit columns
        if self.layout.fit_columns != base.layout.fit_columns {
            if self.layout.fit_columns {
                args.push("--fc".to_string());
            } else {
                args.push("--no-fc".to_string());
            }
        }

        // Text wrapping (tri-state)
        if self.layout.text_wrap != base.layout.text_wrap {
            match self.layout.text_wrap {
                TextWrap::Off => args.push("--no-wrap".to_string()),
                TextWrap::On => args.push("--wrap".to_string()),
                TextWrap::Extra => args.push("--wrap-extra".to_string()),
            }
        }

        // Reflow mode (Step 11.4)
        if self.layout.reflow_mode != base.layout.reflow_mode {
            args.push("--reflow".to_string());
            args.push(self.layout.reflow_mode.as_arg().to_string());
        }

        // Landscape
        let ls_changed = self.destination.dst_landscape != base.destination.dst_landscape
            || self.destination.dst_landscape_pages != base.destination.dst_landscape_pages;
        if ls_changed {
            if !self.destination.dst_landscape && self.destination.dst_landscape_pages.is_empty() {
                args.push("--no-ls".to_string());
            } else if !self.destination.dst_landscape_pages.is_empty() {
                args.push("--ls-pages".to_string());
                args.push(self.destination.dst_landscape_pages.clone());
            } else {
                args.push("--ls".to_string());
            }
        }

        // Justification + full-justify
        if self.output.dst_justify != base.output.dst_justify
            || self.output.dst_fulljustify != base.output.dst_fulljustify
        {
            args.push("-j".to_string());
            args.push(format_justify(
                self.output.dst_justify,
                self.output.dst_fulljustify,
            ));
        }

        // Output DPI (--odpi sets dst_userdpi + dst_dpi + user_mag)
        if self.destination.dst_userdpi != base.destination.dst_userdpi {
            args.push("--odpi".to_string());
            args.push(self.destination.dst_userdpi.to_string());
        }

        // Width with unit
        if (self.destination.dst_userwidth - base.destination.dst_userwidth).abs() > f64::EPSILON
            || self.destination.dst_userwidth_units != base.destination.dst_userwidth_units
        {
            args.push("-w".to_string());
            args.push(format_dimension(
                self.destination.dst_userwidth,
                self.destination.dst_userwidth_units,
            ));
        }

        // Height with unit
        if (self.destination.dst_userheight - base.destination.dst_userheight).abs() > f64::EPSILON
            || self.destination.dst_userheight_units != base.destination.dst_userheight_units
        {
            args.push("--height".to_string());
            args.push(format_dimension(
                self.destination.dst_userheight,
                self.destination.dst_userheight_units,
            ));
        }

        // Exit on complete
        if self.behavior.exit_on_complete != base.behavior.exit_on_complete {
            if self.behavior.exit_on_complete > 0 {
                args.push("-x".to_string());
            } else {
                args.push("--no-x".to_string());
            }
        }

        // Assume yes
        if self.behavior.assume_yes != base.behavior.assume_yes {
            if self.behavior.assume_yes {
                args.push("-y".to_string());
            } else {
                args.push("--no-y".to_string());
            }
        }

        // Verbose — output one -v per level (max 4)
        if self.behavior.verbose != base.behavior.verbose {
            for _ in 0..self.behavior.verbose.clamp(0, 4) {
                args.push("-v".to_string());
            }
        }

        // UI mode — compare query_user (default -1, --ui- sets 0, --ui sets 1)
        if self.behavior.query_user != base.behavior.query_user {
            if self.behavior.query_user == 0 {
                args.push("--ui-".to_string());
            } else if self.behavior.query_user > 0 {
                args.push("--ui".to_string());
            }
        }

        // OCR (Step 11.7 P0-4 / consumes Open Q 9.3.D + 9.4.H + 10.3.I).
        // 严格按 execution-plan §11.7 操作清单 #1 字面：仅 Tesseract 引擎 + 非空 lang 时输出 --ocr <LANG>。
        // - Off / Mupdf(v0.2 等同 off) / Tesseract+empty-lang 均不输出（运行时 lang::resolve 会 fallback eng,
        //   serialize 层不耦合运行时 fallback 保 minimal 输出语义）.
        // - 与 args.rs:413-422 CLI 解析对称：`--ocr off` / `--ocr ""` → Off + empty lang.
        if self.ocr.dst_ocr == OcrMode::Tesseract && !self.ocr.dst_ocr_lang.is_empty() {
            args.push("--ocr".to_string());
            args.push(self.ocr.dst_ocr_lang.clone());
        }

        // OCR strict mode (Step 11.9 P0-6 / consumes Open Q 9.4.F + 9.4.L).
        // 仅在 != base 时输出。base.ocr.ocr_strict_mode 默认为 Fallback，所以：
        // - 默认 Fallback 不输出（保持 minimal 输出 + 与 v0.1.0 行为一致）
        // - Strict / Partial 输出 `--ocr-mode strict|partial`
        // Fallback 分支理论上不会被命中（!= base 时只能是 Strict 或 Partial），
        // 但 match 写全保证 future-proof（若 base 被改变也能正确输出）。
        if self.ocr.ocr_strict_mode != base.ocr.ocr_strict_mode {
            match self.ocr.ocr_strict_mode {
                OcrStrictMode::Strict | OcrStrictMode::Partial => {
                    args.push("--ocr-mode".to_string());
                    args.push(self.ocr.ocr_strict_mode.as_arg().to_string());
                }
                OcrStrictMode::Fallback => {
                    args.push("--ocr-mode".to_string());
                    args.push("fallback".to_string());
                }
            }
        }

        // OCR visibility flags (Step 11.11 P1-2).
        // 仅在 != base 时输出（base = OcrVisibility::DEFAULT = SHOW_SOURCE = 1）。
        // 输出原始 bit mask 十进制（与 `--ocr-visibility-flags <MASK>` CLI 解析对偶）。
        if self.ocr.dst_ocr_visibility_flags != base.ocr.dst_ocr_visibility_flags {
            args.push("--ocr-visibility-flags".to_string());
            args.push(self.ocr.dst_ocr_visibility_flags.bits().to_string());
        }

        // OCR min confidence (Step 11.11 P1-4).
        // 仅在 != base 时输出（base.ocr.ocr_min_confidence = 0.0）。
        // 用 `f32::to_bits` 比较避免浮点 == 的 clippy lint，且与 OcrSettings PartialEq
        // 的语义一致（OcrSettings 派生 PartialEq，f32 默认按位比较 NaN 时仍 != self；
        // 这里仅排除 0.0 默认值，NaN 用户输入会被 clap 拒绝先于到达此处）。
        if self.ocr.ocr_min_confidence.to_bits() != base.ocr.ocr_min_confidence.to_bits() {
            args.push("--ocr-min-confidence".to_string());
            args.push(format!("{}", self.ocr.ocr_min_confidence));
        }
    }
}

// ── Formatting helpers ──────────────────────────────────────────────

/// Format a [`MarginUnit`] as a CLI suffix string.
fn unit_suffix(unit: MarginUnit) -> &'static str {
    match unit {
        MarginUnit::Inches => "in",
        MarginUnit::Cm => "cm",
        MarginUnit::Source => "s",
        MarginUnit::Trimmed => "t",
        MarginUnit::Pixels | MarginUnit::OcrLayer => "",
    }
}

/// Format a dimension value with its unit suffix.
fn format_dimension(val: f64, units_i32: i32) -> String {
    let unit = match units_i32 {
        1 => MarginUnit::Inches,
        2 => MarginUnit::Cm,
        3 => MarginUnit::Source,
        4 => MarginUnit::Trimmed,
        _ => MarginUnit::Pixels,
    };
    format!("{}{}", format_float_compact(val), unit_suffix(unit))
}

/// Format a [`CropBox`] as a CLI margin string.
///
/// Short form: `"0.5in"` when all four values and units are equal.
/// Long form: `"0.1in,0.2in,0.3in,0.4in"` otherwise.
fn format_cropbox(cb: &crate::source::CropBox) -> String {
    let all_vals_equal = cb.box_vals.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-10);
    let all_units_equal = cb.units.iter().all(|u| *u == cb.units[0]);
    if all_vals_equal && all_units_equal {
        format!(
            "{}{}",
            format_float_compact(cb.box_vals[0]),
            unit_suffix(cb.units[0])
        )
    } else {
        let parts: Vec<String> = (0..4)
            .map(|i| {
                format!(
                    "{}{}",
                    format_float_compact(cb.box_vals[i]),
                    unit_suffix(cb.units[i])
                )
            })
            .collect();
        parts.join(",")
    }
}

/// Format a float compactly: no trailing zeros, no unnecessary decimal point.
fn format_float_compact(val: f64) -> String {
    if (val - val.round()).abs() < 1e-10 {
        format!("{}", val.round() as i64)
    } else {
        let s = format!("{val:.6}");
        let s = s.trim_end_matches('0');
        let s = s.trim_end_matches('.');
        s.to_string()
    }
}

/// Format justification value with optional +/- full-justify modifier.
fn format_justify(justify: i32, fulljustify: i32) -> String {
    let mut s = justify.to_string();
    if fulljustify > 0 {
        s.push('+');
    } else if fulljustify == 0 {
        s.push('-');
    }
    s
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::layout::ReflowMode;
    use crate::source::CropBox;

    #[test]
    fn format_float_integer() {
        assert_eq!(format_float_compact(1.0), "1");
    }

    #[test]
    fn format_float_decimal() {
        assert_eq!(format_float_compact(0.5), "0.5");
    }

    #[test]
    fn format_float_small() {
        assert_eq!(format_float_compact(0.02), "0.02");
    }

    #[test]
    fn format_float_many_decimals() {
        assert_eq!(format_float_compact(0.333333), "0.333333");
    }

    #[test]
    fn format_cropbox_all_equal() {
        let cb = CropBox {
            box_vals: [0.5; 4],
            units: [MarginUnit::Inches; 4],
            ..CropBox::default()
        };
        assert_eq!(format_cropbox(&cb), "0.5in");
    }

    #[test]
    fn format_cropbox_different() {
        let cb = CropBox {
            box_vals: [0.1, 0.2, 0.3, 0.4],
            units: [MarginUnit::Inches; 4],
            ..CropBox::default()
        };
        assert_eq!(format_cropbox(&cb), "0.1in,0.2in,0.3in,0.4in");
    }

    #[test]
    fn format_cropbox_pixels() {
        let cb = CropBox {
            box_vals: [10.0; 4],
            units: [MarginUnit::Pixels; 4],
            ..CropBox::default()
        };
        assert_eq!(format_cropbox(&cb), "10");
    }

    #[test]
    fn format_dimension_pixels() {
        assert_eq!(format_dimension(600.0, 0), "600");
    }

    #[test]
    fn format_dimension_inches() {
        assert_eq!(format_dimension(6.0, 1), "6in");
    }

    #[test]
    fn format_dimension_cm() {
        assert_eq!(format_dimension(15.24, 2), "15.24cm");
    }

    #[test]
    fn format_justify_left() {
        assert_eq!(format_justify(0, -1), "0");
    }

    #[test]
    fn format_justify_center_full() {
        assert_eq!(format_justify(1, 1), "1+");
    }

    #[test]
    fn format_justify_left_no_full() {
        assert_eq!(format_justify(0, 0), "0-");
    }

    // ---- Step 11.4 --reflow round-trip ----

    #[test]
    fn reflow_default_auto_omitted() {
        // 默认 Auto，serialize 不应输出 --reflow
        let s = Settings::default();
        let args = s.to_args();
        assert!(
            !args.iter().any(|a| a == "--reflow"),
            "默认 ReflowMode::Auto 不应触发 --reflow 输出: {args:?}"
        );
    }

    #[test]
    fn reflow_off_emits_flag() {
        let mut s = Settings::default();
        s.layout.reflow_mode = ReflowMode::Off;
        let args = s.to_args();
        let mut iter = args.iter();
        assert!(
            iter.any(|a| a == "--reflow") && iter.next().is_some_and(|a| a == "off"),
            "ReflowMode::Off 应输出 --reflow off: {args:?}"
        );
    }

    #[test]
    fn reflow_force_emits_flag() {
        let mut s = Settings::default();
        s.layout.reflow_mode = ReflowMode::Force;
        let args = s.to_args();
        let mut iter = args.iter();
        assert!(
            iter.any(|a| a == "--reflow") && iter.next().is_some_and(|a| a == "force"),
            "ReflowMode::Force 应输出 --reflow force: {args:?}"
        );
    }
}
