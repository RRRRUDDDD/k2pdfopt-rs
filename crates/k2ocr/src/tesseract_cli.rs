//! `k2ocr::tesseract_cli` —— ADR-017 选定的 MVP OCR 引擎实现。
//!
//! 调用流程（与 spike `spikes/ocr-cli/src/main.rs` 同源）：
//!
//! 1. **probe**: `tesseract --version` → 解析首行 `tesseract v5.5.0.xxx`。
//! 2. **list_langs**: `tesseract --list-langs` → 一行一个语言短名（跳过 "List of ..." header）。
//! 3. **recognize**:
//!    a. `effective_roi` 确认 ROI 在 bitmap 范围内
//!    b. 任意 PixelFormat → Gray8（luminance 0.299 R + 0.587 G + 0.114 B；Rgba8 丢 alpha）
//!    c. 写临时 PNG（[`crate::scoped_tempfile::ScopedTempFile`] 自动清理）
//!    d. `tesseract <tmp> stdout -l <lang> --psm N --oem M tsv`
//!    e. [`crate::tsv_parser::parse_tsv`] 解析后加 ROI offset 转 [`OcrWord`]
//!
//! 与 C 版 `willuslib/ocrtess.c::ocrtess_ocrwords_from_bmp8` 的关键差异：
//!
//! | 行为 | C 版 | Rust CLI |
//! |------|------|----------|
//! | 引擎接口 | leptonica `pixCreate` + `tess_capi_get_ocr_multiword` | 子进程 `tesseract <img> stdout tsv` |
//! | ROI 边框 (`bw=max(w/40,6)`) | 加白边后送 OCR | **不加边框**（CLI Tesseract 不需要 + 简化 Rust 端）|
//! | downsample | 可选 `bmp_resize` | 推迟（Open Question 9.1.D）|
//! | DPI | `pixSetXRes/YRes` | 通过临时 PNG metadata（image crate 写 dpi） |
//! | 输出字段 | OCRWORD with `ybase / lcheight / maxheight` | [`OcrWord`] 6 字段（无 baseline）|
//!
//! Open Question 9.1.A：CLI 子进程启动开销（实测 ~735ms/页 spike，长文档累积明显）。
//! 推迟 M7 末 vs leptess FFI benchmark。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use k2types::{Bitmap, OcrWord, PixelFormat};

use crate::scoped_tempfile::ScopedTempFile;
use crate::tsv_parser::parse_tsv;
use crate::types::{OcrEngineInfo, OcrError, OcrPageInput, OcrRoi};
use crate::OcrEngine;

/// 默认 tesseract 可执行名（依赖 PATH 查找）。
pub const DEFAULT_TESSERACT_EXECUTABLE: &str = "tesseract";

/// Step 11.8 P0-5：cancel 轮询间隔（默认 100 ms，与执行计划 §11.8 字面一致）。
/// 选 100 ms 是按 ADR-013 协作式取消的"用户感知 ≤ 200 ms"目标的折中：
/// 子进程实际退出 + drain stdout 还需若干 ms，整体 wall-clock 控制在 ≤ 200 ms。
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Tesseract CLI 引擎实现。
///
/// 不持有 tesseract 子进程实例；每次 `recognize` 起一个短命子进程（与 spike 同源）。
///
/// # Step 11.8 P0-5：协作式取消
///
/// [`Self::with_cancel`] 注入 `Arc<AtomicBool>` 后，`recognize` 内 spawn 子进程 +
/// 轮询 `try_wait`，每 [`CANCEL_POLL_INTERVAL`] 检查一次 cancel flag；翻 true 即
/// Unix 发 SIGINT / Windows 调 `Child::kill`（WinAPI TerminateProcess 等价），
/// 收尸后返 [`OcrError::Cancelled`]。`probe` / `list_langs` 仍走阻塞调用（短命，
/// 不必复杂化）。详见 ADR-013 + 执行计划 §11.8。
pub struct TesseractCliEngine {
    /// 可执行文件路径；默认 `"tesseract"`（依赖 PATH）。
    executable: PathBuf,
    /// 自定义 `TESSDATA_PREFIX`；`None` = 让 tesseract 用系统默认。
    tessdata_prefix: Option<PathBuf>,
    /// Step 11.8 P0-5：cancel 标志。`None` 表示不响应取消（阻塞到子进程自然结束）；
    /// `Some(flag)` 时 recognize 内部轮询，翻 true 即 kill。
    cancel: Option<Arc<AtomicBool>>,
    info_cache: OnceLock<OcrEngineInfo>,
    langs_cache: OnceLock<Vec<String>>,
}

impl Default for TesseractCliEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TesseractCliEngine {
    /// 创建 default engine（`tesseract` 走 PATH，无自定义 tessdata）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            executable: PathBuf::from(DEFAULT_TESSERACT_EXECUTABLE),
            tessdata_prefix: None,
            cancel: None,
            info_cache: OnceLock::new(),
            langs_cache: OnceLock::new(),
        }
    }

    /// builder：指定可执行文件路径（绝对路径或 PATH 可发现的名字）。
    #[must_use]
    pub fn with_executable<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.executable = path.into();
        self
    }

    /// builder：覆盖 `TESSDATA_PREFIX` 环境变量。
    #[must_use]
    pub fn with_tessdata_prefix<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.tessdata_prefix = Some(path.into());
        self
    }

    /// builder（Step 11.8 P0-5）：注入 cancel 标志。
    ///
    /// 调用方应传 [`k2pipeline::CancellationToken::shared`] 返回的 `Arc<AtomicBool>`
    /// （这样 ctrlc handler 翻 flag 后所有 ConvertJob / OCR engine 同时感知）。
    /// 不依赖 k2pipeline crate，避免反向依赖（k2pipeline 已依赖 k2ocr）。
    #[must_use]
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// 用 `executable` + 用户 tessdata_prefix 构造一个 Command。
    fn make_command(&self) -> Command {
        let mut cmd = Command::new(&self.executable);
        if let Some(p) = &self.tessdata_prefix {
            cmd.env("TESSDATA_PREFIX", p);
        }
        cmd
    }

    /// 跑一次 tesseract 子进程，返回 stdout 字节。
    ///
    /// # Step 11.8 P0-5 改造
    ///
    /// - 若未注入 `cancel` 字段 → 走原阻塞 `Command::output` 路径（与 v0.1.0 / v0.2 P0-4
    ///   行为一致，零开销）
    /// - 若注入 `cancel` 字段 → 用 `Command::spawn` + `try_wait` 轮询 +
    ///   `Child::kill`（Unix SIGINT / Windows TerminateProcess）。轮询间隔
    ///   [`CANCEL_POLL_INTERVAL`]（100 ms）；cancel 触发返 [`OcrError::Cancelled`]
    ///
    /// 错误映射：
    /// - NotFound → [`OcrError::EngineNotFound`]
    /// - 其它 IO → [`OcrError::EngineIo`]
    /// - 非 0 退出 → [`OcrError::EngineExitNonZero`]
    /// - cancel 触发 → [`OcrError::Cancelled`]
    fn run(&self, args: &[&str]) -> Result<Vec<u8>, OcrError> {
        // 无 cancel 字段：走原阻塞路径，与 v0.1.0 兼容
        let Some(cancel) = self.cancel.clone() else {
            return self.run_blocking(args);
        };

        // 有 cancel 字段：spawn + try_wait 轮询
        let mut cmd = self.make_command();
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(OcrError::EngineNotFound {
                    engine: self.executable.display().to_string(),
                    message: e.to_string(),
                });
            }
            Err(e) => return Err(OcrError::EngineIo(e)),
        };

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    // 子进程已退出：drain stdout/stderr，按状态决定返回
                    let mut stdout_buf = Vec::new();
                    let mut stderr_buf = Vec::new();
                    if let Some(mut o) = child.stdout.take() {
                        let _ = o.read_to_end(&mut stdout_buf);
                    }
                    if let Some(mut e) = child.stderr.take() {
                        let _ = e.read_to_end(&mut stderr_buf);
                    }
                    return if status.success() {
                        Ok(stdout_buf)
                    } else {
                        Err(OcrError::EngineExitNonZero {
                            exit_code: status.code(),
                            stderr: String::from_utf8_lossy(&stderr_buf).into_owned(),
                        })
                    };
                }
                Ok(None) => {
                    // 子进程仍在跑：检查 cancel
                    if cancel.load(Ordering::Relaxed) {
                        kill_child(&mut child);
                        // 收尸阻塞等子进程实际退出，避免僵尸进程
                        let _ = child.wait();
                        return Err(OcrError::Cancelled);
                    }
                    std::thread::sleep(CANCEL_POLL_INTERVAL);
                }
                Err(e) => return Err(OcrError::EngineIo(e)),
            }
        }
    }

    /// v0.1.0 兼容的阻塞实现，供 `cancel == None` 时调用（零开销）。
    fn run_blocking(&self, args: &[&str]) -> Result<Vec<u8>, OcrError> {
        let mut cmd = self.make_command();
        cmd.args(args);
        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(OcrError::EngineNotFound {
                    engine: self.executable.display().to_string(),
                    message: e.to_string(),
                });
            }
            Err(e) => return Err(OcrError::EngineIo(e)),
        };
        if !output.status.success() {
            return Err(OcrError::EngineExitNonZero {
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(output.stdout)
    }

    /// 内部 helper：跑 tesseract 并拿 stderr+stdout 合并文本（`--version` 旧版写 stderr）。
    fn run_collecting_text(&self, args: &[&str]) -> Result<String, OcrError> {
        let mut cmd = self.make_command();
        cmd.args(args);
        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(OcrError::EngineNotFound {
                    engine: self.executable.display().to_string(),
                    message: e.to_string(),
                });
            }
            Err(e) => return Err(OcrError::EngineIo(e)),
        };
        if !output.status.success() {
            return Err(OcrError::EngineExitNonZero {
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        // tesseract --version 在 4.x/5.x 写 stdout，旧版（< 4）写 stderr；按"非空者优先"合并。
        let mut combined = String::new();
        let out_s = String::from_utf8_lossy(&output.stdout);
        let err_s = String::from_utf8_lossy(&output.stderr);
        if !out_s.trim().is_empty() {
            combined.push_str(&out_s);
        }
        if !err_s.trim().is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&err_s);
        }
        Ok(combined)
    }
}

impl OcrEngine for TesseractCliEngine {
    fn engine_name(&self) -> &'static str {
        "tesseract-cli"
    }

    fn probe(&self) -> Result<OcrEngineInfo, OcrError> {
        if let Some(info) = self.info_cache.get() {
            return Ok(info.clone());
        }
        let text = self.run_collecting_text(&["--version"])?;
        let version = parse_tesseract_version(&text).ok_or_else(|| {
            OcrError::OutputParse(format!("无法解析 Tesseract 版本号: {}", text.trim()))
        })?;
        let info = OcrEngineInfo {
            engine_name: "tesseract-cli".to_string(),
            version,
            data_path: self.tessdata_prefix.clone(),
        };
        let _ = self.info_cache.set(info.clone());
        Ok(info)
    }

    fn list_langs(&self) -> Result<Vec<String>, OcrError> {
        if let Some(langs) = self.langs_cache.get() {
            return Ok(langs.clone());
        }
        let text = self.run_collecting_text(&["--list-langs"])?;
        let langs = parse_tesseract_langs(&text);
        let _ = self.langs_cache.set(langs.clone());
        Ok(langs)
    }

    fn recognize(&self, input: &OcrPageInput<'_>) -> Result<Vec<OcrWord>, OcrError> {
        // 1. 校验 ROI
        let roi = input.effective_roi()?;

        // 2. 校验语言包（空 lang → 默认 "eng"）
        let lang_str = if input.lang.is_empty() {
            "eng".to_string()
        } else {
            input.lang.clone()
        };
        let avail = self.list_langs()?;
        for piece in lang_str.split('+') {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            if !avail.iter().any(|a| a == piece) {
                return Err(OcrError::LanguageNotInstalled {
                    lang: piece.to_string(),
                    available: avail.join(", "),
                });
            }
        }

        // 3. 裁切 ROI 转 Gray8（任何 PixelFormat → 灰度，单条 alloc）
        let cropped = crop_to_gray8(input.bitmap, roi)?;

        // 4. 写临时 PNG
        let tmp = ScopedTempFile::allocate("k2ocr", ".png").map_err(OcrError::EngineIo)?;
        write_gray8_png(&cropped, tmp.path())?;

        // 5. 调 tesseract <file> stdout -l <lang> --psm N --oem M tsv
        let path_str = tmp.path().to_string_lossy().into_owned();
        let psm = input.psm.to_arg();
        let oem = input.oem.to_arg();
        let stdout = self.run(&[
            path_str.as_str(),
            "stdout",
            "-l",
            lang_str.as_str(),
            "--psm",
            psm,
            "--oem",
            oem,
            "tsv",
        ])?;

        // 6. 解析 TSV（min_confidence 0.0-1.0 → 0-100 比例）
        let tsv = String::from_utf8_lossy(&stdout);
        let min_conf_pct = (input.min_confidence * 100.0).clamp(0.0, 100.0);
        let tsv_words = parse_tsv(&tsv, min_conf_pct)?;

        // 7. 转 OcrWord：加 ROI offset + confidence 归一化
        let mut words = Vec::with_capacity(tsv_words.len());
        for w in tsv_words {
            words.push(OcrWord {
                text: w.text,
                x: f64::from(w.left) + f64::from(roi.x0),
                y: f64::from(w.top) + f64::from(roi.y0),
                w: f64::from(w.width),
                h: f64::from(w.height),
                confidence: (w.confidence / 100.0).clamp(0.0, 1.0),
            });
        }

        Ok(words)
    }
}

/// 解析 `tesseract --version` 输出首行版本号。
///
/// 5.5.0 实际输出：
/// ```text
/// tesseract v5.5.0.20241111
///  leptonica-1.85.0
/// ```
///
/// 较旧版本可能是 `tesseract 4.1.1` (无 `v` 前缀)。两种都支持。
pub(crate) fn parse_tesseract_version(text: &str) -> Option<String> {
    let first = text.lines().find(|l| !l.trim().is_empty())?;
    let mut iter = first.split_whitespace();
    let name = iter.next()?;
    if !name.eq_ignore_ascii_case("tesseract") {
        return None;
    }
    let raw_ver = iter.next()?;
    Some(raw_ver.trim_start_matches('v').to_string())
}

/// 解析 `tesseract --list-langs` 输出。
///
/// 5.5.0 实际输出（注意 header 行包含双引号）：
/// ```text
/// List of available languages in "C:\Program Files\Tesseract-OCR/tessdata/" (2):
/// eng
/// osd
/// ```
///
/// `4.x` header 大致一致；我们跳过首行 `List of ...`，剩下逐行收（仅 ASCII alnum + `_` + `-`）。
pub(crate) fn parse_tesseract_langs(text: &str) -> Vec<String> {
    let mut langs = Vec::new();
    for line in text.lines() {
        let s = line.trim();
        if s.is_empty() || s.starts_with("List of") {
            continue;
        }
        // 真实语言短名都是 [a-z0-9_-]+；带空格/其它符号的行（如 stderr 警告）跳过。
        if s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            langs.push(s.to_string());
        }
    }
    langs
}

/// 把任意 PixelFormat 的 [`Bitmap`] 在 ROI 范围内裁切为新的 Gray8 [`Bitmap`]。
///
/// luminance 公式与 [`k2pipeline::convert::render_to_gray8`] 同源（Step 7.3 落地）：
/// `Y = 0.299 R + 0.587 G + 0.114 B`，向下舍入到 `u8`。Rgba8 alpha 通道丢弃。
pub(crate) fn crop_to_gray8(bitmap: &Bitmap, roi: OcrRoi) -> Result<Bitmap, OcrError> {
    let new_w = roi.width();
    let new_h = roi.height();
    let size_usize = (new_w as usize)
        .checked_mul(new_h as usize)
        .ok_or_else(|| OcrError::BitmapEncoding("ROI 宽高乘积溢出".to_string()))?;
    let mut pixels = Vec::with_capacity(size_usize);

    let bpp = bitmap.format.bytes_per_pixel();
    let stride = (bitmap.width as usize) * bpp;

    for y in roi.y0..=roi.y1 {
        let row_start = (y as usize) * stride;
        for x in roi.x0..=roi.x1 {
            let off = row_start + (x as usize) * bpp;
            let gray = match bitmap.format {
                PixelFormat::Gray8 => bitmap.pixels[off],
                PixelFormat::Rgb8 | PixelFormat::Rgba8 => {
                    let r = f32::from(bitmap.pixels[off]);
                    let g = f32::from(bitmap.pixels[off + 1]);
                    let b = f32::from(bitmap.pixels[off + 2]);
                    let y_lin = 0.299_f32 * r + 0.587_f32 * g + 0.114_f32 * b;
                    let y_clamp = y_lin.round().clamp(0.0, 255.0);
                    y_clamp as u8
                }
            };
            pixels.push(gray);
        }
    }

    Bitmap::from_raw(new_w, new_h, bitmap.dpi, PixelFormat::Gray8, pixels).map_err(OcrError::Bitmap)
}

/// 把 Gray8 [`Bitmap`] 序列化为 PNG 写到 `path`。
///
/// 用 `image` crate 0.25。失败包装成 [`OcrError::BitmapEncoding`]。
pub(crate) fn write_gray8_png(bitmap: &Bitmap, path: &Path) -> Result<(), OcrError> {
    if bitmap.format != PixelFormat::Gray8 {
        return Err(OcrError::BitmapEncoding(format!(
            "write_gray8_png 仅接受 Gray8 输入；实际 {:?}",
            bitmap.format
        )));
    }
    let img = image::GrayImage::from_raw(bitmap.width, bitmap.height, bitmap.pixels.clone())
        .ok_or_else(|| {
            OcrError::BitmapEncoding(
                "image::GrayImage::from_raw 返回 None (像素数与 width*height 不匹配)".to_string(),
            )
        })?;
    // 显式 image::ImageFormat::Png 避免依赖文件扩展名。
    let file = std::fs::File::create(path)
        .map_err(|e| OcrError::BitmapEncoding(format!("无法创建 PNG 文件: {e}")))?;
    let mut writer = std::io::BufWriter::new(file);
    img.write_to(&mut writer, image::ImageFormat::Png)
        .map_err(|e| OcrError::BitmapEncoding(format!("PNG 编码失败: {e}")))?;
    writer
        .flush()
        .map_err(|e| OcrError::BitmapEncoding(format!("PNG flush 失败: {e}")))?;
    Ok(())
}

/// Step 11.8 P0-5：跨平台杀子进程。
///
/// - Unix：`nix::sys::signal::kill(pid, SIGINT)`，与 ctrlc handler 同语义（让
///   tesseract 走自己的 cleanup 退出码 130，与 v0.1.0 端到端用户体验一致）
/// - Windows / 其他平台：`std::process::Child::kill`（WinAPI `TerminateProcess`
///   等价）。Windows 无 SIGINT 概念，按惯例直接 terminate
///
/// 调用方负责后续 `child.wait()` 收尸（避免僵尸进程）。
fn kill_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        // 子进程可能 race 已退出；ignore err 由后续 wait() 兜底收尸。
        if let Ok(pid) = i32::try_from(child.id()) {
            let _ = kill(Pid::from_raw(pid), Signal::SIGINT);
        }
    }
    #[cfg(not(unix))]
    {
        // Windows / WASI / 其他：std `Child::kill` 即 TerminateProcess。
        let _ = child.kill();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn make_rgb8(w: u32, h: u32, fill: [u8; 3]) -> Bitmap {
        let pixels = (0..(w * h)).flat_map(|_| fill).collect::<Vec<u8>>();
        Bitmap::from_raw(w, h, 300.0, PixelFormat::Rgb8, pixels).unwrap()
    }

    fn make_rgba8(w: u32, h: u32, fill: [u8; 4]) -> Bitmap {
        let pixels = (0..(w * h)).flat_map(|_| fill).collect::<Vec<u8>>();
        Bitmap::from_raw(w, h, 300.0, PixelFormat::Rgba8, pixels).unwrap()
    }

    fn make_gray8(w: u32, h: u32, fill: u8) -> Bitmap {
        let pixels = vec![fill; (w * h) as usize];
        Bitmap::from_raw(w, h, 300.0, PixelFormat::Gray8, pixels).unwrap()
    }

    #[test]
    fn parse_version_5x() {
        let v =
            parse_tesseract_version("tesseract v5.5.0.20241111\n leptonica-1.85.0\n  libgif 5.2.2");
        assert_eq!(v.as_deref(), Some("5.5.0.20241111"));
    }

    #[test]
    fn parse_version_4x_no_v_prefix() {
        let v = parse_tesseract_version("tesseract 4.1.1\nleptonica-1.79.0\n");
        assert_eq!(v.as_deref(), Some("4.1.1"));
    }

    #[test]
    fn parse_version_empty_returns_none() {
        assert!(parse_tesseract_version("").is_none());
    }

    #[test]
    fn parse_version_garbage_returns_none() {
        assert!(parse_tesseract_version("not a tesseract\nhello").is_none());
    }

    #[test]
    fn parse_version_partial_returns_none() {
        // 仅 "tesseract" 无版本号 → None
        assert!(parse_tesseract_version("tesseract\n").is_none());
    }

    #[test]
    fn parse_version_with_blank_first_line() {
        let v = parse_tesseract_version("\n\ntesseract v5.5.0\n");
        assert_eq!(v.as_deref(), Some("5.5.0"));
    }

    #[test]
    fn parse_langs_5x_with_header() {
        let s = "List of available languages in \"...tessdata/\" (2):\neng\nosd\n";
        let langs = parse_tesseract_langs(s);
        assert_eq!(langs, vec!["eng".to_string(), "osd".to_string()]);
    }

    #[test]
    fn parse_langs_with_chi_sim_and_underscore() {
        let s = "List of available languages in (3):\nchi_sim\neng\nosd\n";
        let langs = parse_tesseract_langs(s);
        assert_eq!(
            langs,
            vec!["chi_sim".to_string(), "eng".to_string(), "osd".to_string()]
        );
    }

    #[test]
    fn parse_langs_filters_non_lang_lines() {
        let s = "List of available languages\neng\n  warning: bad font!\nosd\n";
        let langs = parse_tesseract_langs(s);
        // "warning: bad font!" 含 ":" / 空格 → 过滤
        assert_eq!(langs, vec!["eng".to_string(), "osd".to_string()]);
    }

    #[test]
    fn parse_langs_empty_input() {
        assert!(parse_tesseract_langs("").is_empty());
    }

    #[test]
    fn parse_langs_only_header() {
        let s = "List of available languages in \"...tessdata/\" (0):\n";
        assert!(parse_tesseract_langs(s).is_empty());
    }

    #[test]
    fn parse_langs_no_header_first_line_is_lang() {
        // 极简 path：当 tesseract 直接输出语言名（不见 header 时）。
        let s = "eng\nosd\nchi_sim\n";
        let langs = parse_tesseract_langs(s);
        assert_eq!(langs.len(), 3);
        assert!(langs.contains(&"chi_sim".to_string()));
    }

    #[test]
    fn crop_to_gray8_full_bitmap_gray8_identity() {
        let bmp = make_gray8(4, 3, 200);
        let roi = OcrRoi::new(0, 0, 3, 2);
        let out = crop_to_gray8(&bmp, roi).unwrap();
        assert_eq!(out.width, 4);
        assert_eq!(out.height, 3);
        assert_eq!(out.format, PixelFormat::Gray8);
        assert!(out.pixels.iter().all(|&v| v == 200));
    }

    #[test]
    fn crop_to_gray8_subregion_gray8() {
        // 4x3 bitmap：第一行 0xff，其余 0x10。裁切 (0,1)~(3,2) = 2 行 × 4 列。
        let mut pixels = vec![0xff; 4];
        pixels.extend(vec![0x10; 8]);
        let bmp = Bitmap::from_raw(4, 3, 300.0, PixelFormat::Gray8, pixels).unwrap();
        let out = crop_to_gray8(&bmp, OcrRoi::new(0, 1, 3, 2)).unwrap();
        assert_eq!(out.width, 4);
        assert_eq!(out.height, 2);
        assert!(out.pixels.iter().all(|&v| v == 0x10));
    }

    #[test]
    fn crop_to_gray8_from_rgb8_luminance() {
        // R=255, G=0, B=0 → Y = 0.299*255 ≈ 76.245 → round=76
        let bmp = make_rgb8(2, 2, [255, 0, 0]);
        let out = crop_to_gray8(&bmp, OcrRoi::new(0, 0, 1, 1)).unwrap();
        assert_eq!(out.format, PixelFormat::Gray8);
        assert_eq!(out.pixels, vec![76; 4]);
    }

    #[test]
    fn crop_to_gray8_from_rgb8_white_stays_white() {
        let bmp = make_rgb8(2, 2, [255, 255, 255]);
        let out = crop_to_gray8(&bmp, OcrRoi::new(0, 0, 1, 1)).unwrap();
        assert_eq!(out.pixels, vec![255; 4]);
    }

    #[test]
    fn crop_to_gray8_from_rgba8_drops_alpha() {
        // RGB white + alpha 0；Gray8 输出仍是 255（alpha 不参与 luminance）。
        let bmp = make_rgba8(2, 2, [255, 255, 255, 0]);
        let out = crop_to_gray8(&bmp, OcrRoi::new(0, 0, 1, 1)).unwrap();
        assert_eq!(out.pixels, vec![255; 4]);
    }

    #[test]
    fn crop_to_gray8_single_pixel() {
        let bmp = make_gray8(5, 5, 88);
        let out = crop_to_gray8(&bmp, OcrRoi::new(2, 3, 2, 3)).unwrap();
        assert_eq!(out.width, 1);
        assert_eq!(out.height, 1);
        assert_eq!(out.pixels, vec![88]);
    }

    #[test]
    fn write_gray8_png_round_trips_dimensions() {
        let bmp = make_gray8(8, 5, 200);
        let tmp = ScopedTempFile::allocate("k2ocr-write-test", ".png").unwrap();
        write_gray8_png(&bmp, tmp.path()).unwrap();
        assert!(tmp.path().exists());
        let metadata = std::fs::metadata(tmp.path()).unwrap();
        assert!(metadata.len() > 0);
        // PNG magic header 89 50 4E 47
        let bytes = std::fs::read(tmp.path()).unwrap();
        assert_eq!(&bytes[0..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn write_gray8_png_rejects_non_gray() {
        let bmp = make_rgb8(2, 2, [10, 20, 30]);
        let tmp = ScopedTempFile::allocate("k2ocr-write-test", ".png").unwrap();
        let r = write_gray8_png(&bmp, tmp.path());
        assert!(matches!(r, Err(OcrError::BitmapEncoding(_))));
    }

    #[test]
    fn engine_default_executable_is_tesseract() {
        let e = TesseractCliEngine::new();
        assert_eq!(e.executable.to_string_lossy(), DEFAULT_TESSERACT_EXECUTABLE);
        assert!(e.tessdata_prefix.is_none());
    }

    #[test]
    fn engine_builder_with_executable_override() {
        let e = TesseractCliEngine::new().with_executable("/usr/local/bin/tesseract");
        assert_eq!(e.executable.to_string_lossy(), "/usr/local/bin/tesseract");
    }

    #[test]
    fn engine_builder_with_tessdata_prefix() {
        let e = TesseractCliEngine::new().with_tessdata_prefix("/opt/tessdata");
        assert_eq!(
            e.tessdata_prefix.as_ref().unwrap().to_string_lossy(),
            "/opt/tessdata"
        );
    }

    #[test]
    fn engine_name_is_tesseract_cli() {
        let e = TesseractCliEngine::new();
        assert_eq!(e.engine_name(), "tesseract-cli");
    }

    #[test]
    fn run_engine_not_found_maps_to_engine_not_found_error() {
        let e = TesseractCliEngine::new().with_executable("k2ocr_definitely_no_such_program_3719");
        let r = e.run(&["--version"]);
        assert!(matches!(r, Err(OcrError::EngineNotFound { .. })));
    }

    // ---- Step 11.8 P0-5: cancel 字段 + with_cancel + run() 双路径 ----

    #[test]
    fn engine_default_cancel_is_none() {
        let e = TesseractCliEngine::new();
        assert!(e.cancel.is_none());
    }

    #[test]
    fn engine_with_cancel_attaches_arc() {
        let flag = Arc::new(AtomicBool::new(false));
        let e = TesseractCliEngine::new().with_cancel(Arc::clone(&flag));
        let attached = e.cancel.as_ref().expect("cancel should be Some");
        // 翻外部 flag，engine 内 Arc 同步可见（同一份 AtomicBool）
        flag.store(true, Ordering::Relaxed);
        assert!(attached.load(Ordering::Relaxed));
    }

    #[test]
    fn run_blocking_path_used_when_cancel_none() {
        // cancel=None 走 run_blocking 分支：bin 不存在仍能正确返 EngineNotFound。
        let e = TesseractCliEngine::new()
            .with_executable("k2ocr_definitely_no_such_program_blocking_path_3719");
        let r = e.run(&["--version"]);
        assert!(matches!(r, Err(OcrError::EngineNotFound { .. })));
    }

    #[test]
    fn run_polling_path_returns_engine_not_found_when_bin_missing() {
        // cancel=Some 也走 spawn 分支；bin 不存在时 spawn 直接返 NotFound。
        let flag = Arc::new(AtomicBool::new(false));
        let e = TesseractCliEngine::new()
            .with_executable("k2ocr_definitely_no_such_program_polling_path_3719")
            .with_cancel(flag);
        let r = e.run(&["--version"]);
        assert!(matches!(r, Err(OcrError::EngineNotFound { .. })));
    }

    /// Step 11.8 P0-5：覆盖 spawn 轮询路径的 cancel kill 行为。
    ///
    /// 用平台内置长任务命令（Unix `/bin/sleep` 30s / Windows `ping` 100s）模拟
    /// 长 OCR 子进程；后台线程 100 ms 后翻 cancel，run() 应在 ≤ 2 s 内返
    /// [`OcrError::Cancelled`]，且子进程被 kill（不会卡满整个 timeout）。
    ///
    /// 设计要点：
    /// - run() 是 pub(crate)，可在本 mod 直接调；不必走 recognize 端到端
    /// - 不依赖 tesseract 二进制，纯走子进程取消语义
    /// - 200 ms ≪ 子进程自然耗时 (30 s)，证明 kill 真的发生
    #[test]
    fn run_polling_path_returns_cancelled_when_flipped_during_long_command() {
        let (bin, args): (&str, &[&str]) = if cfg!(windows) {
            ("ping", &["-n", "100", "127.0.0.1"])
        } else {
            ("/bin/sleep", &["30"])
        };
        let flag = Arc::new(AtomicBool::new(false));
        let engine = TesseractCliEngine::new()
            .with_executable(bin)
            .with_cancel(Arc::clone(&flag));

        // 后台线程 100 ms 后翻 cancel
        let flag_bg = Arc::clone(&flag);
        let bg = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag_bg.store(true, Ordering::Relaxed);
        });

        let started = std::time::Instant::now();
        let r = engine.run(args);
        let elapsed = started.elapsed();
        bg.join().unwrap();

        // 主断言：run() 返 Cancelled（不是 EngineExitNonZero / EngineIo）
        assert!(
            matches!(r, Err(OcrError::Cancelled)),
            "expected Cancelled, got {r:?}"
        );
        // wall clock 必须远小于子进程自然耗时（>= 30 s 时表示 kill 没生效）
        assert!(
            elapsed < Duration::from_secs(5),
            "cancel took too long: {elapsed:?}"
        );
        // cancel flag 自然仍为 true
        assert!(flag.load(Ordering::Relaxed));
    }

    /// Step 11.8 P0-5：未翻 cancel 时 run() 正常 Ok 退出，不卡死轮询。
    ///
    /// 用平台内置短命令（Unix `/bin/echo` / Windows `cmd /c echo`），exit code 0
    /// 立即返回 Ok(stdout)。验证 spawn 轮询路径不会因 cancel 字段存在而退化。
    #[test]
    fn run_polling_path_finishes_normally_when_not_cancelled() {
        let (bin, args): (&str, &[&str]) = if cfg!(windows) {
            ("cmd", &["/c", "echo", "ok"])
        } else {
            ("/bin/echo", &["ok"])
        };
        let flag = Arc::new(AtomicBool::new(false));
        let engine = TesseractCliEngine::new()
            .with_executable(bin)
            .with_cancel(Arc::clone(&flag));

        let started = std::time::Instant::now();
        let r = engine.run(args);
        let elapsed = started.elapsed();

        // 子进程正常退出 → Ok(stdout)
        assert!(r.is_ok(), "expected Ok, got {r:?}");
        // 短命令 < 5 s
        assert!(elapsed < Duration::from_secs(5));
        // cancel flag 始终 false
        assert!(!flag.load(Ordering::Relaxed));
        // stdout 含 "ok"
        let stdout = r.unwrap();
        let s = String::from_utf8_lossy(&stdout);
        assert!(s.contains("ok"), "stdout missing 'ok': {s:?}");
    }
}
