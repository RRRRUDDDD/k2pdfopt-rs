//! K2PDFOPT environment variable parsing and merge with CLI args.
//!
//! Source C: `k2pdfopt.c:109-114` (env var read) + `k2parsecmd.c:165-172` (concat & parse).
//!
//! Merge priority: defaults < env < CLI (later wins).
//!
//! C version concatenates env string before cmdline string and parses left-to-right.
//! Rust version parses env and CLI separately, then merges field-by-field with
//! CLI taking priority for explicitly-set fields.

use crate::CliArgs;
use clap::Parser;

/// Parse the `K2PDFOPT` environment variable into [`CliArgs`].
///
/// Returns `None` if the variable is unset or empty.
/// Uses [`shell_words::split`] for POSIX-compatible tokenization,
/// which handles double-quoted strings and backslash escapes
/// (stricter than C's simple whitespace-split tokenizer).
pub fn parse_env() -> Option<CliArgs> {
    let env_str = std::env::var("K2PDFOPT").ok()?;
    if env_str.trim().is_empty() {
        return None;
    }
    let tokens = shell_words::split(&env_str).ok()?;
    if tokens.is_empty() {
        return None;
    }
    Some(CliArgs::parse_from(
        std::iter::once("k2pdfopt-env").chain(tokens.iter().map(String::as_str)),
    ))
}

/// Merge env-derived [`CliArgs`] with CLI-derived [`CliArgs`].
///
/// For each field:
/// - `Option<T>`: CLI `Some` overrides env; env `Some` is fallback.
/// - Bool flag pairs (e.g. `color`/`no_color`): if either CLI flag is `true`,
///   the CLI wins for that pair; otherwise env values propagate.
/// - Tri-state flags (e.g. `wrap`/`wrap_extra`/`no_wrap`): same logic.
/// - `ArgAction::Count` (`verbose`): CLI wins if > 0.
/// - `Vec<String>` (`files`): CLI wins if non-empty.
/// - Meta flags (`list_devices`, `echo_cmd`, etc.): OR-combined (both sources
///   can enable these action flags).
pub fn merge_env_and_cli(env: CliArgs, cli: CliArgs) -> CliArgs {
    // Capture before moves
    let cli_has_ls_pages = cli.ls_pages.is_some();

    CliArgs {
        // Option<T>: CLI overrides, env fallback
        dev: cli.dev.or(env.dev),
        output: cli.output.or(env.output),
        pages: cli.pages.or(env.pages),
        pages_exclude: cli.pages_exclude.or(env.pages_exclude),
        margins: cli.margins.or(env.margins),
        output_margins: cli.output_margins.or(env.output_margins),
        ls_pages: cli.ls_pages.or(env.ls_pages),
        justify: cli.justify.or(env.justify),
        dpi: cli.dpi.or(env.dpi),
        odpi: cli.odpi.or(env.odpi),
        width: cli.width.or(env.width),
        height: cli.height.or(env.height),

        // Bool pair: color / no_color
        color: if cli.color || cli.no_color {
            cli.color
        } else {
            env.color
        },
        no_color: if cli.color || cli.no_color {
            cli.no_color
        } else {
            env.no_color
        },

        // Bool pair: trim / no_trim
        trim: if cli.trim || cli.no_trim {
            cli.trim
        } else {
            env.trim
        },
        no_trim: if cli.trim || cli.no_trim {
            cli.no_trim
        } else {
            env.no_trim
        },

        // Bool pair: fit_columns / no_fit_columns
        fit_columns: if cli.fit_columns || cli.no_fit_columns {
            cli.fit_columns
        } else {
            env.fit_columns
        },
        no_fit_columns: if cli.fit_columns || cli.no_fit_columns {
            cli.no_fit_columns
        } else {
            env.no_fit_columns
        },

        // Tri-state: wrap / wrap_extra / no_wrap
        wrap: if cli.wrap || cli.wrap_extra || cli.no_wrap {
            cli.wrap
        } else {
            env.wrap
        },
        wrap_extra: if cli.wrap || cli.wrap_extra || cli.no_wrap {
            cli.wrap_extra
        } else {
            env.wrap_extra
        },
        no_wrap: if cli.wrap || cli.wrap_extra || cli.no_wrap {
            cli.no_wrap
        } else {
            env.no_wrap
        },

        // Landscape: ls / no_ls / ls_pages
        ls: if cli.ls || cli.no_ls || cli_has_ls_pages {
            cli.ls
        } else {
            env.ls
        },
        no_ls: if cli.ls || cli.no_ls || cli_has_ls_pages {
            cli.no_ls
        } else {
            env.no_ls
        },

        // Bool pair: exit / no_exit
        exit: if cli.exit || cli.no_exit {
            cli.exit
        } else {
            env.exit
        },
        no_exit: if cli.exit || cli.no_exit {
            cli.no_exit
        } else {
            env.no_exit
        },

        // Bool pair: yes / no_yes
        yes: if cli.yes || cli.no_yes {
            cli.yes
        } else {
            env.yes
        },
        no_yes: if cli.yes || cli.no_yes {
            cli.no_yes
        } else {
            env.no_yes
        },

        // Count: verbose — CLI wins if > 0
        verbose: if cli.verbose > 0 {
            cli.verbose
        } else {
            env.verbose
        },

        // Bool pair: no_interactive / interactive
        no_interactive: if cli.no_interactive || cli.interactive {
            cli.no_interactive
        } else {
            env.no_interactive
        },
        interactive: if cli.no_interactive || cli.interactive {
            cli.interactive
        } else {
            env.interactive
        },

        // Meta flags: OR-combined — either source can trigger the action
        list_devices: cli.list_devices || env.list_devices,
        echo_cmd: cli.echo_cmd || env.echo_cmd,
        dry_run: cli.dry_run || env.dry_run,
        compat_report: cli.compat_report || env.compat_report,

        // OCR (Step 9.3): CLI overrides, env fallback (Option<String>)
        ocr: cli.ocr.or(env.ocr),

        // OCR strict mode (Step 11.9 P0-6): CLI overrides, env fallback (Option<String>)
        ocr_mode: cli.ocr_mode.or(env.ocr_mode),

        // OCR visibility flags (Step 11.11 P1-2): CLI overrides, env fallback (Option<u8>)
        ocr_visibility_flags: cli.ocr_visibility_flags.or(env.ocr_visibility_flags),

        // OCR min confidence (Step 11.11 P1-4): CLI overrides, env fallback (Option<f32>)
        ocr_min_confidence: cli.ocr_min_confidence.or(env.ocr_min_confidence),

        // Reflow mode (Step 11.4): CLI overrides, env fallback (Option<String>)
        reflow: cli.reflow.or(env.reflow),

        // Files: CLI wins if non-empty
        files: if cli.files.is_empty() {
            env.files
        } else {
            cli.files
        },
    }
}

/// Build [`k2settings::Settings`] from both the `K2PDFOPT` env var and CLI args,
/// applying the correct priority: defaults < env < CLI.
pub fn build_settings() -> k2settings::Settings {
    let cli = CliArgs::parse();
    match parse_env() {
        Some(env) => k2settings::Settings::from(merge_env_and_cli(env, cli)),
        None => k2settings::Settings::from(cli),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn merge_option_cli_overrides() {
        let env = CliArgs::parse_from(["k2pdfopt", "--dev", "kv"]);
        let cli = CliArgs::parse_from(["k2pdfopt", "--dev", "kpw"]);
        let merged = merge_env_and_cli(env, cli);
        assert_eq!(merged.dev.as_deref(), Some("kpw"));
    }

    #[test]
    fn merge_option_env_fallback() {
        let env = CliArgs::parse_from(["k2pdfopt", "--dev", "kv"]);
        let cli = CliArgs::parse_from(["k2pdfopt"]);
        let merged = merge_env_and_cli(env, cli);
        assert_eq!(merged.dev.as_deref(), Some("kv"));
    }

    #[test]
    fn merge_bool_pair_cli_overrides() {
        let env = CliArgs::parse_from(["k2pdfopt", "--c"]);
        let cli = CliArgs::parse_from(["k2pdfopt", "--no-c"]);
        let merged = merge_env_and_cli(env, cli);
        assert!(!merged.color);
        assert!(merged.no_color);
    }

    #[test]
    fn merge_bool_pair_env_fallback() {
        let env = CliArgs::parse_from(["k2pdfopt", "--c"]);
        let cli = CliArgs::parse_from(["k2pdfopt"]);
        let merged = merge_env_and_cli(env, cli);
        assert!(merged.color);
        assert!(!merged.no_color);
    }

    #[test]
    fn merge_bool_pair_neither_set() {
        let env = CliArgs::parse_from(["k2pdfopt"]);
        let cli = CliArgs::parse_from(["k2pdfopt"]);
        let merged = merge_env_and_cli(env, cli);
        assert!(!merged.color);
        assert!(!merged.no_color);
    }

    #[test]
    fn merge_tristate_wrap_extra() {
        let env = CliArgs::parse_from(["k2pdfopt", "--wrap"]);
        let cli = CliArgs::parse_from(["k2pdfopt", "--wrap-extra"]);
        let merged = merge_env_and_cli(env, cli);
        assert!(!merged.wrap);
        assert!(merged.wrap_extra);
        assert!(!merged.no_wrap);
    }

    #[test]
    fn merge_tristate_env_fallback() {
        let env = CliArgs::parse_from(["k2pdfopt", "--no-wrap"]);
        let cli = CliArgs::parse_from(["k2pdfopt"]);
        let merged = merge_env_and_cli(env, cli);
        assert!(merged.no_wrap);
    }

    #[test]
    fn merge_verbose_cli_wins() {
        let env = CliArgs::parse_from(["k2pdfopt", "-vvv"]);
        let cli = CliArgs::parse_from(["k2pdfopt", "-v"]);
        let merged = merge_env_and_cli(env, cli);
        assert_eq!(merged.verbose, 1);
    }

    #[test]
    fn merge_verbose_env_fallback() {
        let env = CliArgs::parse_from(["k2pdfopt", "-vv"]);
        let cli = CliArgs::parse_from(["k2pdfopt"]);
        let merged = merge_env_and_cli(env, cli);
        assert_eq!(merged.verbose, 2);
    }

    #[test]
    fn merge_files_cli_wins() {
        let env = CliArgs::parse_from(["k2pdfopt", "env.pdf"]);
        let cli = CliArgs::parse_from(["k2pdfopt", "cli.pdf"]);
        let merged = merge_env_and_cli(env, cli);
        assert_eq!(merged.files, vec!["cli.pdf"]);
    }

    #[test]
    fn merge_files_env_fallback() {
        let env = CliArgs::parse_from(["k2pdfopt", "env.pdf"]);
        let cli = CliArgs::parse_from(["k2pdfopt"]);
        let merged = merge_env_and_cli(env, cli);
        assert_eq!(merged.files, vec!["env.pdf"]);
    }

    #[test]
    fn merge_multiple_env_fields() {
        let env = CliArgs::parse_from(["k2pdfopt", "--dev", "kpw", "-p", "1-5", "--c"]);
        let cli = CliArgs::parse_from(["k2pdfopt", "--no-c"]);
        let merged = merge_env_and_cli(env, cli);
        assert_eq!(merged.dev.as_deref(), Some("kpw"));
        assert_eq!(merged.pages.as_deref(), Some("1-5"));
        assert!(!merged.color);
        assert!(merged.no_color);
    }

    #[test]
    fn merge_landscape_with_ls_pages() {
        let env = CliArgs::parse_from(["k2pdfopt", "--ls"]);
        let cli = CliArgs::parse_from(["k2pdfopt", "--ls-pages", "3-5"]);
        let merged = merge_env_and_cli(env, cli);
        assert!(!merged.ls);
        assert_eq!(merged.ls_pages.as_deref(), Some("3-5"));
    }

    #[test]
    fn env_roundtrip_via_settings() {
        // env: --dev kv -v ; cli: -p 1-3
        let env = CliArgs::parse_from(["k2pdfopt", "--dev", "kv", "-v"]);
        let cli = CliArgs::parse_from(["k2pdfopt", "-p", "1-3"]);
        let merged = merge_env_and_cli(env, cli);
        let settings = k2settings::Settings::from(merged);
        assert_eq!(settings.destination.device_alias.as_deref(), Some("kv"));
        assert_eq!(settings.behavior.pagelist, "1-3");
        assert_eq!(settings.behavior.verbose, 1);
    }
}
