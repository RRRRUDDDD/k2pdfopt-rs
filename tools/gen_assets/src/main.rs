//! Step 10.2 — Generate shell completions and roff man page from `k2cli::CliArgs`.
//!
//! Output layout (all paths relative to workspace root):
//! - `completions/bash/k2pdfopt.bash`
//! - `completions/zsh/_k2pdfopt`
//! - `completions/powershell/k2pdfopt.ps1`
//! - `completions/fish/k2pdfopt.fish`
//! - `completions/elvish/k2pdfopt.elv`
//! - `docs/k2pdfopt-rs.1`
//!
//! Driver is idempotent: re-running overwrites every artifact deterministically.
//!
//! Usage (from workspace root):
//! ```sh
//! cargo run --release -p gen-assets
//! cargo run --release -p gen-assets -- --check        # verify on disk matches generated
//! cargo run --release -p gen-assets -- --out-dir DIR  # write under DIR instead of workspace root
//! ```

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use clap_mangen::Man;
use k2cli::CliArgs;

const BIN_NAME: &str = "k2pdfopt";

#[derive(Parser, Debug)]
#[command(
    name = "gen-assets",
    about = "Generate shell completions and man page for k2pdfopt"
)]
struct DriverArgs {
    /// Verify on-disk artifacts match the freshly generated content. Exits non-zero on drift.
    #[arg(long)]
    check: bool,

    /// Target directory root. Defaults to the workspace root inferred from CARGO_MANIFEST_DIR.
    #[arg(long, value_name = "DIR")]
    out_dir: Option<PathBuf>,
}

struct Artifact {
    relative: &'static str,
    bytes: Vec<u8>,
}

fn main() -> Result<()> {
    let cli = DriverArgs::parse();
    let root = resolve_out_dir(cli.out_dir.as_deref())?;
    let artifacts = build_artifacts()?;

    if cli.check {
        let drift = verify(&root, &artifacts)?;
        if drift.is_empty() {
            println!(
                "gen-assets: all artifacts up to date ({} files)",
                artifacts.len()
            );
            return Ok(());
        }
        eprintln!("gen-assets: drift detected in {} file(s):", drift.len());
        for path in &drift {
            eprintln!("  - {}", path.display());
        }
        bail!("artifacts out of date; re-run `cargo run -p gen-assets` to refresh");
    }

    write_all(&root, &artifacts)?;
    println!(
        "gen-assets: wrote {} files under {}",
        artifacts.len(),
        root.display()
    );
    Ok(())
}

fn resolve_out_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    // CARGO_MANIFEST_DIR points at tools/gen_assets/ at build time; go up two levels.
    let manifest = env_path("CARGO_MANIFEST_DIR")?;
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            anyhow!(
                "cannot derive workspace root from CARGO_MANIFEST_DIR={}",
                manifest.display()
            )
        })
}

fn env_path(key: &str) -> Result<PathBuf> {
    std::env::var_os(key)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("environment variable {} not set", key))
}

fn build_artifacts() -> Result<Vec<Artifact>> {
    let mut cmd = CliArgs::command();
    cmd.set_bin_name(BIN_NAME);

    let mut out = Vec::with_capacity(6);

    for (shell, relative) in [
        (Shell::Bash, "completions/bash/k2pdfopt.bash"),
        (Shell::Zsh, "completions/zsh/_k2pdfopt"),
        (Shell::PowerShell, "completions/powershell/k2pdfopt.ps1"),
        (Shell::Fish, "completions/fish/k2pdfopt.fish"),
        (Shell::Elvish, "completions/elvish/k2pdfopt.elv"),
    ] {
        let mut buf = Vec::new();
        generate(shell, &mut cmd, BIN_NAME, &mut buf);
        out.push(Artifact {
            relative,
            bytes: buf,
        });
    }

    // Man page (section 1, roff format).
    let man = Man::new(cmd.clone()).title("K2PDFOPT-RS").section("1");
    let mut man_bytes = Vec::new();
    man.render(&mut man_bytes).context("render man page")?;
    out.push(Artifact {
        relative: "docs/k2pdfopt-rs.1",
        bytes: man_bytes,
    });

    Ok(out)
}

fn write_all(root: &Path, artifacts: &[Artifact]) -> Result<()> {
    for art in artifacts {
        let dst = root.join(art.relative);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).with_context(|| format!("mkdir -p {}", parent.display()))?;
        }
        fs::write(&dst, &art.bytes).with_context(|| format!("write {}", dst.display()))?;
    }
    Ok(())
}

fn verify(root: &Path, artifacts: &[Artifact]) -> Result<Vec<PathBuf>> {
    let mut drift = Vec::new();
    for art in artifacts {
        let dst = root.join(art.relative);
        match fs::read(&dst) {
            Ok(actual) if actual == art.bytes => {}
            Ok(_) | Err(_) => drift.push(dst),
        }
    }
    Ok(drift)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn artifacts_are_non_empty_and_unique() {
        let arts = build_artifacts().expect("build artifacts");
        // 5 completions + 1 man page.
        assert_eq!(arts.len(), 6, "expected exactly 6 artifacts");
        for art in &arts {
            assert!(!art.bytes.is_empty(), "artifact {} is empty", art.relative);
        }
        let mut paths: Vec<_> = arts.iter().map(|a| a.relative).collect();
        paths.sort();
        let unique = paths.iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), paths.len(), "duplicate artifact path");
    }

    #[test]
    fn bash_completion_mentions_binary_name() {
        let arts = build_artifacts().expect("build artifacts");
        let bash = arts
            .iter()
            .find(|a| a.relative.ends_with("k2pdfopt.bash"))
            .expect("bash");
        let text = std::str::from_utf8(&bash.bytes).expect("utf8");
        assert!(
            text.contains("k2pdfopt"),
            "bash completion should reference k2pdfopt"
        );
        // clap_complete bash output uses `_k2pdfopt()` function name.
        assert!(
            text.contains("_k2pdfopt"),
            "bash completion should define _k2pdfopt fn"
        );
    }

    #[test]
    fn man_page_has_roff_header() {
        let arts = build_artifacts().expect("build artifacts");
        let man = arts
            .iter()
            .find(|a| a.relative.ends_with("k2pdfopt-rs.1"))
            .expect("man");
        let text = std::str::from_utf8(&man.bytes).expect("utf8");
        // clap_mangen 0.3+ emits `.ie .ds Aq` quote-compat prologue before `.TH`,
        // so check for the macro anywhere in the header window rather than at offset 0.
        assert!(
            text.lines().take(5).any(|line| line.starts_with(".TH")),
            "man page should contain .TH macro in header"
        );
        assert!(
            text.contains("K2PDFOPT-RS"),
            "man page should carry uppercase title"
        );
    }

    #[test]
    fn round_trip_write_and_verify_in_tempdir() {
        let tmp = std::env::temp_dir().join("k2pdfopt-rs-gen-assets-test");
        // Best-effort clean previous run.
        let _ = fs::remove_dir_all(&tmp);
        let arts = build_artifacts().expect("build artifacts");
        write_all(&tmp, &arts).expect("write");
        let drift = verify(&tmp, &arts).expect("verify");
        assert!(drift.is_empty(), "round-trip drift: {drift:?}");
        // Cleanup.
        let _ = fs::remove_dir_all(&tmp);
    }
}
