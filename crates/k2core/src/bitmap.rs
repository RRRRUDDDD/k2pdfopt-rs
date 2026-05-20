//! Bitmap I/O —— PNG / PAM / PPM 读写。
//!
//! ## 设计
//!
//! - **PNG**：`image` crate 0.25 + 仅启用 `png` feature。覆盖 Gray8 / Rgb8 / Rgba8 三种格式。
//! - **PAM**：与 mutool stdout 管道（[`k2render::mutool`]）的输入格式兼容；
//!   完整实现 `P7` 头解析 + Gray8 / RGB / RGB_ALPHA 写入。
//! - **PPM (P6)**：仅二进制 RGB8（Gray8 输入会先升 RGB）。
//!
//! ## 错误模型
//!
//! 用 [`BitmapIoError`]（thiserror 派生）封装"格式错误 + IO 错误 + 像素布局错误"。
//! 库内不引入 `anyhow`，让上游 crate 决定如何包装（典型用 `anyhow::Result<T>` 接口）。
//!
//! 来源 C 文件：`willuslib/bmp.c` (lines 528-540, bmp_read_png / bmp_write_png 等)。

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use image::{ColorType, DynamicImage, GenericImageView};
use k2types::{Bitmap, BitmapError, PixelFormat};
use thiserror::Error;

/// PNG / PAM / PPM 读写过程中的可恢复错误。
#[derive(Debug, Error)]
pub enum BitmapIoError {
    /// 文件系统 / 输入输出错误（打开、读写失败）。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// `image` crate 解码 / 编码失败（PNG 格式异常等）。
    #[error("image codec error: {0}")]
    Image(#[from] image::ImageError),

    /// PAM 头字段缺失 / 格式不识别。
    #[error("invalid PAM header: {0}")]
    InvalidPam(String),

    /// PAM 头声明的像素字节数与实际不符。
    #[error("PAM pixel size mismatch: expected {expected}, got {actual}")]
    PamSizeMismatch { expected: usize, actual: usize },

    /// 不支持的源像素格式（如 PNG 含 16-bit 通道，或不支持的颜色空间）。
    #[error("unsupported color format: {0}")]
    UnsupportedFormat(String),

    /// Bitmap 构造期内部错误（宽高溢出、字节数不匹配等）。
    #[error("bitmap structure error: {0}")]
    Bitmap(#[from] BitmapError),
}

// ============================================================================
// PNG
// ============================================================================

/// 从路径读 PNG 文件为 `Bitmap`。
///
/// - 8-bit 灰度 → `PixelFormat::Gray8`
/// - 8-bit RGB / 调色板 → `PixelFormat::Rgb8`
/// - 8-bit RGBA → `PixelFormat::Rgba8`
/// - 16-bit 等高位深统一向下转换到 8-bit（按 `image` crate 行为）
///
/// 不解析 PNG 中的 DPI 元数据；调用方需自行提供 `dpi` 参数。
pub fn read_png<P: AsRef<Path>>(path: P, dpi: f32) -> Result<Bitmap, BitmapIoError> {
    let img = image::open(path.as_ref())?;
    dynamic_image_to_bitmap(&img, dpi)
}

/// 把 `DynamicImage` 转成 `Bitmap`（私有 helper，供 PNG / 其他后端复用）。
fn dynamic_image_to_bitmap(img: &DynamicImage, dpi: f32) -> Result<Bitmap, BitmapIoError> {
    let (w, h) = img.dimensions();
    match img.color() {
        ColorType::L8 => {
            let buf = img.to_luma8();
            Bitmap::from_raw(w, h, dpi, PixelFormat::Gray8, buf.into_raw()).map_err(Into::into)
        }
        ColorType::Rgb8 => {
            let buf = img.to_rgb8();
            Bitmap::from_raw(w, h, dpi, PixelFormat::Rgb8, buf.into_raw()).map_err(Into::into)
        }
        ColorType::Rgba8 => {
            let buf = img.to_rgba8();
            Bitmap::from_raw(w, h, dpi, PixelFormat::Rgba8, buf.into_raw()).map_err(Into::into)
        }
        // 16-bit / La / 等 → 统一向下到 Rgb8 / Rgba8 / Luma8
        ColorType::La8 => {
            let buf = img.to_rgba8();
            Bitmap::from_raw(w, h, dpi, PixelFormat::Rgba8, buf.into_raw()).map_err(Into::into)
        }
        ColorType::L16 | ColorType::La16 => {
            let buf = img.to_luma8();
            Bitmap::from_raw(w, h, dpi, PixelFormat::Gray8, buf.into_raw()).map_err(Into::into)
        }
        ColorType::Rgb16 => {
            let buf = img.to_rgb8();
            Bitmap::from_raw(w, h, dpi, PixelFormat::Rgb8, buf.into_raw()).map_err(Into::into)
        }
        ColorType::Rgba16 | ColorType::Rgb32F | ColorType::Rgba32F => {
            let buf = img.to_rgba8();
            Bitmap::from_raw(w, h, dpi, PixelFormat::Rgba8, buf.into_raw()).map_err(Into::into)
        }
        other => Err(BitmapIoError::UnsupportedFormat(format!("{other:?}"))),
    }
}

/// 把 `Bitmap` 写入 PNG 文件。
pub fn write_png<P: AsRef<Path>>(bitmap: &Bitmap, path: P) -> Result<(), BitmapIoError> {
    let color = match bitmap.format {
        PixelFormat::Gray8 => ColorType::L8,
        PixelFormat::Rgb8 => ColorType::Rgb8,
        PixelFormat::Rgba8 => ColorType::Rgba8,
    };
    image::save_buffer(
        path.as_ref(),
        &bitmap.pixels,
        bitmap.width,
        bitmap.height,
        color,
    )?;
    Ok(())
}

// ============================================================================
// PAM (Netpbm P7)
// ============================================================================

/// 把 `Bitmap` 写为 PAM（P7）字节流，写入任意实现 `Write` 的 sink。
///
/// 头部固定字段：`WIDTH / HEIGHT / DEPTH / MAXVAL=255 / TUPLTYPE`。
/// 像素数据按 `pixels` 原始字节序输出（无字节序转换）。
pub fn write_pam<W: Write>(bitmap: &Bitmap, writer: &mut W) -> Result<(), BitmapIoError> {
    let (depth, tupltype) = match bitmap.format {
        PixelFormat::Gray8 => (1u32, "GRAYSCALE"),
        PixelFormat::Rgb8 => (3, "RGB"),
        PixelFormat::Rgba8 => (4, "RGB_ALPHA"),
    };
    writeln!(writer, "P7")?;
    writeln!(writer, "WIDTH {}", bitmap.width)?;
    writeln!(writer, "HEIGHT {}", bitmap.height)?;
    writeln!(writer, "DEPTH {depth}")?;
    writeln!(writer, "MAXVAL 255")?;
    writeln!(writer, "TUPLTYPE {tupltype}")?;
    writeln!(writer, "ENDHDR")?;
    writer.write_all(&bitmap.pixels)?;
    Ok(())
}

/// 从内存字节流解析 PAM (P7)。完整支持 Gray8 (DEPTH=1) / RGB (DEPTH=3) /
/// RGB_ALPHA (DEPTH=4)。MAXVAL 必须为 255。
///
/// 不在 IO 边界做"管道剥离"；调用方应在调用前剔除 mutool 子进程的前导 stderr。
pub fn read_pam(bytes: &[u8], dpi: f32) -> Result<Bitmap, BitmapIoError> {
    let endhdr_marker = b"\nENDHDR\n";
    let header_end = find_subslice(bytes, endhdr_marker)
        .ok_or_else(|| BitmapIoError::InvalidPam("missing ENDHDR".into()))?;
    let body_start = header_end + endhdr_marker.len();
    let header_bytes = &bytes[..header_end];
    let body = &bytes[body_start..];

    let header_str = std::str::from_utf8(header_bytes)
        .map_err(|_| BitmapIoError::InvalidPam("header is not UTF-8".into()))?;

    let mut width = None;
    let mut height = None;
    let mut depth = None;
    let mut maxval = None;
    let mut tupltype: Option<String> = None;
    let mut saw_magic = false;
    for line in header_str.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "P7" {
            saw_magic = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("WIDTH") {
            width = Some(parse_pam_uint(rest, "WIDTH")?);
        } else if let Some(rest) = line.strip_prefix("HEIGHT") {
            height = Some(parse_pam_uint(rest, "HEIGHT")?);
        } else if let Some(rest) = line.strip_prefix("DEPTH") {
            depth = Some(parse_pam_uint(rest, "DEPTH")?);
        } else if let Some(rest) = line.strip_prefix("MAXVAL") {
            maxval = Some(parse_pam_uint(rest, "MAXVAL")?);
        } else if let Some(rest) = line.strip_prefix("TUPLTYPE") {
            tupltype = Some(rest.trim().to_string());
        }
        // 其他未识别字段（如 comments `#`）静默忽略
    }

    if !saw_magic {
        return Err(BitmapIoError::InvalidPam("missing P7 magic".into()));
    }
    let width = width.ok_or_else(|| BitmapIoError::InvalidPam("missing WIDTH".into()))?;
    let height = height.ok_or_else(|| BitmapIoError::InvalidPam("missing HEIGHT".into()))?;
    let depth = depth.ok_or_else(|| BitmapIoError::InvalidPam("missing DEPTH".into()))?;
    let maxval = maxval.ok_or_else(|| BitmapIoError::InvalidPam("missing MAXVAL".into()))?;
    if maxval != 255 {
        return Err(BitmapIoError::InvalidPam(format!(
            "unsupported MAXVAL {maxval}, only 255 is supported"
        )));
    }

    let format = match depth {
        1 => PixelFormat::Gray8,
        3 => PixelFormat::Rgb8,
        4 => PixelFormat::Rgba8,
        other => {
            return Err(BitmapIoError::InvalidPam(format!(
                "unsupported DEPTH {other}, expected 1/3/4"
            )));
        }
    };

    // 可选：TUPLTYPE 与 DEPTH 一致性校验（仅警告级别，不致命）
    if let Some(t) = tupltype.as_deref() {
        let consistent = matches!(
            (depth, t),
            (1, "GRAYSCALE") | (1, "BLACKANDWHITE") | (3, "RGB") | (4, "RGB_ALPHA") | (4, "CMYK")
        );
        if !consistent {
            // 静默接受 - mutool 不同版本输出可能略有差异
        }
    }

    let bpp = format.bytes_per_pixel();
    let expected = (width as usize) * (height as usize) * bpp;
    if body.len() < expected {
        return Err(BitmapIoError::PamSizeMismatch {
            expected,
            actual: body.len(),
        });
    }
    let pixels = body[..expected].to_vec();
    Bitmap::from_raw(width, height, dpi, format, pixels).map_err(Into::into)
}

/// 把 `Bitmap` 写入 PAM 文件（路径方式）。
pub fn write_pam_file<P: AsRef<Path>>(bitmap: &Bitmap, path: P) -> Result<(), BitmapIoError> {
    let f = File::create(path.as_ref())?;
    let mut w = BufWriter::new(f);
    write_pam(bitmap, &mut w)?;
    w.flush()?;
    Ok(())
}

/// 从 PAM 文件读取（路径方式）。
pub fn read_pam_file<P: AsRef<Path>>(path: P, dpi: f32) -> Result<Bitmap, BitmapIoError> {
    let mut f = BufReader::new(File::open(path.as_ref())?);
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    read_pam(&buf, dpi)
}

// ============================================================================
// PPM (Netpbm P6) - 仅 binary RGB
// ============================================================================

/// 把 `Bitmap` 写为 PPM P6（二进制 RGB）。
///
/// - `Gray8`：自动升 RGB（R=G=B=灰度值）
/// - `Rgb8`：直接写
/// - `Rgba8`：丢弃 alpha 通道（PPM 不支持透明）
pub fn write_ppm<W: Write>(bitmap: &Bitmap, writer: &mut W) -> Result<(), BitmapIoError> {
    writeln!(writer, "P6")?;
    writeln!(writer, "{} {}", bitmap.width, bitmap.height)?;
    writeln!(writer, "255")?;
    match bitmap.format {
        PixelFormat::Rgb8 => writer.write_all(&bitmap.pixels)?,
        PixelFormat::Gray8 => {
            for &g in &bitmap.pixels {
                writer.write_all(&[g, g, g])?;
            }
        }
        PixelFormat::Rgba8 => {
            for chunk in bitmap.pixels.chunks_exact(4) {
                writer.write_all(&chunk[..3])?;
            }
        }
    }
    Ok(())
}

/// 把 `Bitmap` 写入 PPM 文件（路径方式）。
pub fn write_ppm_file<P: AsRef<Path>>(bitmap: &Bitmap, path: P) -> Result<(), BitmapIoError> {
    let f = File::create(path.as_ref())?;
    let mut w = BufWriter::new(f);
    write_ppm(bitmap, &mut w)?;
    w.flush()?;
    Ok(())
}

// ============================================================================
// 内部 helpers
// ============================================================================

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_pam_uint(rest: &str, field: &'static str) -> Result<u32, BitmapIoError> {
    rest.trim()
        .parse::<u32>()
        .map_err(|e| BitmapIoError::InvalidPam(format!("invalid {field}: {e}")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn synthetic_gray(width: u32, height: u32) -> Bitmap {
        let mut b = Bitmap::new(width, height, 100.0, PixelFormat::Gray8).unwrap();
        for y in 0..height {
            let row = b.row_mut(y).unwrap();
            for (x, px) in row.iter_mut().enumerate() {
                *px = ((x as u32 + y) % 256) as u8;
            }
        }
        b
    }

    fn synthetic_rgb(width: u32, height: u32) -> Bitmap {
        let mut b = Bitmap::new(width, height, 200.0, PixelFormat::Rgb8).unwrap();
        for y in 0..height {
            let row = b.row_mut(y).unwrap();
            for (i, chunk) in row.chunks_exact_mut(3).enumerate() {
                chunk[0] = (i as u32 % 256) as u8;
                chunk[1] = ((i as u32 + 50) % 256) as u8;
                chunk[2] = ((i as u32 + 100) % 256) as u8;
            }
        }
        b
    }

    fn synthetic_rgba(width: u32, height: u32) -> Bitmap {
        let mut b = Bitmap::new(width, height, 150.0, PixelFormat::Rgba8).unwrap();
        for y in 0..height {
            let row = b.row_mut(y).unwrap();
            for (i, chunk) in row.chunks_exact_mut(4).enumerate() {
                chunk[0] = ((i + (y as usize)) % 256) as u8;
                chunk[1] = ((i * 2 + (y as usize)) % 256) as u8;
                chunk[2] = ((i * 3) % 256) as u8;
                chunk[3] = 255;
            }
        }
        b
    }

    // ---------- PNG roundtrip ----------

    #[test]
    fn png_gray8_roundtrip() {
        let tmp = std::env::temp_dir().join("k2core_png_gray8.png");
        let orig = synthetic_gray(8, 5);
        write_png(&orig, &tmp).unwrap();
        let loaded = read_png(&tmp, 100.0).unwrap();
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(loaded.format, PixelFormat::Gray8);
        assert_eq!(loaded.width, 8);
        assert_eq!(loaded.height, 5);
        assert_eq!(loaded.pixels, orig.pixels);
    }

    #[test]
    fn png_rgb8_roundtrip() {
        let tmp = std::env::temp_dir().join("k2core_png_rgb8.png");
        let orig = synthetic_rgb(6, 4);
        write_png(&orig, &tmp).unwrap();
        let loaded = read_png(&tmp, 200.0).unwrap();
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(loaded.format, PixelFormat::Rgb8);
        assert_eq!(loaded.pixels, orig.pixels);
    }

    #[test]
    fn png_rgba8_roundtrip() {
        let tmp = std::env::temp_dir().join("k2core_png_rgba8.png");
        let orig = synthetic_rgba(4, 3);
        write_png(&orig, &tmp).unwrap();
        let loaded = read_png(&tmp, 150.0).unwrap();
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(loaded.format, PixelFormat::Rgba8);
        assert_eq!(loaded.pixels, orig.pixels);
    }

    // ---------- PAM roundtrip ----------

    #[test]
    fn pam_gray8_roundtrip_in_memory() {
        let orig = synthetic_gray(4, 3);
        let mut buf = Vec::new();
        write_pam(&orig, &mut buf).unwrap();
        let loaded = read_pam(&buf, 100.0).unwrap();
        assert_eq!(loaded.format, PixelFormat::Gray8);
        assert_eq!(loaded.width, 4);
        assert_eq!(loaded.height, 3);
        assert_eq!(loaded.pixels, orig.pixels);
    }

    #[test]
    fn pam_rgb8_roundtrip_in_memory() {
        let orig = synthetic_rgb(3, 2);
        let mut buf = Vec::new();
        write_pam(&orig, &mut buf).unwrap();
        let loaded = read_pam(&buf, 200.0).unwrap();
        assert_eq!(loaded.format, PixelFormat::Rgb8);
        assert_eq!(loaded.pixels, orig.pixels);
    }

    #[test]
    fn pam_rgba8_roundtrip_in_memory() {
        let orig = synthetic_rgba(2, 2);
        let mut buf = Vec::new();
        write_pam(&orig, &mut buf).unwrap();
        let loaded = read_pam(&buf, 150.0).unwrap();
        assert_eq!(loaded.format, PixelFormat::Rgba8);
        assert_eq!(loaded.pixels, orig.pixels);
    }

    #[test]
    fn pam_header_missing_endhdr_fails() {
        let bad = b"P7\nWIDTH 2\nHEIGHT 2\nDEPTH 1\nMAXVAL 255\n";
        let err = read_pam(bad, 100.0).unwrap_err();
        assert!(matches!(err, BitmapIoError::InvalidPam(_)));
    }

    #[test]
    fn pam_unsupported_maxval_fails() {
        let bad =
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 1\nMAXVAL 65535\nTUPLTYPE GRAYSCALE\nENDHDR\n\x00\x00";
        let err = read_pam(bad, 100.0).unwrap_err();
        if let BitmapIoError::InvalidPam(msg) = err {
            assert!(msg.contains("MAXVAL"));
        } else {
            panic!("expected InvalidPam, got {err:?}");
        }
    }

    #[test]
    fn pam_unsupported_depth_fails() {
        let bad = b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 5\nMAXVAL 255\nENDHDR\n\x00\x00\x00\x00\x00";
        let err = read_pam(bad, 100.0).unwrap_err();
        if let BitmapIoError::InvalidPam(msg) = err {
            assert!(msg.contains("DEPTH"));
        } else {
            panic!("expected InvalidPam, got {err:?}");
        }
    }

    #[test]
    fn pam_body_too_short_fails() {
        // 声明 2x2 Gray8 应该有 4 字节，但只给 2 字节
        let bad = b"P7\nWIDTH 2\nHEIGHT 2\nDEPTH 1\nMAXVAL 255\nENDHDR\n\x10\x20";
        let err = read_pam(bad, 100.0).unwrap_err();
        assert!(matches!(err, BitmapIoError::PamSizeMismatch { .. }));
    }

    // ---------- PPM ----------

    #[test]
    fn ppm_rgb8_basic() {
        let orig = synthetic_rgb(3, 2);
        let mut buf = Vec::new();
        write_ppm(&orig, &mut buf).unwrap();
        // 解析 PPM 头
        let header_end = find_subslice(&buf, b"\n255\n").unwrap() + b"\n255\n".len();
        let body = &buf[header_end..];
        assert_eq!(body, orig.pixels.as_slice());
    }

    #[test]
    fn ppm_gray8_expanded_to_rgb() {
        let orig = synthetic_gray(2, 1);
        let mut buf = Vec::new();
        write_ppm(&orig, &mut buf).unwrap();
        let header_end = find_subslice(&buf, b"\n255\n").unwrap() + b"\n255\n".len();
        let body = &buf[header_end..];
        // 2 像素 -> 6 字节
        assert_eq!(body.len(), 6);
        // 每个灰度 g 应该映射为 (g, g, g)
        assert_eq!(body[0], body[1]);
        assert_eq!(body[1], body[2]);
    }

    #[test]
    fn ppm_rgba_drops_alpha() {
        let orig = synthetic_rgba(2, 1);
        let mut buf = Vec::new();
        write_ppm(&orig, &mut buf).unwrap();
        let header_end = find_subslice(&buf, b"\n255\n").unwrap() + b"\n255\n".len();
        let body = &buf[header_end..];
        // 2 像素 RGB -> 6 字节
        assert_eq!(body.len(), 6);
        // 第一个像素 RGB 应等于 RGBA 中前 3 字节
        assert_eq!(&body[..3], &orig.pixels[..3]);
    }

    #[test]
    fn pam_file_roundtrip_disk() {
        let tmp = std::env::temp_dir().join("k2core_pam_file.pam");
        let orig = synthetic_rgb(5, 3);
        write_pam_file(&orig, &tmp).unwrap();
        let loaded = read_pam_file(&tmp, 200.0).unwrap();
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(loaded.pixels, orig.pixels);
    }
}
