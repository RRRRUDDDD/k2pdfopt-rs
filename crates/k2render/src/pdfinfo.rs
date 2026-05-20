//! PDF 元信息提取 - 通过 `mutool info` 文本输出解析得 [`PdfInfo`]。
//!
//! 注：mutool 1.27.0 不支持 `-F json` 输出（与 v2.1 §10 M2 原描述不一致——
//! `-F` 在 1.27.0 实际是 "list fonts"）。本模块改走纯文本解析路线，覆盖：
//! - `PDF-X.Y` 行 → [`PdfInfo::pdf_version`]
//! - `Pages: N` 行 → [`PdfInfo::page_count`]
//! - `Encryption object` 节出现 → [`PdfInfo::encrypted`] = true
//! - `Info object (X 0 R):` 后的 `<<...>>` 字典 → 文档元数据字段
//! - `Mediaboxes (N):` 节 → [`PdfInfo::mediaboxes_pt`]（按"唯一" mediabox 列出）
//!
//! 加密 PDF 走 [`crate::renderer::RenderError::Encrypted`] 错误路径（mutool 退出码非 0 +
//! stderr 含 `password`/`authenticate`/`encrypt` 关键字）。
//!
//! 详见 `rust-rewrite-execution-plan.md` Step 4.2 与 ADR-015。

use crate::mutool::{check_mutool_exit, parse_mediabox_line_pub, run_mutool_info};
use crate::renderer::RenderError;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// PDF 元信息结构。除 [`PdfInfo::page_count`] 外的 metadata 字段均 [`Option`]，
/// 源 PDF 未提供时为 [`None`]。
#[derive(Debug, Clone, PartialEq)]
pub struct PdfInfo {
    /// PDF 规范版本，如 `"PDF-1.4"`（mutool info 顶部首个 `PDF-` 开头行）。
    pub pdf_version: Option<String>,
    /// 页数（来自 `Pages: N` 行）。
    pub page_count: usize,
    /// `(width_pt, height_pt)` 的 mediabox 列表。
    ///
    /// 注意：mutool info 按 "唯一" mediabox 输出——所有页共享一个 mediabox 时只列一项。
    /// 本字段长度可能 ≤ [`PdfInfo::page_count`]。要拿"每页一个 mediabox"请用
    /// [`crate::MutoolRenderer::page_size`]。
    pub mediaboxes_pt: Vec<(f32, f32)>,
    /// 文档是否带 Encryption 字典（来自 `Encryption object` 节）。
    pub encrypted: bool,
    /// 文档标题（Info dict `/Title`）。
    pub title: Option<String>,
    /// 作者（Info dict `/Author`）。
    pub author: Option<String>,
    /// 主题（Info dict `/Subject`）。
    pub subject: Option<String>,
    /// 关键字（Info dict `/Keywords`）。
    pub keywords: Option<String>,
    /// 创建者程序（Info dict `/Creator`）。
    pub creator: Option<String>,
    /// 生成器程序（Info dict `/Producer`）。
    pub producer: Option<String>,
    /// 创建时间，原始 PDF 时间字符串如 `"D:20260512140128+08'00"`。
    pub creation_date: Option<String>,
    /// 修改时间。
    pub mod_date: Option<String>,
}

/// 构造 [`PdfInfo::from_path_with_options`] 的可选参数。
#[derive(Debug, Clone)]
pub struct PdfInfoOptions {
    /// PDF 解密密码（mutool `-p` 参数）。
    pub password: Option<String>,
    /// 自定义 mutool 二进制路径；默认使用 PATH 中的 `mutool`。
    pub binary: PathBuf,
}

impl Default for PdfInfoOptions {
    fn default() -> Self {
        Self {
            password: None,
            binary: PathBuf::from("mutool"),
        }
    }
}

impl PdfInfo {
    /// 默认构造：使用 PATH 中的 `mutool`，不传密码。
    pub fn from_path<P: AsRef<Path>>(pdf_path: P) -> Result<Self> {
        Self::from_path_with_options(pdf_path, PdfInfoOptions::default())
    }

    /// 完整构造：允许传密码 + 自定义 mutool 路径。
    ///
    /// 加密 PDF 不传密码时返回 [`RenderError::Encrypted`]；二进制缺失返回
    /// [`RenderError::BinaryNotFound`]；其他 mutool 错误返回 [`RenderError::SubprocessFailed`]。
    pub fn from_path_with_options<P: AsRef<Path>>(
        pdf_path: P,
        opts: PdfInfoOptions,
    ) -> Result<Self> {
        let pdf_path = pdf_path.as_ref();
        if !pdf_path.exists() {
            anyhow::bail!("PDF file not found: {}", pdf_path.display());
        }
        check_binary(&opts.binary)?;
        let out = run_mutool_info(&opts.binary, pdf_path, opts.password.as_deref())?;
        // run_mutool_info 已经在内部调过 check_mutool_exit；这里再校验一次保险
        check_mutool_exit(&out, pdf_path)?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        parse_mutool_info_output(&stdout)
            .with_context(|| format!("parse mutool info output for {}", pdf_path.display()))
    }
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

/// 解析 mutool info 的完整 stdout 文本输出，提取结构化字段。
///
/// `pub(crate)` 暴露以便单元测试用静态样本验证。
pub(crate) fn parse_mutool_info_output(stdout: &str) -> Result<PdfInfo> {
    let mut info = PdfInfo {
        pdf_version: None,
        page_count: 0,
        mediaboxes_pt: Vec::new(),
        encrypted: false,
        title: None,
        author: None,
        subject: None,
        keywords: None,
        creator: None,
        producer: None,
        creation_date: None,
        mod_date: None,
    };
    let mut page_count_seen = false;
    // Info object 节状态：Some(buf) 表示正在累积 `<<...>>` 字典内容
    let mut info_buffer: Option<String> = None;
    let mut in_mediaboxes = false;

    for raw_line in stdout.lines() {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();

        // PDF version 检测：以 `PDF-` 开头的单独行，且形如 `PDF-X.Y`
        if info.pdf_version.is_none() && is_pdf_version_line(trimmed) {
            info.pdf_version = Some(trimmed.to_string());
            continue;
        }
        // Pages: N
        if let Some(rest) = trimmed.strip_prefix("Pages:") {
            info.page_count = rest
                .trim()
                .parse::<usize>()
                .with_context(|| format!("parse Pages: value from `{trimmed}`"))?;
            page_count_seen = true;
            continue;
        }
        // Encryption object 节标记加密
        if trimmed.starts_with("Encryption object") {
            info.encrypted = true;
            continue;
        }
        // Info object 节：下一行起累积到遇到 `>>` 闭合或空行
        if trimmed.starts_with("Info object") {
            info_buffer = Some(String::new());
            continue;
        }
        if let Some(buf) = info_buffer.as_mut() {
            if trimmed.is_empty() {
                parse_info_dict(buf, &mut info);
                info_buffer = None;
                continue;
            }
            if !buf.is_empty() {
                buf.push(' ');
            }
            buf.push_str(trimmed);
            if trimmed.ends_with(">>") {
                parse_info_dict(buf, &mut info);
                info_buffer = None;
            }
            continue;
        }
        // Mediaboxes 节
        if trimmed.starts_with("Mediaboxes") {
            in_mediaboxes = true;
            continue;
        }
        if in_mediaboxes {
            if line.is_empty() {
                continue;
            }
            // mediabox 行以制表符 / 空格开头
            if !line.starts_with(char::is_whitespace) {
                in_mediaboxes = false;
            } else if let Some((_idx, w, h)) = parse_mediabox_line_pub(line) {
                info.mediaboxes_pt.push((w, h));
            }
        }
    }

    if !page_count_seen {
        return Err(
            RenderError::InvalidSource("no `Pages:` line in mutool info output".into()).into(),
        );
    }
    Ok(info)
}

/// 检测 `PDF-X.Y` 形式的版本行（排除如 `PDF-related` 这类误报）。
fn is_pdf_version_line(line: &str) -> bool {
    let Some(tail) = line.strip_prefix("PDF-") else {
        return false;
    };
    // 至少含一个数字 + 点 + 数字
    let mut bytes = tail.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_digit() {
        return false;
    }
    tail.contains('.') && tail.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// 解析 `<<...>>` 形式的 PDF Info 字典文本，提取已知字段到 [`PdfInfo`]。
///
/// 支持紧凑形式 `<</Title(...)/Producer(...)>>` 与多行形式 `<<\n /Title (...)\n>>`。
fn parse_info_dict(buf: &str, info: &mut PdfInfo) {
    let bytes = buf.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' {
            // 解析 key：紧跟 `/` 后的字母数字
            let key_start = i + 1;
            let mut key_end = key_start;
            while key_end < bytes.len() {
                let b = bytes[key_end];
                if b == b'(' || b == b'<' || b == b'/' || b == b'>' || b.is_ascii_whitespace() {
                    break;
                }
                key_end += 1;
            }
            if key_end == key_start {
                i += 1;
                continue;
            }
            let key = &buf[key_start..key_end];
            // 跳过空白
            let mut j = key_end;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= bytes.len() {
                break;
            }
            match bytes[j] {
                b'(' => {
                    if let Some((value, after)) = parse_paren_literal(buf, j) {
                        assign_info_field(info, key, &value);
                        i = after;
                        continue;
                    }
                    // 无法闭合：跳过这个 key
                    i = j + 1;
                }
                b'<' if bytes.get(j + 1) == Some(&b'<') => {
                    // 嵌套字典——不支持，跳过这个 key
                    i = j + 2;
                }
                b'<' => {
                    // hex string `<...>`：跳过
                    if let Some(end_off) = buf[j..].find('>') {
                        i = j + end_off + 1;
                    } else {
                        return;
                    }
                }
                _ => {
                    // 其他值类型（name / number / bool）——跳到下一空白
                    let mut k = j;
                    while k < bytes.len() && !bytes[k].is_ascii_whitespace() && bytes[k] != b'/' {
                        k += 1;
                    }
                    i = k;
                }
            }
        } else {
            i += 1;
        }
    }
}

/// 解析 PDF literal string `( ... )`，处理转义与嵌套括号。
///
/// 返回 `(unescaped_value, position_after_closing_paren)`；输入 `start` 必须指向 `(`。
/// 未闭合返回 `None`。
fn parse_paren_literal(buf: &str, start: usize) -> Option<(String, usize)> {
    let bytes = buf.as_bytes();
    if bytes.get(start) != Some(&b'(') {
        return None;
    }
    let mut depth: i32 = 0;
    let mut value = String::new();
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'(' => {
                if depth > 0 {
                    value.push('(');
                }
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((value, i + 1));
                }
                value.push(')');
                i += 1;
            }
            b'\\' => {
                if i + 1 >= bytes.len() {
                    return None;
                }
                let next = bytes[i + 1];
                let unescaped = match next {
                    b'n' => Some('\n'),
                    b'r' => Some('\r'),
                    b't' => Some('\t'),
                    b'b' => Some('\u{0008}'),
                    b'f' => Some('\u{000C}'),
                    b'(' => Some('('),
                    b')' => Some(')'),
                    b'\\' => Some('\\'),
                    _ => None,
                };
                if let Some(c) = unescaped {
                    value.push(c);
                    i += 2;
                } else {
                    // 未识别转义：保留下一字符（PDF spec: `\<other>` ≡ `<other>`）
                    value.push(next as char);
                    i += 2;
                }
            }
            _ => {
                value.push(b as char);
                i += 1;
            }
        }
    }
    None
}

fn assign_info_field(info: &mut PdfInfo, key: &str, value: &str) {
    match key {
        "Title" => info.title = Some(value.to_string()),
        "Author" => info.author = Some(value.to_string()),
        "Subject" => info.subject = Some(value.to_string()),
        "Keywords" => info.keywords = Some(value.to_string()),
        "Creator" => info.creator = Some(value.to_string()),
        "Producer" => info.producer = Some(value.to_string()),
        "CreationDate" => info.creation_date = Some(value.to_string()),
        "ModDate" => info.mod_date = Some(value.to_string()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn sample_info_output_with_metadata() -> &'static str {
        "tests/golden/single-column/c-output.pdf:\n\
         \n\
         PDF-1.3\n\
         Info object (13 0 R):\n\
         <</Title(c-output.pdf)/CreationDate(D:20260512140126+08'00)/ModDate(D:20260512140126+08'00)/Producer(K2pdfopt v2.55)>>\n\
         Pages: 2\n\
         \n\
         Not a ZUGFeRD file.\n\
         Retrieving info from pages 1-2...\n\
         Mediaboxes (1):\n\
         \t1\t(2 0 R):\t[ 0 0 244.6 328.3 ]\n\
         \n\
         Fonts (3):\n\
         \t1\t(2 0 R):\tType1 'Helvetica' WinAnsiEncoding (0 0 R)\n"
    }

    #[test]
    fn parse_basic_fields_from_metadata_sample() {
        let info = parse_mutool_info_output(sample_info_output_with_metadata()).unwrap();
        assert_eq!(info.pdf_version.as_deref(), Some("PDF-1.3"));
        assert_eq!(info.page_count, 2);
        assert!(!info.encrypted);
        assert_eq!(info.title.as_deref(), Some("c-output.pdf"));
        assert_eq!(info.producer.as_deref(), Some("K2pdfopt v2.55"));
        assert_eq!(
            info.creation_date.as_deref(),
            Some("D:20260512140126+08'00")
        );
        assert_eq!(info.mod_date.as_deref(), Some("D:20260512140126+08'00"));
        assert!(info.author.is_none());
        assert!(info.subject.is_none());
        assert_eq!(info.mediaboxes_pt.len(), 1);
        let (w, h) = info.mediaboxes_pt[0];
        assert!((w - 244.6).abs() < 0.2);
        assert!((h - 328.3).abs() < 0.2);
    }

    #[test]
    fn parse_detects_encryption_section() {
        let encrypted_sample = "tests/fixtures/encrypted.pdf:\n\
            \n\
            PDF-1.4\n\
            \n\
            Encryption object (0 0 R):\n\
            <<...>>\n\
            Pages: 1\n";
        let info = parse_mutool_info_output(encrypted_sample).unwrap();
        assert!(info.encrypted);
        assert_eq!(info.page_count, 1);
        assert_eq!(info.pdf_version.as_deref(), Some("PDF-1.4"));
    }

    #[test]
    fn parse_missing_pages_returns_invalid_source() {
        let bad = "PDF-1.4\nMediaboxes (1):\n\t1\t(2 0 R):\t[ 0 0 10 10 ]\n";
        let err = parse_mutool_info_output(bad).unwrap_err();
        let typed = err
            .downcast_ref::<RenderError>()
            .expect("err should be typed RenderError");
        assert!(matches!(typed, RenderError::InvalidSource(_)));
    }

    #[test]
    fn parse_no_info_dict_yields_none_metadata() {
        let no_info = "tests/fixtures/single-column.pdf:\n\
            \n\
            PDF-1.4\n\
            \n\
            Pages: 1\n\
            \n\
            Not a ZUGFeRD file.\n\
            Mediaboxes (1):\n\t1\t(5 0 R):\t[ 0 0 595 842 ]\n";
        let info = parse_mutool_info_output(no_info).unwrap();
        assert_eq!(info.page_count, 1);
        assert!(info.title.is_none());
        assert!(info.producer.is_none());
        assert!(info.author.is_none());
        assert!(info.creator.is_none());
        assert_eq!(info.mediaboxes_pt.len(), 1);
    }

    #[test]
    fn parse_paren_literal_handles_escapes_and_nesting() {
        let s = "(hello\\nworld)";
        let (v, end) = parse_paren_literal(s, 0).unwrap();
        assert_eq!(v, "hello\nworld");
        assert_eq!(end, s.len());

        let s2 = "(a\\(b\\))";
        let (v2, _) = parse_paren_literal(s2, 0).unwrap();
        assert_eq!(v2, "a(b)");

        let s3 = "(outer (inner) tail)";
        let (v3, _) = parse_paren_literal(s3, 0).unwrap();
        assert_eq!(v3, "outer (inner) tail");

        // 反斜杠 + 未识别字符：透传字符
        let s4 = r"(\x)";
        let (v4, _) = parse_paren_literal(s4, 0).unwrap();
        assert_eq!(v4, "x");
    }

    #[test]
    fn parse_paren_literal_rejects_unclosed() {
        assert!(parse_paren_literal("(no close", 0).is_none());
        assert!(parse_paren_literal("(escape only \\", 0).is_none());
    }

    #[test]
    fn parse_paren_literal_requires_open_paren_at_start() {
        assert!(parse_paren_literal("not a paren", 0).is_none());
    }

    #[test]
    fn parse_info_dict_extracts_all_known_fields() {
        let mut info = empty_info();
        let dict = "<</Title (Test Title) /Author (Test Author) /Subject (Subj) /Keywords (a, b) /Creator (Cr) /Producer (Pr) /CreationDate (D:20250101) /ModDate (D:20250102)>>";
        parse_info_dict(dict, &mut info);
        assert_eq!(info.title.as_deref(), Some("Test Title"));
        assert_eq!(info.author.as_deref(), Some("Test Author"));
        assert_eq!(info.subject.as_deref(), Some("Subj"));
        assert_eq!(info.keywords.as_deref(), Some("a, b"));
        assert_eq!(info.creator.as_deref(), Some("Cr"));
        assert_eq!(info.producer.as_deref(), Some("Pr"));
        assert_eq!(info.creation_date.as_deref(), Some("D:20250101"));
        assert_eq!(info.mod_date.as_deref(), Some("D:20250102"));
    }

    #[test]
    fn parse_info_dict_handles_compact_format() {
        // mutool info 紧凑格式：键值紧贴无空白
        let mut info = empty_info();
        let dict =
            "<</Title(c-output.pdf)/Producer(K2pdfopt v2.55)/CreationDate(D:20260512140126+08'00)>>";
        parse_info_dict(dict, &mut info);
        assert_eq!(info.title.as_deref(), Some("c-output.pdf"));
        assert_eq!(info.producer.as_deref(), Some("K2pdfopt v2.55"));
        assert_eq!(
            info.creation_date.as_deref(),
            Some("D:20260512140126+08'00")
        );
    }

    #[test]
    fn parse_info_dict_skips_unknown_keys() {
        let mut info = empty_info();
        let dict = "<</Trapped /False /CustomField (X) /Title (T) /Pages 3>>";
        parse_info_dict(dict, &mut info);
        assert_eq!(info.title.as_deref(), Some("T"));
    }

    #[test]
    fn parse_info_dict_skips_hex_string_values() {
        let mut info = empty_info();
        let dict = "<</ID <ABCDEF1234> /Title (Real Title)>>";
        parse_info_dict(dict, &mut info);
        assert_eq!(info.title.as_deref(), Some("Real Title"));
    }

    #[test]
    fn parse_mediabox_section_with_multiple_entries() {
        let sample = "PDF-1.4\n\
            Pages: 3\n\
            Mediaboxes (3):\n\
            \t1\t(2 0 R):\t[ 0 0 595 842 ]\n\
            \t2\t(7 0 R):\t[ 0 0 612 792 ]\n\
            \t3\t(12 0 R):\t[ 0 0 100 200 ]\n\
            \n\
            Fonts (1):\n";
        let info = parse_mutool_info_output(sample).unwrap();
        assert_eq!(info.mediaboxes_pt.len(), 3);
        assert!((info.mediaboxes_pt[0].0 - 595.0).abs() < 1e-3);
        assert!((info.mediaboxes_pt[1].0 - 612.0).abs() < 1e-3);
        assert!((info.mediaboxes_pt[2].1 - 200.0).abs() < 1e-3);
    }

    #[test]
    fn is_pdf_version_line_basic_cases() {
        assert!(is_pdf_version_line("PDF-1.4"));
        assert!(is_pdf_version_line("PDF-2.0"));
        assert!(!is_pdf_version_line("PDF-related"));
        assert!(!is_pdf_version_line("PDF-"));
        assert!(!is_pdf_version_line("not PDF-1.4 inline"));
    }

    #[test]
    fn pdf_info_options_default_uses_mutool_in_path() {
        let opts = PdfInfoOptions::default();
        assert_eq!(opts.binary, PathBuf::from("mutool"));
        assert!(opts.password.is_none());
    }

    fn empty_info() -> PdfInfo {
        PdfInfo {
            pdf_version: None,
            page_count: 0,
            mediaboxes_pt: Vec::new(),
            encrypted: false,
            title: None,
            author: None,
            subject: None,
            keywords: None,
            creator: None,
            producer: None,
            creation_date: None,
            mod_date: None,
        }
    }
}
