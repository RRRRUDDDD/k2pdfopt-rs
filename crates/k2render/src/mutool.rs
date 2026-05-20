//! mutool 后端实现：通过 `mutool draw -F pam -o -` stdout 管道渲染 PDF。
//!
//! 默认走 stdout PAM 管道（ADR-015 决策）。Spike B 实测对比临时 PNG 快约 1.47x；
//! 详见 `spikes/mutool-backend/REPORT.md` 与 `docs/adr/ADR-015-mutool-pipeline.md`。
//!
//! 关键 mutool 输出格式（实测于 mutool 1.27.0）：
//! - `mutool info <pdf>` 顶部含 `Pages: N` 行；`Mediaboxes (N):` 节内每页一行
//!   形如 `\t<idx>\t(<obj>):\t[ llx lly urx ury ]`。
//! - `mutool draw -F pam` 输出 P7 PAM 头（`WIDTH/HEIGHT/DEPTH/MAXVAL/TUPLTYPE/ENDHDR\n`）
//!   后接 `WIDTH * HEIGHT * DEPTH` 字节的二进制 RGB_ALPHA 像素流。
//! - 加密失败时 stderr 含 `cannot authenticate password`，exit code = 1。

use crate::renderer::{DocumentRenderer, RenderError};
use anyhow::{Context, Result};
use k2types::{Bitmap, BitmapPage, PixelFormat};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

/// 构造 [`MutoolRenderer`] 的可选参数。
pub struct MutoolOptions {
    /// PDF 解密密码（mutool 通过 `-p <pw>` 接收）。
    pub password: Option<String>,
    /// 自定义 mutool 二进制路径；默认使用 PATH 中的 `mutool`。
    pub binary: PathBuf,
}

impl Default for MutoolOptions {
    fn default() -> Self {
        Self {
            password: None,
            binary: PathBuf::from("mutool"),
        }
    }
}

/// 通过 `mutool draw` 子进程渲染 PDF 的实现。
///
/// 构造时探测 mutool 可用 + 读 `Pages:` 行得到 page_count；首次查询 [`Self::page_size`]
/// 时全量读取 mediabox 写入 `page_sizes` 缓存（[`OnceLock`]）。渲染走 stdout PAM 管道。
#[derive(Debug)]
pub struct MutoolRenderer {
    pdf_path: PathBuf,
    page_count: usize,
    password: Option<String>,
    binary: PathBuf,
    page_sizes: OnceLock<Vec<(f32, f32)>>,
}

impl MutoolRenderer {
    /// 默认构造：使用 PATH 中的 `mutool`，不传密码。
    pub fn new<P: AsRef<Path>>(pdf_path: P) -> Result<Self> {
        Self::with_options(pdf_path, MutoolOptions::default())
    }

    /// 完整构造：允许传密码 + 自定义 mutool 路径。
    pub fn with_options<P: AsRef<Path>>(pdf_path: P, opts: MutoolOptions) -> Result<Self> {
        let pdf_path = pdf_path.as_ref().to_path_buf();
        if !pdf_path.exists() {
            anyhow::bail!("PDF file not found: {}", pdf_path.display());
        }
        Self::check_binary(&opts.binary)?;
        let password = opts.password;
        let page_count = Self::query_page_count(&opts.binary, &pdf_path, password.as_deref())?;
        Ok(Self {
            pdf_path,
            page_count,
            password,
            binary: opts.binary,
            page_sizes: OnceLock::new(),
        })
    }

    fn check_binary(binary: &Path) -> Result<()> {
        match Command::new(binary).arg("-v").output() {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(RenderError::BinaryNotFound(binary.display().to_string()).into())
            }
            Err(e) => Err(e).context(format!(
                "failed to invoke mutool binary `{}`",
                binary.display()
            )),
        }
    }

    fn query_page_count(binary: &Path, pdf: &Path, password: Option<&str>) -> Result<usize> {
        let out = Self::run_mutool_info(binary, pdf, password)?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            if let Some(rest) = line.strip_prefix("Pages:") {
                return rest
                    .trim()
                    .parse::<usize>()
                    .context("parse `Pages:` value from mutool info output");
            }
        }
        Err(RenderError::InvalidSource(format!(
            "no `Pages:` line in mutool info output for {}",
            pdf.display()
        ))
        .into())
    }

    /// 执行 `mutool info [-p pw] <pdf>` 并校验退出码。
    fn run_mutool_info(binary: &Path, pdf: &Path, password: Option<&str>) -> Result<Output> {
        run_mutool_info(binary, pdf, password)
    }

    fn check_mutool_exit(out: &Output, pdf: &Path) -> Result<()> {
        check_mutool_exit(out, pdf)
    }

    fn ensure_page_sizes(&self) -> Result<&[(f32, f32)]> {
        if self.page_sizes.get().is_none() {
            let sizes = self.query_all_page_sizes()?;
            // race 时 set 返回 Err；不要紧，下面 get 会拿到另一线程写入的结果
            let _ = self.page_sizes.set(sizes);
        }
        self.page_sizes.get().map(Vec::as_slice).ok_or_else(|| {
            RenderError::InvalidSource("page_sizes cache invariant violated".into()).into()
        })
    }

    fn query_all_page_sizes(&self) -> Result<Vec<(f32, f32)>> {
        let out = Self::run_mutool_info(&self.binary, &self.pdf_path, self.password.as_deref())?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut by_index: Vec<Option<(f32, f32)>> = vec![None; self.page_count];
        let mut in_mediaboxes = false;
        for raw_line in stdout.lines() {
            let trimmed = raw_line.trim_end();
            if trimmed.starts_with("Mediaboxes") {
                in_mediaboxes = true;
                continue;
            }
            if !in_mediaboxes {
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            // 形如 `\t1\t(5 0 R):\t[ ... ]`：第一字符必须是空白
            if !trimmed.starts_with(char::is_whitespace) {
                in_mediaboxes = false;
                continue;
            }
            if let Some((page_num, w, h)) = parse_mediabox_line(trimmed) {
                if let Some(slot) = by_index.get_mut(page_num.saturating_sub(1)) {
                    *slot = Some((w, h));
                }
            }
        }
        let mut result = Vec::with_capacity(self.page_count);
        for (i, opt) in by_index.into_iter().enumerate() {
            match opt {
                Some(sz) => result.push(sz),
                None => {
                    return Err(RenderError::InvalidSource(format!(
                        "no mediabox for page {} in mutool info output",
                        i + 1
                    ))
                    .into());
                }
            }
        }
        Ok(result)
    }

    fn validate_index(&self, page_index: usize) -> Result<()> {
        if page_index >= self.page_count {
            return Err(RenderError::PageOutOfRange {
                requested: page_index,
                total: self.page_count,
            }
            .into());
        }
        Ok(())
    }
}

impl DocumentRenderer for MutoolRenderer {
    fn page_count(&self) -> Result<usize> {
        Ok(self.page_count)
    }

    fn page_size(&self, page_index: usize) -> Result<(f32, f32)> {
        self.validate_index(page_index)?;
        let cached = self.ensure_page_sizes()?;
        cached.get(page_index).copied().ok_or_else(|| {
            RenderError::InvalidSource(format!("page_size missing for index {page_index}")).into()
        })
    }

    fn render_page(&self, page_index: usize, dpi: f32) -> Result<BitmapPage> {
        self.validate_index(page_index)?;
        if !dpi.is_finite() || dpi <= 0.0 {
            anyhow::bail!("invalid dpi: {dpi}");
        }
        let one_based = page_index_to_arg(page_index)?;
        let mut cmd = Command::new(&self.binary);
        cmd.args(["draw", "-F", "pam", "-o", "-", "-r"])
            .arg(format_dpi(dpi));
        if let Some(pw) = &self.password {
            cmd.arg("-p").arg(pw);
        }
        cmd.arg(&self.pdf_path).arg(&one_based);
        let out = cmd.output().context("mutool draw subprocess failed")?;
        Self::check_mutool_exit(&out, &self.pdf_path)?;
        let bitmap = decode_pam(&out.stdout, dpi)?;
        let source_size_pt = self.page_size(page_index).unwrap_or_else(|_| {
            // mediabox 解析失败时退化用 bitmap 尺寸 / DPI 推算
            let safe_dpi = dpi.max(f32::EPSILON);
            (
                (bitmap.width as f32) * 72.0 / safe_dpi,
                (bitmap.height as f32) * 72.0 / safe_dpi,
            )
        });
        Ok(BitmapPage {
            page_index,
            bitmap,
            source_dpi: dpi,
            source_size_pt,
            rotation: 0.0,
        })
    }
}

fn page_index_to_arg(page_index: usize) -> Result<String> {
    let one_based = page_index
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("page_index overflow: {}", page_index))?;
    Ok(one_based.to_string())
}

fn format_dpi(dpi: f32) -> String {
    // mutool -r 接受整数；非整数 DPI 四舍五入再传
    let rounded = if dpi.is_finite() {
        dpi.round() as i64
    } else {
        1
    };
    rounded.max(1).to_string()
}

fn parse_mediabox_line(line: &str) -> Option<(usize, f32, f32)> {
    parse_mediabox_line_pub(line)
}

/// 解析 `\t<idx>\t(<obj>):\t[ llx lly urx ury ]` 形式的 mediabox 行，
/// 返回 `(idx_1based, width_pt, height_pt)`。`pub(crate)` 暴露给 [`crate::pdfinfo`] 复用。
pub(crate) fn parse_mediabox_line_pub(line: &str) -> Option<(usize, f32, f32)> {
    // 形如 `\t1\t(5 0 R):\t[ 0 0 595 842 ]`
    let trimmed = line.trim_start();
    let mut parts = trimmed.splitn(2, |c: char| c.is_whitespace());
    let page_num: usize = parts.next()?.trim().parse().ok()?;
    let rest = parts.next()?;
    let bracket_start = rest.find('[')?;
    let bracket_end = rest[bracket_start..].find(']')? + bracket_start;
    let inside = &rest[bracket_start + 1..bracket_end];
    let nums: Vec<f32> = inside
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    if nums.len() != 4 {
        return None;
    }
    let width = (nums[2] - nums[0]).abs();
    let height = (nums[3] - nums[1]).abs();
    Some((page_num, width, height))
}

/// 执行 `mutool info [-p pw] <pdf>` 并校验退出码。`pub(crate)` 暴露给 [`crate::pdfinfo`] 复用。
pub(crate) fn run_mutool_info(binary: &Path, pdf: &Path, password: Option<&str>) -> Result<Output> {
    let mut cmd = Command::new(binary);
    cmd.arg("info");
    if let Some(pw) = password {
        cmd.arg("-p").arg(pw);
    }
    cmd.arg(pdf);
    let out = cmd.output().context("mutool info subprocess failed")?;
    check_mutool_exit(&out, pdf)?;
    Ok(out)
}

/// 检测 mutool 子进程退出码。非 0 时根据 stderr 关键字区分 [`RenderError::Encrypted`] /
/// [`RenderError::SubprocessFailed`]。`pub(crate)` 暴露给 [`crate::pdfinfo`] 复用。
pub(crate) fn check_mutool_exit(out: &Output, pdf: &Path) -> Result<()> {
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("password") || lower.contains("authenticate") || lower.contains("encrypt") {
        return Err(RenderError::Encrypted {
            path: pdf.display().to_string(),
        }
        .into());
    }
    Err(RenderError::SubprocessFailed {
        code: out.status.code().unwrap_or(-1),
        stderr: stderr.into_owned(),
    }
    .into())
}

/// PAM (P7) 头部解析 + 像素数据剥离。
///
/// PAM 头是 ASCII 行：`P7`、`WIDTH N`、`HEIGHT N`、`DEPTH N`、`MAXVAL N`、`TUPLTYPE <name>`，
/// 以 `ENDHDR\n` 结尾，其后是 `WIDTH * HEIGHT * DEPTH` 字节的二进制像素流（行优先）。
///
/// 当前支持组合：
/// - `DEPTH 1` → [`PixelFormat::Gray8`]
/// - `DEPTH 3` → [`PixelFormat::Rgb8`]
/// - `DEPTH 4 / TUPLTYPE RGB_ALPHA` → [`PixelFormat::Rgba8`]（mutool 默认）
fn decode_pam(data: &[u8], dpi: f32) -> Result<Bitmap> {
    let header_end = data
        .windows(b"ENDHDR\n".len())
        .position(|w| w == b"ENDHDR\n")
        .ok_or_else(|| RenderError::InvalidPam("missing ENDHDR marker".into()))?;
    let header_bytes = &data[..header_end];
    let header_str = std::str::from_utf8(header_bytes)
        .map_err(|e| RenderError::InvalidPam(format!("header not UTF-8: {e}")))?;
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut depth: Option<u32> = None;
    let mut maxval: Option<u32> = None;
    let mut tupltype: Option<String> = None;
    let mut saw_magic = false;
    for line in header_str.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !saw_magic {
            if line != "P7" {
                return Err(RenderError::InvalidPam(format!(
                    "unexpected magic: `{line}` (want `P7`)"
                ))
                .into());
            }
            saw_magic = true;
            continue;
        }
        let (key, val) = line
            .split_once(char::is_whitespace)
            .map(|(k, v)| (k, v.trim()))
            .unwrap_or((line, ""));
        match key {
            "WIDTH" => width = val.parse().ok(),
            "HEIGHT" => height = val.parse().ok(),
            "DEPTH" => depth = val.parse().ok(),
            "MAXVAL" => maxval = val.parse::<u32>().ok(),
            "TUPLTYPE" => tupltype = Some(val.to_string()),
            "ENDHDR" => break,
            _ => {}
        }
    }
    let width = width.ok_or_else(|| RenderError::InvalidPam("missing WIDTH".into()))?;
    let height = height.ok_or_else(|| RenderError::InvalidPam("missing HEIGHT".into()))?;
    let depth = depth.ok_or_else(|| RenderError::InvalidPam("missing DEPTH".into()))?;
    let maxval = maxval.ok_or_else(|| RenderError::InvalidPam("missing MAXVAL".into()))?;
    if maxval != 255 {
        return Err(RenderError::InvalidPam(format!(
            "unsupported MAXVAL={maxval} (only 255 / 8-bit supported)"
        ))
        .into());
    }
    let format = match (depth, tupltype.as_deref()) {
        (1, _) => PixelFormat::Gray8,
        (3, _) => PixelFormat::Rgb8,
        (4, _) => PixelFormat::Rgba8,
        _ => {
            return Err(RenderError::InvalidPam(format!(
                "unsupported DEPTH={depth} TUPLTYPE={tupltype:?}"
            ))
            .into());
        }
    };
    let body_start = header_end + b"ENDHDR\n".len();
    let body = data
        .get(body_start..)
        .ok_or_else(|| RenderError::InvalidPam("body missing".into()))?;
    Bitmap::from_raw(width, height, dpi, format, body.to_vec())
        .map_err(|e| anyhow::Error::new(e).context("PAM body did not match declared dimensions"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn parse_mediabox_a4() {
        let (idx, w, h) = parse_mediabox_line("\t1\t(5 0 R):\t[ 0 0 595 842 ]").unwrap();
        assert_eq!(idx, 1);
        assert!((w - 595.0).abs() < 1e-3);
        assert!((h - 842.0).abs() < 1e-3);
    }

    #[test]
    fn parse_mediabox_offset() {
        let (idx, w, h) = parse_mediabox_line("\t3\t(9 0 R):\t[ 10 20 605 862 ]").unwrap();
        assert_eq!(idx, 3);
        assert!((w - 595.0).abs() < 1e-3);
        assert!((h - 842.0).abs() < 1e-3);
    }

    #[test]
    fn parse_mediabox_invalid_returns_none() {
        assert!(parse_mediabox_line("Pages: 5").is_none());
        // 不到 4 个数字
        assert!(parse_mediabox_line("\t1\tfoo\t[ 1 2 3 ]").is_none());
        // 缺右括号
        assert!(parse_mediabox_line("\t1\tfoo\t[ 0 0 595 842").is_none());
    }

    #[test]
    fn decode_pam_minimal_gray() {
        let header = b"P7\nWIDTH 2\nHEIGHT 2\nDEPTH 1\nMAXVAL 255\nTUPLTYPE GRAYSCALE\nENDHDR\n";
        let body = [0u8, 64, 128, 255];
        let mut blob = Vec::with_capacity(header.len() + body.len());
        blob.extend_from_slice(header);
        blob.extend_from_slice(&body);
        let bmp = decode_pam(&blob, 150.0).unwrap();
        assert_eq!(bmp.width, 2);
        assert_eq!(bmp.height, 2);
        assert_eq!(bmp.format, PixelFormat::Gray8);
        assert_eq!(bmp.pixels, vec![0, 64, 128, 255]);
        assert!((bmp.dpi - 150.0).abs() < 1e-3);
    }

    #[test]
    fn decode_pam_rgba() {
        let mut blob = Vec::new();
        blob.extend_from_slice(
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n",
        );
        blob.extend_from_slice(&[0xFF, 0x80, 0x40, 0xFF]);
        let bmp = decode_pam(&blob, 300.0).unwrap();
        assert_eq!(bmp.format, PixelFormat::Rgba8);
        assert_eq!(bmp.pixels.len(), 4);
    }

    #[test]
    fn decode_pam_missing_endhdr() {
        let blob = b"P7\nWIDTH 2\nHEIGHT 2\nDEPTH 1\nMAXVAL 255\nTUPLTYPE GRAYSCALE\n";
        let err = decode_pam(blob, 100.0).unwrap_err();
        let typed = err.downcast_ref::<RenderError>().unwrap();
        assert!(matches!(typed, RenderError::InvalidPam(_)));
    }

    #[test]
    fn decode_pam_bad_magic() {
        let mut blob = Vec::new();
        blob.extend_from_slice(
            b"P6\nWIDTH 1\nHEIGHT 1\nDEPTH 1\nMAXVAL 255\nTUPLTYPE GRAYSCALE\nENDHDR\n",
        );
        blob.push(0);
        let err = decode_pam(&blob, 100.0).unwrap_err();
        let typed = err.downcast_ref::<RenderError>().unwrap();
        assert!(matches!(typed, RenderError::InvalidPam(_)));
    }

    #[test]
    fn decode_pam_unsupported_depth() {
        let mut blob = Vec::new();
        blob.extend_from_slice(
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 2\nMAXVAL 255\nTUPLTYPE GRAYSCALE_ALPHA\nENDHDR\n",
        );
        blob.extend_from_slice(&[0, 0]);
        let err = decode_pam(&blob, 100.0).unwrap_err();
        let typed = err.downcast_ref::<RenderError>().unwrap();
        assert!(matches!(typed, RenderError::InvalidPam(_)));
    }

    #[test]
    fn decode_pam_unsupported_maxval() {
        let mut blob = Vec::new();
        blob.extend_from_slice(
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 1\nMAXVAL 65535\nTUPLTYPE GRAYSCALE\nENDHDR\n",
        );
        blob.extend_from_slice(&[0, 0]);
        let err = decode_pam(&blob, 100.0).unwrap_err();
        let typed = err.downcast_ref::<RenderError>().unwrap();
        assert!(matches!(typed, RenderError::InvalidPam(_)));
    }

    #[test]
    fn format_dpi_handles_fraction_and_invalid() {
        assert_eq!(format_dpi(150.0), "150");
        assert_eq!(format_dpi(150.4), "150");
        assert_eq!(format_dpi(150.6), "151");
        assert_eq!(format_dpi(0.0), "1");
        assert_eq!(format_dpi(-1.0), "1");
        assert_eq!(format_dpi(f32::NAN), "1");
        assert_eq!(format_dpi(f32::INFINITY), "1");
    }

    #[test]
    fn page_index_to_arg_basic() {
        assert_eq!(page_index_to_arg(0).unwrap(), "1");
        assert_eq!(page_index_to_arg(42).unwrap(), "43");
    }
}
