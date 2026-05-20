//! `k2ocr::tsv_parser` —— Tesseract `tsv` 输出解析。
//!
//! TSV 格式（tesseract 4.0+）：
//! ```text
//! level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext
//! 1\t1\t0\t0\t0\t0\t0\t0\t1234\t567\t-1\t
//! 2\t1\t1\t0\t0\t0\t45\t67\t100\t30\t-1\t
//! 3\t1\t1\t1\t0\t0\t45\t67\t100\t30\t-1\t
//! 4\t1\t1\t1\t1\t0\t45\t67\t100\t30\t-1\t
//! 5\t1\t1\t1\t1\t1\t45\t67\t50\t30\t95.0\thello
//! 5\t1\t1\t1\t1\t2\t100\t67\t50\t30\t93.5\tworld
//! ```
//!
//! - `level=5` 才是 word 行（4=line / 3=paragraph / 2=block / 1=page 跳过）。
//! - `conf` 取值 `-1` 表示占位（非 word 行），否则 `0..=100` 浮点。
//! - `text` 列可含空格、可空（被丢弃）。
//!
//! 输出坐标系：返回的 `TsvWord.x/y` 是**子图局部坐标**（左上原点，y 向下）。
//! ROI 偏移由 `TesseractCliEngine::recognize` 在拼装最终 `OcrWord` 时加回。
//!
//! 设计要点：
//! - **fail-soft**：单行畸形不返 Err，整体丢弃（与 spike `parse_tsv` 同源 + ADR-017 容错诉求）。
//! - **header 自适应**：跳过第一行（无论是否真是 header），再按 level/field 计数严格筛选。
//!   适配 `tesseract 5.5.0` 与 `4.x` 两种 header 排版。
//! - **CRLF 兼容**：行尾用 [`str::trim_end_matches`] 显式剥离 `\r`，Windows tesseract 写 CRLF。

use crate::types::OcrError;

/// TSV 单 word 解析结果（**局部坐标**，未加 ROI offset）。
#[derive(Debug, Clone, PartialEq)]
pub struct TsvWord {
    pub text: String,
    /// 置信度 `0.0..=100.0`（与 Tesseract 同源原始尺度）。
    pub confidence: f32,
    /// word 左上角 x（pixel, 局部）。
    pub left: i32,
    /// word 左上角 y（pixel, 局部）。
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl TsvWord {
    /// word 矩形右边 x。
    #[must_use]
    pub fn right(&self) -> i32 {
        self.left + self.width
    }

    /// word 矩形底边 y。
    #[must_use]
    pub fn bottom(&self) -> i32 {
        self.top + self.height
    }
}

/// 解析整段 TSV。
///
/// `min_confidence` 范围 `0.0..=100.0`（与 TSV 原始 conf 同尺度，不归一化）；
/// 设为 `0.0` 时不过滤（含 conf=0 word）。负数 conf（line/block placeholder）始终被丢弃。
///
/// 返回 `Err(OcrError::OutputParse)` 仅当 TSV 完全无法解读（如空字符串 + 无 header）；
/// 单行畸形（字段不足、非数字）一律丢弃，与 spike 同源 fail-soft。
pub fn parse_tsv(tsv: &str, min_confidence: f32) -> Result<Vec<TsvWord>, OcrError> {
    let mut words = Vec::new();
    let mut saw_any_line = false;
    let mut header_skipped = false;

    for raw in tsv.lines() {
        saw_any_line = true;
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if !header_skipped {
            // 第一行非空就跳过（无论是否真是 header）。
            header_skipped = true;
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 12 {
            continue;
        }
        // level 解析失败 → 整行丢弃。
        let level: i32 = match fields[0].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if level != 5 {
            continue;
        }
        let conf: f32 = match fields[10].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if conf < 0.0 {
            continue;
        }
        if conf < min_confidence {
            continue;
        }
        // text 列允许内含空格但首尾空白裁掉；纯空白丢弃。
        let text = fields[11].trim().to_string();
        if text.is_empty() {
            continue;
        }
        // bbox 4 字段（left/top/width/height）失败 → 整行丢弃。
        let parse_i32 = |idx: usize| -> Option<i32> { fields[idx].parse().ok() };
        let Some(left) = parse_i32(6) else { continue };
        let Some(top) = parse_i32(7) else { continue };
        let Some(width) = parse_i32(8) else { continue };
        let Some(height) = parse_i32(9) else { continue };
        if width <= 0 || height <= 0 {
            continue;
        }
        words.push(TsvWord {
            text,
            confidence: conf,
            left,
            top,
            width,
            height,
        });
    }

    if !saw_any_line {
        return Err(OcrError::OutputParse(
            "Tesseract TSV 完全为空（含无 header）".to_string(),
        ));
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const SAMPLE_HEADER: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext";

    fn make_line(
        level: i32,
        left: i32,
        top: i32,
        w: i32,
        h: i32,
        conf: &str,
        text: &str,
    ) -> String {
        format!("{level}\t1\t1\t1\t1\t1\t{left}\t{top}\t{w}\t{h}\t{conf}\t{text}")
    }

    #[test]
    fn empty_input_returns_err() {
        let r = parse_tsv("", 0.0);
        assert!(matches!(r, Err(OcrError::OutputParse(_))));
    }

    #[test]
    fn header_only_returns_empty_ok() {
        let r = parse_tsv(SAMPLE_HEADER, 0.0).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn parses_single_word() {
        let tsv = format!(
            "{}\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "95.5", "hello")
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "hello");
        assert!((r[0].confidence - 95.5).abs() < 1e-6);
        assert_eq!(r[0].left, 10);
        assert_eq!(r[0].top, 20);
        assert_eq!(r[0].width, 50);
        assert_eq!(r[0].height, 30);
        assert_eq!(r[0].right(), 60);
        assert_eq!(r[0].bottom(), 50);
    }

    #[test]
    fn parses_two_words() {
        let tsv = format!(
            "{}\n{}\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "95.5", "hello"),
            make_line(5, 60, 20, 50, 30, "92.0", "world")
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].text, "hello");
        assert_eq!(r[1].text, "world");
        assert!((r[1].confidence - 92.0).abs() < 1e-6);
    }

    #[test]
    fn skips_non_word_levels() {
        let tsv = format!(
            "{}\n{}\n{}\n{}\n{}\n",
            SAMPLE_HEADER,
            make_line(1, 0, 0, 1000, 500, "-1", ""),
            make_line(2, 10, 20, 100, 200, "-1", ""),
            make_line(3, 10, 20, 100, 200, "-1", ""),
            make_line(4, 10, 20, 100, 200, "-1", ""),
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn skips_negative_confidence() {
        let tsv = format!(
            "{}\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "-1", "garbage")
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn skips_empty_text() {
        let tsv = format!(
            "{}\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "95.0", "")
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn skips_whitespace_only_text() {
        let tsv = format!(
            "{}\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "95.0", "   ")
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn skips_zero_or_negative_dimensions() {
        let tsv = format!(
            "{}\n{}\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 0, 30, "95.0", "zero_w"),
            make_line(5, 10, 20, 50, 0, "95.0", "zero_h"),
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn filters_by_min_confidence() {
        let tsv = format!(
            "{}\n{}\n{}\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "30.0", "low"),
            make_line(5, 70, 20, 50, 30, "60.0", "mid"),
            make_line(5, 130, 20, 50, 30, "90.0", "high")
        );
        let r = parse_tsv(&tsv, 50.0).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].text, "mid");
        assert_eq!(r[1].text, "high");
    }

    #[test]
    fn min_confidence_zero_keeps_zero_conf() {
        let tsv = format!(
            "{}\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "0.0", "zero_conf")
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "zero_conf");
    }

    #[test]
    fn handles_crlf_line_endings() {
        let tsv = format!(
            "{}\r\n{}\r\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "95.0", "windows")
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "windows");
    }

    #[test]
    fn handles_utf8_text() {
        let tsv = format!(
            "{}\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "95.0", "中文")
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "中文");
    }

    #[test]
    fn ignores_short_lines() {
        let tsv = format!(
            "{}\n{}\nshort\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "95.0", "first"),
            make_line(5, 70, 20, 50, 30, "94.0", "second"),
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn ignores_nonnumeric_fields() {
        let tsv = format!(
            "{}\nbad\tx\tx\tx\tx\tx\tx\tx\tx\tx\tx\ttext_here\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 70, 20, 50, 30, "94.0", "second"),
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "second");
    }

    #[test]
    fn confidence_integer_format() {
        // tesseract 5.x 偶尔输出整数 conf "95" 而非 "95.0"
        let tsv = format!(
            "{}\n5\t1\t1\t1\t1\t1\t10\t20\t50\t30\t95\tint_conf\n",
            SAMPLE_HEADER
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert_eq!(r.len(), 1);
        assert!((r[0].confidence - 95.0).abs() < 1e-6);
    }

    #[test]
    fn text_with_internal_spaces_preserved() {
        // word level 通常单 token，但本字段实际为 \t-separated 最后列，单 word 内不会含 tab
        let tsv = format!(
            "{}\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "95.0", "a-b")
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert_eq!(r[0].text, "a-b");
    }

    #[test]
    fn blank_lines_in_middle_skipped() {
        let tsv = format!(
            "{}\n{}\n\n\n{}\n",
            SAMPLE_HEADER,
            make_line(5, 10, 20, 50, 30, "95.0", "a"),
            make_line(5, 70, 20, 50, 30, "94.0", "b")
        );
        let r = parse_tsv(&tsv, 0.0).unwrap();
        assert_eq!(r.len(), 2);
    }
}
