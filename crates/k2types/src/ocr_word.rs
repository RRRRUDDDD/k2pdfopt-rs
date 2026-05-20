//! `OcrWord` —— 单个 OCR 识别词，PDF 不可见文字层的最小写入单位。
//!
//! 设计来源：
//! - `rust-rewrite-plan.md` v2.1 §9.4（`PdfWriter::add_ocr_layer(&[OcrWord])`）
//! - `rust-rewrite-plan.md` v2.1 §9.5（OcrEngine 输出 `Vec<OcrWord>`）
//! - `rust-rewrite-execution-plan.md` Step 7.2 / Step 9.2（本步骤）
//! - C 对照：`willuslib/ocrwords.c` `OCRWORD` 结构
//!
//! Step 5.6 在 `k2layout::ocr_staging` 已落地同名 struct；Step 7.2 把权威定义迁
//! 到 k2types，`k2layout` 改为 re-export；Step 9.2 修正坐标系语义：`y` 是 word
//! **顶部** y（image top-left 原点，y 向下增），与 [`crate::OcrWord`] 实际生产
//! 路径（Tesseract TSV `top` 字段）一致；旧版误把 `y` 注释为"左下角"导致
//! [`OcrWord::y_top`] 命名错位（实际返 `y + h`，是 word 矩形**底部** y）。
//!
//! Step 9.2 修复：
//! - 文档更正：`y` 是顶部（image top 原点 y 向下增长）
//! - 旧 [`y_top`](OcrWord::y_top) 误名废弃（返 `y + h` 实为 word 底部 y），新增
//!   [`y_bottom`](OcrWord::y_bottom) 替代（语义清晰：word 矩形底部 y）；旧函数
//!   仍保留但 `#[deprecated]` 标注，行为不变，避免破坏既有 import
//! - 详见 Open Question 9.1.C / 7.2.D
//!
//! # C 对照表
//!
//! | C `OCRWORD` 字段 | Rust [`OcrWord`] 字段 | 说明 |
//! |------------------|----------------------|------|
//! | `c` (column)     | `x`                  | word 左边 x（pixel）|
//! | `r` (row, 字符底部行号) | `y_bottom() = y + h` | C 的 `r` 是底部；Rust `y` 是顶部，转换 `y = r - h + 1`（参见 [`crate::OcrWord`] 与 [`OcrWord::y_bottom`]）|
//! | `w` (width)      | `w`                  | word 宽（pixel）|
//! | `h` (height)     | `h`                  | word 高（pixel）|
//! | `text`           | `text`               | UTF-8 文本 |
//! | (无)             | `confidence`         | 0-1 归一化，Tesseract conf/100 |

/// 单个 OCR 识别词。
///
/// # 坐标系约定（Step 9.2 修正）
///
/// `(x, y)` 是 word 矩形的**左上角**像素坐标，**image top-left 原点**，y 向下
/// 增长。换算到 PDF baseline 坐标系（左下原点 y 向上）应使用 [`Self::y_bottom`]：
///
/// ```text
/// PDF baseline_y_pt = page_height_pt - y_bottom() * 72 / dpi
///                   = page_height_pt - (y + h) * 72 / dpi
/// ```
///
/// 与 C 版的 `OCRWORD.c, OCRWORD.r`（C `r` 是底部行号）相比，Rust `y = r - h + 1`
/// 是 word 顶部行号；这是因为 Tesseract TSV 输出的 `top` 字段就是矩形顶部 y。
#[derive(Debug, Clone)]
pub struct OcrWord {
    /// 识别出的文本内容（UTF-8）。对应 C `OCRWORD.text`。
    pub text: String,
    /// word 矩形**左上角** x 坐标（pixel；image top-left 原点）。对应 C `OCRWORD.c`。
    pub x: f64,
    /// word 矩形**顶部** y 坐标（pixel；image top-left 原点，y 向下增长）。
    /// **不**等于 C `OCRWORD.r`（C `r` 是底部行号）；转换关系：`y = r - h + 1`。
    pub y: f64,
    /// word 宽度（pixel）。对应 C `OCRWORD.w`。
    pub w: f64,
    /// word 高度（pixel）。对应 C `OCRWORD.h`。
    pub h: f64,
    /// OCR 置信度（0.0 ~ 1.0）。C 版无对应字段，由 Tesseract conf 列除以 100 得到。
    pub confidence: f32,
}

impl OcrWord {
    /// 简化构造：仅 text + 矩形坐标；confidence 默认 1.0。
    #[must_use]
    pub fn new<S: Into<String>>(text: S, x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            text: text.into(),
            x,
            y,
            w,
            h,
            confidence: 1.0,
        }
    }

    /// word 矩形右边 x（`x + w`）。
    #[must_use]
    pub fn x_right(&self) -> f64 {
        self.x + self.w
    }

    /// word 矩形**底部** y（image top-left 原点：`y + h`）。
    ///
    /// 用于 PDF baseline 换算：`baseline_y_pt = page_h_pt - y_bottom() * 72 / dpi`。
    /// 等价于 C 版 `OCRWORD.r`（C `r` 字段语义）。
    #[must_use]
    pub fn y_bottom(&self) -> f64 {
        self.y + self.h
    }

    /// **已废弃**：误命名（实际返回 `y + h` 即 word **底部** y，不是顶部）；
    /// 旧实现保留不变只为兼容 Step 7.2 已写代码，请使用 [`Self::y_bottom`]。
    ///
    /// Step 9.2 通过：Open Question 9.1.C 后续 release 阶段 (Step 10.x) 删除。
    #[deprecated(since = "0.0.1", note = "误命名；请用 `y_bottom()`（数值不变）")]
    #[must_use]
    pub fn y_top(&self) -> f64 {
        self.y + self.h
    }

    /// word 矩形**中线** y（`y + h/2`）。
    ///
    /// 用于 [`drain_in_range`](../../../k2layout/master/ocr_staging/struct.OcrStaging.html#method.drain_in_range)
    /// 判定 word 是否被"切到上半页"：当 `y_center < rowcount` 时归属上一页。
    /// 与 C 版 `r - maxheight + h/2`（约等于中线）的等价 Rust 实现（不依赖
    /// `maxheight` 字段，Rust 端 OcrWord 未保留该字段；详见 Open Question 9.2.A）。
    ///
    /// 对应 C 源：`k2pdfoptlib/k2master.c:1535-1544`（masterinfo_publish OCR 选页）。
    #[must_use]
    pub fn y_center(&self) -> f64 {
        self.y + self.h * 0.5
    }
}

// 手动 PartialEq 因 f64 / f32 不实现 Eq（NaN 比较语义保守，逐字段 to_bits 比对）。
// 与 k2layout::ocr_staging 现版本一致。
impl PartialEq for OcrWord {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.x.to_bits() == other.x.to_bits()
            && self.y.to_bits() == other.y.to_bits()
            && self.w.to_bits() == other.w.to_bits()
            && self.h.to_bits() == other.h.to_bits()
            && self.confidence.to_bits() == other.confidence.to_bits()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, deprecated)]

    use super::*;

    #[test]
    fn new_sets_default_confidence_1() {
        let w = OcrWord::new("hello", 10.0, 20.0, 50.0, 12.0);
        assert_eq!(w.text, "hello");
        assert!((w.confidence - 1.0).abs() < 1e-6);
    }

    #[test]
    fn x_right_y_bottom_math() {
        let w = OcrWord::new("abc", 100.0, 200.0, 30.0, 12.0);
        assert!((w.x_right() - 130.0).abs() < 1e-6);
        assert!((w.y_bottom() - 212.0).abs() < 1e-6);
    }

    #[test]
    fn y_center_is_midpoint() {
        let w = OcrWord::new("abc", 0.0, 100.0, 10.0, 20.0);
        // y=100, h=20 → mid = 100 + 10 = 110
        assert!((w.y_center() - 110.0).abs() < 1e-6);
    }

    #[test]
    fn y_top_deprecated_still_returns_y_plus_h() {
        // Step 9.2 兼容：旧函数保留不变，等价于 y_bottom
        let w = OcrWord::new("abc", 0.0, 5.0, 30.0, 7.0);
        assert!((w.y_top() - w.y_bottom()).abs() < 1e-12);
        assert!((w.y_top() - 12.0).abs() < 1e-6);
    }

    #[test]
    fn partial_eq_via_bits() {
        let w1 = OcrWord {
            text: "x".to_string(),
            x: 1.5,
            y: 2.5,
            w: 3.5,
            h: 4.5,
            confidence: 0.9,
        };
        let w2 = w1.clone();
        assert_eq!(w1, w2);
    }

    #[test]
    fn partial_eq_text_differs() {
        let w1 = OcrWord::new("a", 0.0, 0.0, 1.0, 1.0);
        let w2 = OcrWord::new("b", 0.0, 0.0, 1.0, 1.0);
        assert_ne!(w1, w2);
    }

    #[test]
    fn partial_eq_x_differs() {
        let w1 = OcrWord::new("a", 0.0, 0.0, 1.0, 1.0);
        let w2 = OcrWord::new("a", 0.000001, 0.0, 1.0, 1.0);
        assert_ne!(w1, w2);
    }

    #[test]
    fn utf8_text_works() {
        let w = OcrWord::new("中文", 0.0, 0.0, 24.0, 16.0);
        assert_eq!(w.text, "中文");
        assert_eq!(w.text.chars().count(), 2);
    }
}
