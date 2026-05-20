//! Golden baseline generator — runs C k2pdfopt on each fixture and captures output.
//!
//! Usage:
//!   cargo run --bin run_c_baseline -- --all
//!   cargo run --bin run_c_baseline -- --k2-bin /path/to/k2pdfopt.exe --fixture single-column
//!
//! Step 2.4 deliverable.

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Index schema (must match tests/fixtures/INDEX.json)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct IndexEntry {
    /// Includes .pdf suffix in INDEX.json, e.g. "single-column.pdf"
    name: String,
    #[allow(dead_code)]
    pages: u32,
    #[allow(dead_code)]
    characteristics: Vec<String>,
    #[serde(default)]
    baseline_skipped: Option<bool>,
}

impl IndexEntry {
    /// Fixture stem without .pdf suffix, e.g. "single-column"
    fn stem(&self) -> &str {
        self.name.strip_suffix(".pdf").unwrap_or(&self.name)
    }
}

// INDEX.json is a top-level JSON array, not a wrapping object.
type Index = Vec<IndexEntry>;

// ---------------------------------------------------------------------------
// Metadata schema (written per fixture under tests/golden/<name>/metadata.json)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct FixtureMeta {
    fixture: String,
    c_output_pages: u32,
    c_output_bytes: u64,
    c_output_md5: String,
    k2pdfopt_version: String,
    mutool_version: String,
    generated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

struct Args {
    k2_bin: PathBuf,
    fixture_filter: Option<String>,
    all: bool,
}

fn parse_args() -> anyhow::Result<Args> {
    let raw: Vec<String> = std::env::args().collect();
    let mut k2_bin = PathBuf::from("k2pdfopt");
    let mut fixture_filter = None;
    let mut all = false;
    let mut i = 1;
    while i < raw.len() {
        match raw[i].as_str() {
            "--k2-bin" => {
                i += 1;
                k2_bin = PathBuf::from(
                    raw.get(i)
                        .ok_or_else(|| anyhow::anyhow!("--k2-bin requires a path"))?,
                );
            }
            "--fixture" => {
                i += 1;
                fixture_filter = Some(
                    raw.get(i)
                        .ok_or_else(|| anyhow::anyhow!("--fixture requires a name"))?
                        .clone(),
                );
            }
            "--all" => all = true,
            "--help" | "-h" => {
                eprintln!("Usage: run_c_baseline [--k2-bin <path>] [--all | --fixture <name>]");
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
        i += 1;
    }
    if !all && fixture_filter.is_none() {
        anyhow::bail!("specify --all or --fixture <name>");
    }
    Ok(Args {
        k2_bin,
        fixture_filter,
        all,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn workspace_root() -> anyhow::Result<PathBuf> {
    // Compiled binary lives at <workspace>/target/<profile>/run_c_baseline.exe
    // Need to walk up 3 levels to reach workspace root.
    // Also support CARGO_MANIFEST_DIR for more reliable resolution.
    if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        // CARGO_MANIFEST_DIR = tools/run_c_baseline, go up 2 to workspace root
        let mut p = PathBuf::from(dir);
        if p.file_name() == Some(std::ffi::OsStr::new("run_c_baseline")) {
            p = p
                .parent()
                .and_then(|x| x.parent())
                .ok_or_else(|| anyhow::anyhow!("cannot determine workspace root"))?
                .to_path_buf();
        }
        return Ok(p);
    }
    // Fallback: walk up from exe location
    let exe = std::env::current_exe()?;
    let p = exe
        .parent() // target/release or target/debug
        .and_then(|x| x.parent()) // target
        .and_then(|x| x.parent()) // workspace root
        .ok_or_else(|| anyhow::anyhow!("cannot determine workspace root from exe path"))?;
    Ok(p.to_path_buf())
}

fn file_md5(path: &Path) -> anyhow::Result<String> {
    let data = fs::read(path)?;
    let mut hasher = Md5::new();
    hasher.update(&data);
    Ok(format!("{:x}", hasher.finalize()))
}

fn tool_version(bin: &Path, version_arg: &str) -> String {
    let output = Command::new(bin).arg(version_arg).output().ok();
    let raw = output
        .and_then(|o| {
            let stdout = String::from_utf8(o.stdout).unwrap_or_default();
            let stderr = String::from_utf8(o.stderr).unwrap_or_default();
            // Prefer whichever has content
            if stdout.trim().is_empty() {
                stderr
            } else {
                stdout
            }
            .into()
        })
        .unwrap_or_else(|| "unknown".into());
    // Take only the first non-empty line
    raw.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("unknown")
        .trim()
        .to_string()
}

/// Run a command, forward stderr, and check exit code.
fn run(cmd: &mut Command, label: &str) -> anyhow::Result<()> {
    eprintln!("[run_c_baseline] {label}");
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("{label} failed with exit code {:?}", status.code());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Core: process one fixture
// ---------------------------------------------------------------------------

fn process_fixture(ws: &Path, k2_bin: &Path, fixture_name: &str) -> anyhow::Result<()> {
    let fixture_pdf = ws
        .join("tests/fixtures")
        .join(format!("{fixture_name}.pdf"));
    if !fixture_pdf.exists() {
        anyhow::bail!("fixture not found: {}", fixture_pdf.display());
    }

    let golden_dir = ws.join("tests/golden").join(fixture_name);
    let pages_dir = golden_dir.join("c-pages");
    fs::create_dir_all(&pages_dir)?;

    let c_output = golden_dir.join("c-output.pdf");

    // Step 1: Run C k2pdfopt
    // For encrypted PDFs: C k2pdfopt has no CLI password flag, so decrypt first with mutool.
    let input_pdf = if fixture_name == "encrypted" {
        let decrypted = golden_dir.join("_decrypted_tmp.pdf");
        let mut dec_cmd = Command::new("mutool");
        dec_cmd
            .arg("clean")
            .arg("-p")
            .arg("test")
            .arg("-D")
            .arg(&fixture_pdf)
            .arg(&decrypted);
        run(&mut dec_cmd, &format!("mutool decrypt {fixture_name}"))?;
        decrypted
    } else {
        fixture_pdf.clone()
    };

    let mut k2_cmd = Command::new(k2_bin);
    k2_cmd
        .arg("-dev")
        .arg("kv")
        .arg("-ui-")
        .arg("-x")
        .arg("-o")
        .arg(&c_output)
        .arg(&input_pdf);

    let k2_result = run(&mut k2_cmd, &format!("k2pdfopt {fixture_name}"));

    // Clean up temp decrypted file
    if input_pdf != fixture_pdf {
        let _ = fs::remove_file(&input_pdf);
    }

    if k2_result.is_err() {
        eprintln!(
            "[run_c_baseline] WARNING: k2pdfopt failed for {fixture_name} — skipping baseline"
        );
        // Record skip in metadata
        let meta = FixtureMeta {
            fixture: fixture_name.into(),
            c_output_pages: 0,
            c_output_bytes: 0,
            c_output_md5: String::new(),
            k2pdfopt_version: String::new(),
            mutool_version: String::new(),
            generated_at: chrono_now(),
            note: Some("baseline_skipped: k2pdfopt failed".into()),
        };
        let meta_path = golden_dir.join("metadata.json");
        fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;
        eprintln!("[run_c_baseline]   -> metadata.json written (baseline_skipped)");
        return Ok(());
    }

    // Step 2: Convert output PDF to PNGs with mutool
    let png_pattern = pages_dir.join("page-%04d.png");
    let mut mutool_cmd = Command::new("mutool");
    mutool_cmd
        .arg("convert")
        .arg("-O")
        .arg("resolution=150")
        .arg("-o")
        .arg(&png_pattern)
        .arg(&c_output);

    // mutool convert may fail gracefully — not a hard error
    if let Err(e) = run(&mut mutool_cmd, &format!("mutool convert {fixture_name}")) {
        eprintln!("[run_c_baseline] WARNING: mutool convert failed for {fixture_name}: {e}");
    }

    // Step 3: Write metadata.json
    let md5 = file_md5(&c_output).unwrap_or_else(|_| "error".into());
    let bytes = fs::metadata(&c_output).map(|m| m.len()).unwrap_or(0);

    // Count PNG pages
    let page_count = fs::read_dir(&pages_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "png")
                        .unwrap_or(false)
                })
                .count() as u32
        })
        .unwrap_or(0);

    let note = if fixture_name == "encrypted" {
        Some("encrypted fixture: pre-decrypted with mutool clean -p test -D before k2pdfopt".into())
    } else if fixture_name == "blank-page" {
        Some("blank-page fixture: k2pdfopt produces 0-page output".into())
    } else {
        None
    };

    let meta = FixtureMeta {
        fixture: fixture_name.into(),
        c_output_pages: page_count,
        c_output_bytes: bytes,
        c_output_md5: md5,
        k2pdfopt_version: tool_version(k2_bin, "-v"),
        mutool_version: tool_version(Path::new("mutool"), "-v"),
        generated_at: chrono_now(),
        note,
    };

    let meta_path = golden_dir.join("metadata.json");
    fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;
    eprintln!(
        "[run_c_baseline]   -> {} pages, {} bytes, md5={}",
        page_count, bytes, meta.c_output_md5
    );

    Ok(())
}

fn chrono_now() -> String {
    // No chrono dep — use std time + offset from UNIX_EPOCH for a simple timestamp
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("unix+{}", dur.as_secs())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    let ws = workspace_root()?;

    // Resolve k2_bin: try as-is, then relative to workspace, then with .exe suffix
    let mut k2_bin = args.k2_bin.clone();
    if !k2_bin.exists() {
        let ws_relative = ws.join(&args.k2_bin);
        if ws_relative.exists() {
            k2_bin = ws_relative;
        } else {
            let with_exe = args.k2_bin.with_extension("exe");
            let ws_relative_exe = ws.join(&with_exe);
            if ws_relative_exe.exists() {
                k2_bin = ws_relative_exe;
            } else if with_exe.exists() {
                k2_bin = with_exe;
            } else {
                anyhow::bail!(
                    "k2pdfopt binary not found (tried {}, {}, {})",
                    args.k2_bin.display(),
                    ws_relative.display(),
                    ws_relative_exe.display()
                );
            }
        }
    }
    eprintln!("[run_c_baseline] workspace: {}", ws.display());
    eprintln!("[run_c_baseline] k2pdfopt:  {}", k2_bin.display());

    // Load INDEX.json
    let index_path = ws.join("tests/fixtures/INDEX.json");
    let index: Index = serde_json::from_str(&fs::read_to_string(&index_path)?)?;
    eprintln!(
        "[run_c_baseline] loaded {} fixtures from INDEX.json",
        index.len()
    );

    // Select fixtures to process
    let targets: Vec<&IndexEntry> = if args.all {
        index.iter().collect()
    } else if let Some(ref name) = args.fixture_filter {
        // Accept both "single-column" and "single-column.pdf"
        index
            .iter()
            .filter(|f| f.stem() == name.as_str() || f.name == name.as_str())
            .collect()
    } else {
        Vec::new()
    };

    if targets.is_empty() {
        anyhow::bail!("no fixtures selected");
    }

    let mut ok = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for entry in &targets {
        if entry.baseline_skipped.unwrap_or(false) {
            eprintln!(
                "[run_c_baseline] SKIP {} (baseline_skipped in INDEX)",
                entry.stem()
            );
            skipped += 1;
            continue;
        }
        match process_fixture(&ws, &k2_bin, entry.stem()) {
            Ok(()) => ok += 1,
            Err(e) => {
                eprintln!("[run_c_baseline] FAIL {}: {e}", entry.stem());
                failed += 1;
            }
        }
    }

    eprintln!(
        "\n[run_c_baseline] Done: {} ok, {} skipped, {} failed out of {} total",
        ok,
        skipped,
        failed,
        targets.len()
    );

    if failed > 0 {
        anyhow::bail!("{failed} fixture(s) failed baseline generation");
    }
    Ok(())
}
