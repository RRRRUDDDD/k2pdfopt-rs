//! k2pdfopt-rs binary entry point.
//!
//! M1 (Step 3.4+): clap-based CLI via k2cli crate.
//! M1 (Step 3.5): env merge + echo-cmd via Settings::to_args().
//! M1 (Step 3.6): help text + subcommands (list-devices, echo-cmd,
//!                dry-run, compat-report).
//! M5 (Step 7.3): ConvertJob pipeline integration (renderer → layout → PDF writer).
//! M5 (Step 7.4): indicatif progress bar + ctrlc cooperative cancellation (ADR-013).
//! M7 (Step 9.3): --ocr CLI flag + TesseractCliEngine 实例化 + OcrSettings 注入 ConvertJob.
//! M7 (Step 9.4): multi-lang OCR resolution + missing-lang warnings via `lang::resolve`.

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use k2cli::{
    cmd_compat_report, cmd_dry_run, cmd_echo_cmd, cmd_list_devices, merge_env_and_cli, parse_env,
    CliArgs,
};
use k2ocr::{lang::ResolveOptions, OcrEngine, TesseractCliEngine};
use k2pipeline::ocr_bridge::{download_hint, resolve_lang_via_engine, ResolveLangError};
use k2pipeline::{
    CancellationToken, ConvertError, ConvertJob, ConvertJobConfig, ProgressEvent, ProgressObserver,
};
use k2settings::ocr::{OcrMode, OcrStrictMode};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

fn main() -> ExitCode {
    let cli = CliArgs::parse();

    // Meta flags that exit immediately (before env merge needed)
    if cli.list_devices {
        print!("{}", cmd_list_devices());
        return ExitCode::SUCCESS;
    }

    if cli.compat_report {
        // Build settings to provide context in the report
        let settings = match parse_env() {
            Some(env) => k2settings::Settings::from(merge_env_and_cli(env, cli.clone())),
            None => k2settings::Settings::from(cli.clone()),
        };
        print!("{}", cmd_compat_report(&settings));
        return ExitCode::SUCCESS;
    }

    // Build settings with env merge: defaults < K2PDFOPT env < CLI
    let mut settings = match parse_env() {
        Some(env) => k2settings::Settings::from(merge_env_and_cli(env, cli.clone())),
        None => k2settings::Settings::from(cli.clone()),
    };

    if cli.echo_cmd {
        println!("{}", cmd_echo_cmd(&settings));
        return ExitCode::SUCCESS;
    }

    if cli.dry_run {
        println!("{}", cmd_dry_run(&settings, &cli.files));
        return ExitCode::SUCCESS;
    }

    // Normal processing path (Step 7.3 M5 end-to-end pipeline + Step 7.4 progress/cancel)
    if cli.files.is_empty() {
        eprintln!("k2pdfopt-rs: No input files specified. Use --help for usage.");
        return ExitCode::from(2);
    }

    // ---- Step 7.4: 安装 Ctrl-C handler + 取消令牌 ----
    // ctrlc::set_handler 在 spawn-once 语义下重复 set 会返错；这里 ignore
    // 失败（如 CI 容器内已设置），处理代码自身防御性退出。
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_clone = Arc::clone(&cancel_flag);
    let handler_installed = ctrlc::set_handler(move || {
        if !cancel_flag_clone.swap(true, Ordering::Relaxed) {
            // 仅首次按 Ctrl-C 打印提示，避免多次 ctrl-c 时刷屏
            eprintln!("\nk2pdfopt-rs: cancelling… (will exit at next safe checkpoint)");
        }
    })
    .is_ok();
    if !handler_installed && cli.verbose >= 2 {
        eprintln!("k2pdfopt-rs: warning - ctrl-c handler not installed");
    }
    let cancel = CancellationToken::from_atomic(cancel_flag);

    let config = ConvertJobConfig::from_settings(&settings);

    // Step 9.3: 当 settings.ocr.dst_ocr == Tesseract 时实例化 OCR 引擎，
    // 注入到 ConvertJob；其他模式（Off / Mupdf）保留 None。
    // Step 9.4: 引擎 probe 成功后用 resolve_lang_via_engine 做多语言/缺失/fallback 处理。
    // Step 11.8 P0-5: TesseractCliEngine.with_cancel(cancel.shared()) 共享 Ctrl-C
    //   flag，让 OCR 子进程在用户按 Ctrl-C 时也能被 kill (Unix SIGINT / Windows
    //   TerminateProcess)；engine 与 ConvertJob.cancel 是同一个 Arc<AtomicBool>。
    let ocr_engine: Option<Arc<dyn OcrEngine>> = if matches!(
        settings.ocr.dst_ocr,
        OcrMode::Tesseract
    ) {
        let engine: Arc<dyn OcrEngine> =
            Arc::new(TesseractCliEngine::new().with_cancel(cancel.shared()));
        // 立即 probe 一次，把"tesseract 不在 PATH"之类问题在跑 pipeline 前抛出
        match engine.probe() {
            Ok(info) => {
                // Step 9.4: 解析多语言 / 检测缺失 / fallback 到 eng + 打印 warning
                // Step 11.9: 把 settings.ocr.ocr_strict_mode 传入控制 strict / partial /
                //   fallback 行为，默认 Fallback 与 v0.1.0 完全一致。
                //   Strict 模式 + 缺语言 → resolve_ocr_lang_or_warn 返 Err，
                //   立即 exit 1（user value：严格模式名副其实 fail-fast，不让
                //   下游 figure_bypassed 路径吞掉 strict 语义 / 见 Open Q 11.9.A）。
                let strict_mode = settings.ocr.ocr_strict_mode;
                if let Err(e) = resolve_ocr_lang_or_warn(
                    engine.as_ref(),
                    &mut settings,
                    strict_mode,
                    cli.verbose,
                ) {
                    eprintln!(
                            "k2pdfopt-rs: fatal - strict OCR mode requires all requested languages installed: {e}"
                        );
                    return ExitCode::from(1);
                }
                if cli.verbose >= 1 {
                    eprintln!(
                        "k2pdfopt-rs: OCR engine {} v{} ready (lang={})",
                        info.engine_name, info.version, settings.ocr.dst_ocr_lang
                    );
                }
                Some(engine)
            }
            Err(e) => {
                eprintln!(
                    "k2pdfopt-rs: --ocr requested but engine probe failed: {e}. Disabling OCR."
                );
                None
            }
        }
    } else {
        None
    };

    let mut had_error = false;
    let mut was_cancelled = false;

    for input in &cli.files {
        if cancel.is_cancelled() {
            was_cancelled = true;
            break;
        }
        let output = compute_output_path(input, cli.output.as_deref());
        if cli.verbose >= 1 {
            println!(
                "k2pdfopt-rs: converting {} -> {} (dst {}x{} @{} dpi)",
                input,
                output.display(),
                config.dst_width,
                config.dst_height,
                config.dst_dpi
            );
        }

        // CLI observer：每个 job 独立一个 progress bar
        let cli_observer = Arc::new(CliObserver::new(input.clone(), cli.verbose));
        let observer: Arc<dyn ProgressObserver> = cli_observer.clone();
        let mut job = ConvertJob::new(input, &output, config.clone())
            .with_observer(observer)
            .with_cancel(cancel.clone())
            .with_ocr_settings(settings.ocr.clone())
            .with_reflow_mode(settings.layout.reflow_mode);
        if let Some(ref engine) = ocr_engine {
            job = job.with_ocr_engine(Arc::clone(engine));
        }

        match job.run() {
            Ok(()) => {
                cli_observer.on_job_finished_ok();
                if cli.verbose >= 1 {
                    println!("k2pdfopt-rs: wrote {}", output.display());
                }
            }
            Err(ConvertError::Cancelled) => {
                cli_observer.on_job_cancelled();
                was_cancelled = true;
                break;
            }
            Err(e) => {
                cli_observer.on_job_failed(&e);
                eprintln!("k2pdfopt-rs: error processing {input}: {e}");
                had_error = true;
            }
        }
    }

    if was_cancelled {
        // POSIX 约定：128 + SIGINT(2) = 130
        ExitCode::from(130)
    } else if had_error {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// CLI 进度回调实现：基于 [`indicatif::ProgressBar`]，每 ConvertJob 一个实例。
///
/// 设计要点：
///
/// - 终端非 TTY 时 indicatif 会自动隐藏，不污染管道输出
/// - 互斥锁包 `ProgressBar` 是为了 `ProgressObserver: Send + Sync` 要求；
///   indicatif 本身 thread-safe 但内部状态可变；用 Mutex 简化所有权
/// - verbose=0：仅进度条；verbose>=1：进度条 + per-event println
struct CliObserver {
    input_label: String,
    verbose: u8,
    bar: Mutex<Option<ProgressBar>>,
}

impl CliObserver {
    fn new(input: String, verbose: u8) -> Self {
        Self {
            input_label: input,
            verbose,
            bar: Mutex::new(None),
        }
    }

    fn lazy_init_bar(&self, total: u64) {
        if let Ok(mut g) = self.bar.lock() {
            if g.is_none() {
                let pb = ProgressBar::new(total);
                let style = ProgressStyle::with_template(
                    "{prefix:>8} [{elapsed_precise}] [{bar:30.cyan/blue}] {pos}/{len} {wide_msg}",
                )
                .unwrap_or_else(|_| ProgressStyle::default_bar())
                .progress_chars("=>-");
                pb.set_style(style);
                pb.set_prefix("convert");
                pb.set_message(self.input_label.clone());
                *g = Some(pb);
            } else if let Some(b) = g.as_ref() {
                b.set_length(total);
            }
        }
    }

    fn with_bar<F: FnOnce(&ProgressBar)>(&self, f: F) {
        if let Ok(g) = self.bar.lock() {
            if let Some(b) = g.as_ref() {
                f(b);
            }
        }
    }

    fn on_job_finished_ok(&self) {
        self.with_bar(|b| {
            b.finish_with_message(format!("done: {}", self.input_label));
        });
    }

    fn on_job_cancelled(&self) {
        self.with_bar(|b| {
            b.abandon_with_message(format!("cancelled: {}", self.input_label));
        });
    }

    fn on_job_failed(&self, err: &ConvertError) {
        self.with_bar(|b| {
            b.abandon_with_message(format!("error: {err}"));
        });
    }
}

impl ProgressObserver for CliObserver {
    fn on_event(&self, event: &ProgressEvent) {
        match event {
            ProgressEvent::JobStart { total_pages } => {
                self.lazy_init_bar(*total_pages as u64);
                if self.verbose >= 2 {
                    eprintln!(
                        "k2pdfopt-rs: JobStart total_pages={} ({})",
                        total_pages, self.input_label
                    );
                }
            }
            ProgressEvent::PageStart { source_page } => {
                if self.verbose >= 2 {
                    eprintln!("k2pdfopt-rs: PageStart source_page={}", source_page);
                }
            }
            ProgressEvent::PageDone {
                source_page,
                dst_pages,
            } => {
                self.with_bar(|b| {
                    b.set_position(*source_page as u64 + 1);
                    b.set_message(format!(
                        "{} (page {} → +{} dst)",
                        self.input_label,
                        source_page + 1,
                        dst_pages
                    ));
                });
                if self.verbose >= 2 {
                    eprintln!(
                        "k2pdfopt-rs: PageDone source_page={} dst_pages={}",
                        source_page, dst_pages
                    );
                }
            }
            ProgressEvent::PdfWrite {
                dst_pages_written,
                total_dst_pages,
            } => {
                if self.verbose >= 2 {
                    eprintln!(
                        "k2pdfopt-rs: PdfWrite written={}/{}",
                        dst_pages_written, total_dst_pages
                    );
                }
            }
            ProgressEvent::OcrPage {
                source_page,
                words_found,
            } => {
                if self.verbose >= 2 {
                    eprintln!(
                        "k2pdfopt-rs: OcrPage src={} words={}",
                        source_page, words_found
                    );
                }
            }
            ProgressEvent::JobDone {
                total_input_pages,
                total_output_pages,
                elapsed_ms,
            } => {
                self.with_bar(|b| {
                    b.set_position(*total_input_pages as u64);
                });
                if self.verbose >= 1 {
                    eprintln!(
                        "k2pdfopt-rs: JobDone input={} output={} elapsed_ms={}",
                        total_input_pages, total_output_pages, elapsed_ms
                    );
                }
            }
            ProgressEvent::Warn { message } => {
                self.with_bar(|b| {
                    b.println(format!("warn: {}", message));
                });
            }
        }
    }
}

/// 计算单个输入文件对应的输出路径。
///
/// - 若 `output_fmt` 提供且不含 `%s`：直接当作输出路径
/// - 若 `output_fmt` 提供且含 `%s`：用 `<input_stem>_k2opt.pdf` 模板替换（M1 简化）
/// - 若未提供：默认 `<input_stem>_k2opt.pdf`（与 C 版默认 `dst_opname_format` 一致）
fn compute_output_path(input: &str, output_fmt: Option<&str>) -> PathBuf {
    let stem = Path::new(input)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let default_name = format!("{}_k2opt.pdf", stem);

    match output_fmt {
        Some(fmt) if fmt.contains("%s") => {
            let name = fmt.replace("%s", stem);
            let p = Path::new(&name);
            if p.extension().is_some() {
                p.to_path_buf()
            } else {
                PathBuf::from(format!("{}.pdf", name))
            }
        }
        Some(p) => PathBuf::from(p),
        None => {
            // 默认放在与输入同目录
            let parent = Path::new(input).parent();
            match parent {
                Some(p) if !p.as_os_str().is_empty() => p.join(&default_name),
                _ => PathBuf::from(&default_name),
            }
        }
    }
}

/// Step 9.4 - 多语言 OCR lang 解析 + warning 输出。
///
/// 调用 [`resolve_lang_via_engine`] 用引擎实际可用的语言包决定 `dst_ocr_lang` 的最终值：
///
/// - **全部命中**：静默通过（verbose>=1 时打印 resolved）
/// - **部分缺失**：丢弃缺失段，保留命中段 + warning（含 `download_hint` URL）
/// - **全部缺失但 eng 可用**：fallback 到 `eng` + warning
/// - **全部缺失且 eng 也缺**：保留原始 settings.ocr.dst_ocr_lang（让 recognize 时报 LanguageNotInstalled）
/// - **引擎查询失败**：保留原 settings，仅打 warning（不阻塞 pipeline，下游 recognize 会再报错）
///
/// 函数会在解析成功时把 `res.resolved_arg` 写回 `settings.ocr.dst_ocr_lang`，
/// 使后续 `ConvertJob` 内 `build_ocr_input` 走的就是 resolved 后的 lang。
///
/// **Step 11.9 P0-6**：`mode` 参数从 `settings.ocr.ocr_strict_mode` 取得，控制 strict / partial / fallback
/// 行为。映射对照见 [`OcrStrictMode::to_resolve_bools`]：
/// - `Strict`  → `ResolveOptions { fallback_to_eng: false, allow_partial: false }`（缺即 Err）
/// - `Partial` → `ResolveOptions { fallback_to_eng: false, allow_partial: true  }`（丢失保留命中）
/// - `Fallback`→ `ResolveOptions { fallback_to_eng: true,  allow_partial: true  }`（v0.1.0 行为）
///
/// # 返回
///
/// - `Ok(())`：解析成功（含 partial drop / fallback eng 的 warning 路径），调用方继续 pipeline。
/// - `Err(ResolveLangError)`：仅在 strict 模式且检测到缺语言 / NoUsableLang 时返。
///   调用方应据此 exit 1，让 strict 模式 fail-fast 兑现"缺即报错"语义。
///   `EngineQuery` 失败仍走 Err 路径（让用户看到底层问题），但 main.rs 也可以选择
///   忽略 EngineQuery 错误改成 warning + 兜底 v0.1.0 行为 —— 当前实现 fail-fast。
fn resolve_ocr_lang_or_warn(
    engine: &dyn OcrEngine,
    settings: &mut k2settings::Settings,
    mode: OcrStrictMode,
    verbose: u8,
) -> Result<(), ResolveLangError> {
    // Step 11.9: OcrStrictMode → (fallback_to_eng, allow_partial) → ResolveOptions
    let (fallback_to_eng, allow_partial) = mode.to_resolve_bools();
    let opts = ResolveOptions {
        fallback_to_eng,
        allow_partial,
    };
    let is_strict = matches!(mode, OcrStrictMode::Strict);
    match resolve_lang_via_engine(engine, &settings.ocr, &opts) {
        Ok(res) => {
            if res.fallback_used {
                eprintln!(
                    "k2pdfopt-rs: warning - requested OCR lang '{}' not installed; falling back to '{}'",
                    settings.ocr.dst_ocr_lang, res.resolved_arg
                );
                for missing in &res.missing {
                    eprintln!(
                        "    install hint: {} -> {}",
                        missing,
                        download_hint(missing)
                    );
                }
            } else if res.has_missing() {
                eprintln!(
                    "k2pdfopt-rs: warning - dropping missing OCR lang(s); using '{}'",
                    res.resolved_arg
                );
                for missing in &res.missing {
                    eprintln!(
                        "    install hint: {} -> {}",
                        missing,
                        download_hint(missing)
                    );
                }
            }
            if verbose >= 1 {
                eprintln!(
                    "k2pdfopt-rs: OCR lang resolved to '{}' (requested '{}')",
                    res.resolved_arg, settings.ocr.dst_ocr_lang
                );
            }
            settings.ocr.dst_ocr_lang = res.resolved_arg;
            Ok(())
        }
        Err(e) => {
            if is_strict {
                // Step 11.9 strict 模式 fail-fast：让调用方 exit 1 兑现"缺即报错"语义。
                // 与 v0.1.0 行为差异：v0.1.0 时不存在 strict 模式，缺语言永远 fallback eng + warning。
                Err(e)
            } else {
                // Fallback / Partial 模式：保留原打 warning 不阻塞行为（兼容 v0.1.0）。
                // 这里 e 通常是 ResolveLangError::EngineQuery（list_langs 失败），偶尔是
                // NoUsableLang（fallback 关 + eng 缺，理论上 Fallback / Partial 不会触发 NoUsableLang
                // 路径除非 eng 也缺）。
                eprintln!("k2pdfopt-rs: warning - could not resolve OCR lang: {e}");
                Ok(())
            }
        }
    }
}
