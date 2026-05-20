//! `k2ocr::types` —— OCR 引擎对外类型。
//!
//! 设计要点：
//! - **不依赖 `k2settings::OcrSettings`**：避免反向依赖（与 Step 6.1/6.2/6.3/8.1/8.3
//!   的 `ColumnSettings/RowSettings/WordSettings/WrapPipelineSettings/FigureSettings`
//!   独立 struct 同源约定）。`k2settings::OcrSettings` 在调用端做映射。
//! - **ROI 局部坐标**：`OcrPageInput::roi` 是 `bitmap` 内部的像素 ROI；
//!   引擎返回的 `OcrWord.x/y` 加回 `roi.x0/y0` 后即原页坐标。
//! - **PSM/OEM 与 Tesseract 同源**：`PageSegmentationMode` 0~13 全 14 个、
//!   `OcrEngineMode` 0~3 全 4 个；非 tesseract 引擎按 best-effort 映射。
//!
//! 来源：
//! - `rust-rewrite-execution-plan.md` Step 9.1（M7 起步）
//! - ADR-017 OCR engine choice
//! - C 对照：`willuslib/ocrtess.c::ocrtess_ocrwords_from_bmp8` 入参（segmode + dpi + downsample）

use std::path::PathBuf;
use thiserror::Error;

use k2types::BitmapError;

/// 像素 ROI（inclusive 语义，与 k2core::Rect / C `c1,r1,c2,r2` 一致）。
///
/// 当作为 `OcrPageInput::roi` 时，给出引擎处理的目标矩形子集；
/// `None` 表示整页。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrRoi {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl OcrRoi {
    /// 构造一个 inclusive ROI；要求 `x0 <= x1 && y0 <= y1`。
    #[must_use]
    pub const fn new(x0: u32, y0: u32, x1: u32, y1: u32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// ROI 宽度（pixels），inclusive 语义所以 `x1 - x0 + 1`。
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.x1.saturating_sub(self.x0).saturating_add(1)
    }

    /// ROI 高度（pixels）。
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.y1.saturating_sub(self.y0).saturating_add(1)
    }
}

/// Tesseract `--psm` Page Segmentation Mode。
///
/// 取值与 C 版 `ocrtess_ocrwords_from_bmp8` 的 `segmode` 参数一致
/// （`willuslib/ocrtess.c:795` `segmode<0 || segmode>10 ? 6 : segmode`，但 tesseract
/// 5.x 实际范围是 0~13 共 14 种；默认 `Block (6)` 与 C 同源）。
///
/// 详见 `tesseract --help-psm`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageSegmentationMode {
    OsdOnly,
    AutoOsd,
    AutoOnly,
    Auto,
    SingleColumnVarSize,
    SingleUniformBlock,
    /// 默认（与 C 版 segmode=6 同源）。
    #[default]
    Block,
    SingleTextLine,
    SingleWord,
    CircleWord,
    SingleChar,
    SparseText,
    SparseTextOsd,
    RawLine,
}

impl PageSegmentationMode {
    /// 映射到 tesseract `--psm` 数值（0..=13）。
    #[must_use]
    pub const fn to_arg(self) -> &'static str {
        match self {
            PageSegmentationMode::OsdOnly => "0",
            PageSegmentationMode::AutoOsd => "1",
            PageSegmentationMode::AutoOnly => "2",
            PageSegmentationMode::Auto => "3",
            PageSegmentationMode::SingleColumnVarSize => "4",
            PageSegmentationMode::SingleUniformBlock => "5",
            PageSegmentationMode::Block => "6",
            PageSegmentationMode::SingleTextLine => "7",
            PageSegmentationMode::SingleWord => "8",
            PageSegmentationMode::CircleWord => "9",
            PageSegmentationMode::SingleChar => "10",
            PageSegmentationMode::SparseText => "11",
            PageSegmentationMode::SparseTextOsd => "12",
            PageSegmentationMode::RawLine => "13",
        }
    }

    /// 从整数构造（C `segmode<0||>13` 时落回 `Block`）。
    #[must_use]
    pub const fn from_i32(v: i32) -> Self {
        match v {
            0 => PageSegmentationMode::OsdOnly,
            1 => PageSegmentationMode::AutoOsd,
            2 => PageSegmentationMode::AutoOnly,
            3 => PageSegmentationMode::Auto,
            4 => PageSegmentationMode::SingleColumnVarSize,
            5 => PageSegmentationMode::SingleUniformBlock,
            7 => PageSegmentationMode::SingleTextLine,
            8 => PageSegmentationMode::SingleWord,
            9 => PageSegmentationMode::CircleWord,
            10 => PageSegmentationMode::SingleChar,
            11 => PageSegmentationMode::SparseText,
            12 => PageSegmentationMode::SparseTextOsd,
            13 => PageSegmentationMode::RawLine,
            _ => PageSegmentationMode::Block,
        }
    }
}

/// Tesseract `--oem` OCR Engine Mode。
///
/// 详见 `tesseract --help-oem`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OcrEngineMode {
    LegacyOnly,
    LstmOnly,
    LegacyAndLstm,
    /// 默认（tesseract 5.x 即 LSTM）。
    #[default]
    Default,
}

impl OcrEngineMode {
    /// 映射到 tesseract `--oem` 数值。
    #[must_use]
    pub const fn to_arg(self) -> &'static str {
        match self {
            OcrEngineMode::LegacyOnly => "0",
            OcrEngineMode::LstmOnly => "1",
            OcrEngineMode::LegacyAndLstm => "2",
            OcrEngineMode::Default => "3",
        }
    }
}

/// OCR 调用入参。
#[derive(Debug, Clone)]
pub struct OcrPageInput<'a> {
    /// 待识别的栅格化页面（任意 [`k2types::PixelFormat`]，引擎内部转 Gray8）。
    pub bitmap: &'a k2types::Bitmap,
    /// `None` 表示整页；否则在 ROI 内 OCR（与 C `ocrtess_ocrwords_from_bmp8` 的 x1/y1/x2/y2 一致）。
    pub roi: Option<OcrRoi>,
    /// Tesseract 语言代码（如 `eng` / `chi_sim` / `chi_sim+eng`）。空串 = 引擎默认（`eng`）。
    pub lang: String,
    /// 页面分割模式（默认 `Block` 与 C 同源）。
    pub psm: PageSegmentationMode,
    /// OCR 引擎模式（默认 `Default` 与 tesseract 5.x 同源）。
    pub oem: OcrEngineMode,
    /// 输入像素 DPI（用来调引擎内字号假设；与 [`k2settings::OcrSettings::ocr_dpi`] 同源）。
    pub dpi: f32,
    /// 过滤低置信度 word；范围 `0.0..=1.0`。`0.0` = 不过滤。
    pub min_confidence: f32,
}

impl<'a> OcrPageInput<'a> {
    /// 构造一个用 `eng` + 默认 PSM/OEM 的最简入参。
    #[must_use]
    pub fn new(bitmap: &'a k2types::Bitmap, dpi: f32) -> Self {
        Self {
            bitmap,
            roi: None,
            lang: String::new(),
            psm: PageSegmentationMode::Block,
            oem: OcrEngineMode::Default,
            dpi,
            min_confidence: 0.0,
        }
    }

    /// builder：指定 ROI。
    #[must_use]
    pub fn with_roi(mut self, roi: OcrRoi) -> Self {
        self.roi = Some(roi);
        self
    }

    /// builder：指定 lang。
    #[must_use]
    pub fn with_lang<S: Into<String>>(mut self, lang: S) -> Self {
        self.lang = lang.into();
        self
    }

    /// builder：指定 PSM。
    #[must_use]
    pub fn with_psm(mut self, psm: PageSegmentationMode) -> Self {
        self.psm = psm;
        self
    }

    /// builder：指定 OEM。
    #[must_use]
    pub fn with_oem(mut self, oem: OcrEngineMode) -> Self {
        self.oem = oem;
        self
    }

    /// builder：指定 min_confidence。
    #[must_use]
    pub fn with_min_confidence(mut self, min: f32) -> Self {
        self.min_confidence = min;
        self
    }

    /// 计算应该送给引擎的 ROI（None → 整页）。
    /// 返回的 OcrRoi 一定在 bitmap 范围内（或返回 `Err` 标记越界）。
    pub(crate) fn effective_roi(&self) -> Result<OcrRoi, OcrError> {
        let (w, h) = (self.bitmap.width, self.bitmap.height);
        if w == 0 || h == 0 {
            return Err(OcrError::EmptyBitmap);
        }
        match self.roi {
            None => Ok(OcrRoi::new(0, 0, w - 1, h - 1)),
            Some(roi) => {
                if roi.x0 > roi.x1 || roi.y0 > roi.y1 || roi.x1 >= w || roi.y1 >= h {
                    return Err(OcrError::RoiOutOfBounds {
                        roi,
                        bitmap_width: w,
                        bitmap_height: h,
                    });
                }
                Ok(roi)
            }
        }
    }
}

/// 引擎自检结果（`OcrEngine::probe` 返回）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrEngineInfo {
    /// 引擎短名（`"tesseract-cli"` / `"leptess-ffi"` / 自定义）。
    pub engine_name: String,
    /// 引擎版本字符串（如 `"5.5.0"`）。Tesseract CLI 走 `--version` 首行解析。
    pub version: String,
    /// 引擎 tessdata 目录（若已知）。
    pub data_path: Option<PathBuf>,
}

/// OCR 引擎错误。`Send + Sync` 以满足 trait object 在多线程中传递。
#[derive(Debug, Error)]
pub enum OcrError {
    /// `Command::new("tesseract")` 找不到二进制（PATH 找不到）。
    #[error("OCR 引擎可执行文件未找到: {engine}: {message}")]
    EngineNotFound { engine: String, message: String },

    /// 引擎版本不兼容（如 tesseract < 4 无 TSV）。
    #[error("OCR 引擎版本不兼容: {engine} = {version}, 要求 {required}")]
    UnsupportedVersion {
        engine: String,
        version: String,
        required: String,
    },

    /// `--list-langs` 显示请求的语言包未安装。
    #[error("OCR 语言包未安装: '{lang}'，可用: [{available}]")]
    LanguageNotInstalled { lang: String, available: String },

    /// 子进程 I/O 错误（stdin 写入 / stdout 读 / wait）。
    #[error("OCR 子进程 I/O 错误: {0}")]
    EngineIo(#[from] std::io::Error),

    /// 引擎调用退出码非 0 + stderr 摘要。
    #[error("OCR 子进程退出码非 0: exit_code={exit_code:?}, stderr={stderr}")]
    EngineExitNonZero {
        exit_code: Option<i32>,
        stderr: String,
    },

    /// 输出解析失败（TSV 格式异常）。
    #[error("OCR 输出解析失败: {0}")]
    OutputParse(String),

    /// Bitmap 编码失败（写 PNG/PGM 时 image crate 报错）。
    #[error("OCR 输入 Bitmap 编码失败: {0}")]
    BitmapEncoding(String),

    /// Bitmap 数据层错误（构造/越界 etc.）。
    #[error("OCR 输入 Bitmap 数据错误: {0}")]
    Bitmap(#[from] BitmapError),

    /// 入参 Bitmap 宽高为 0。
    #[error("OCR 输入 Bitmap 为空 (width=0 或 height=0)")]
    EmptyBitmap,

    /// 入参 ROI 越界。
    #[error("OCR 入参 ROI 越界: {roi:?}, bitmap={bitmap_width}x{bitmap_height}")]
    RoiOutOfBounds {
        roi: OcrRoi,
        bitmap_width: u32,
        bitmap_height: u32,
    },

    /// Cargo feature 关闭（如 `leptess` 未启用却调 `LeptessEngine`）。
    #[error("OCR feature 未启用: {0}")]
    FeatureDisabled(&'static str),

    /// Step 11.8 P0-5 新增：用户在 OCR 子进程执行过程中翻 [`crate::tesseract_cli::TesseractCliEngine`]
    /// 携带的 cancel token（`Arc<AtomicBool>`），引擎主动 kill 子进程后返回本变体。
    ///
    /// 调用方（[`k2pipeline::ConvertJob`]）应把本错误映射到
    /// `ConvertError::Cancelled` 而非 `ConvertError::Ocr`，以保留 POSIX 退出码
    /// 130（128 + SIGINT）语义，与 ADR-013 协作式取消 + Step 7.4 既定行为一致。
    #[error("ocr cancelled by user")]
    Cancelled,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use k2types::{Bitmap, PixelFormat};

    fn make_bitmap(w: u32, h: u32) -> Bitmap {
        Bitmap::new(w, h, 300.0, PixelFormat::Gray8).unwrap()
    }

    #[test]
    fn ocr_roi_width_height() {
        let r = OcrRoi::new(10, 20, 30, 50);
        assert_eq!(r.width(), 21);
        assert_eq!(r.height(), 31);
    }

    #[test]
    fn ocr_roi_single_pixel() {
        let r = OcrRoi::new(5, 5, 5, 5);
        assert_eq!(r.width(), 1);
        assert_eq!(r.height(), 1);
    }

    #[test]
    fn psm_to_arg_block_default() {
        let psm = PageSegmentationMode::default();
        assert_eq!(psm.to_arg(), "6");
    }

    #[test]
    fn psm_to_arg_all_variants() {
        let pairs: [(PageSegmentationMode, &str); 14] = [
            (PageSegmentationMode::OsdOnly, "0"),
            (PageSegmentationMode::AutoOsd, "1"),
            (PageSegmentationMode::AutoOnly, "2"),
            (PageSegmentationMode::Auto, "3"),
            (PageSegmentationMode::SingleColumnVarSize, "4"),
            (PageSegmentationMode::SingleUniformBlock, "5"),
            (PageSegmentationMode::Block, "6"),
            (PageSegmentationMode::SingleTextLine, "7"),
            (PageSegmentationMode::SingleWord, "8"),
            (PageSegmentationMode::CircleWord, "9"),
            (PageSegmentationMode::SingleChar, "10"),
            (PageSegmentationMode::SparseText, "11"),
            (PageSegmentationMode::SparseTextOsd, "12"),
            (PageSegmentationMode::RawLine, "13"),
        ];
        for (psm, expected) in pairs {
            assert_eq!(psm.to_arg(), expected, "mismatch for {psm:?}");
        }
    }

    #[test]
    fn psm_from_i32_invalid_falls_back_to_block() {
        assert_eq!(
            PageSegmentationMode::from_i32(-1),
            PageSegmentationMode::Block
        );
        assert_eq!(
            PageSegmentationMode::from_i32(14),
            PageSegmentationMode::Block
        );
        assert_eq!(
            PageSegmentationMode::from_i32(99),
            PageSegmentationMode::Block
        );
    }

    #[test]
    fn psm_from_i32_valid_round_trip() {
        for i in 0..=13 {
            let psm = PageSegmentationMode::from_i32(i);
            assert_eq!(psm.to_arg().parse::<i32>().unwrap(), i);
        }
    }

    #[test]
    fn oem_default_is_3() {
        assert_eq!(OcrEngineMode::default().to_arg(), "3");
    }

    #[test]
    fn oem_to_arg_all_variants() {
        let pairs = [
            (OcrEngineMode::LegacyOnly, "0"),
            (OcrEngineMode::LstmOnly, "1"),
            (OcrEngineMode::LegacyAndLstm, "2"),
            (OcrEngineMode::Default, "3"),
        ];
        for (oem, expected) in pairs {
            assert_eq!(oem.to_arg(), expected);
        }
    }

    #[test]
    fn ocr_page_input_new_defaults() {
        let bmp = make_bitmap(100, 100);
        let inp = OcrPageInput::new(&bmp, 300.0);
        assert_eq!(inp.bitmap.width, 100);
        assert!(inp.roi.is_none());
        assert!(inp.lang.is_empty());
        assert_eq!(inp.psm, PageSegmentationMode::Block);
        assert_eq!(inp.oem, OcrEngineMode::Default);
        assert!((inp.dpi - 300.0).abs() < 1e-6);
        assert!((inp.min_confidence - 0.0).abs() < 1e-6);
    }

    #[test]
    fn ocr_page_input_builder_chain() {
        let bmp = make_bitmap(50, 50);
        let inp = OcrPageInput::new(&bmp, 300.0)
            .with_lang("chi_sim+eng")
            .with_psm(PageSegmentationMode::SingleTextLine)
            .with_oem(OcrEngineMode::LstmOnly)
            .with_roi(OcrRoi::new(0, 0, 49, 49))
            .with_min_confidence(0.5);
        assert_eq!(inp.lang, "chi_sim+eng");
        assert_eq!(inp.psm, PageSegmentationMode::SingleTextLine);
        assert_eq!(inp.oem, OcrEngineMode::LstmOnly);
        assert!(inp.roi.is_some());
        assert!((inp.min_confidence - 0.5).abs() < 1e-6);
    }

    #[test]
    fn effective_roi_default_is_full_bitmap() {
        let bmp = make_bitmap(80, 60);
        let inp = OcrPageInput::new(&bmp, 300.0);
        let roi = inp.effective_roi().unwrap();
        assert_eq!(roi, OcrRoi::new(0, 0, 79, 59));
    }

    #[test]
    fn effective_roi_explicit_inside_bitmap() {
        let bmp = make_bitmap(100, 100);
        let inp = OcrPageInput::new(&bmp, 300.0).with_roi(OcrRoi::new(10, 20, 80, 90));
        let roi = inp.effective_roi().unwrap();
        assert_eq!(roi, OcrRoi::new(10, 20, 80, 90));
    }

    #[test]
    fn effective_roi_out_of_bounds_x() {
        let bmp = make_bitmap(100, 100);
        let inp = OcrPageInput::new(&bmp, 300.0).with_roi(OcrRoi::new(0, 0, 100, 50));
        assert!(matches!(
            inp.effective_roi(),
            Err(OcrError::RoiOutOfBounds { .. })
        ));
    }

    #[test]
    fn effective_roi_out_of_bounds_y() {
        let bmp = make_bitmap(100, 100);
        let inp = OcrPageInput::new(&bmp, 300.0).with_roi(OcrRoi::new(0, 0, 50, 100));
        assert!(matches!(
            inp.effective_roi(),
            Err(OcrError::RoiOutOfBounds { .. })
        ));
    }

    #[test]
    fn effective_roi_inverted_returns_err() {
        let bmp = make_bitmap(100, 100);
        let inp = OcrPageInput::new(&bmp, 300.0).with_roi(OcrRoi::new(50, 50, 10, 10));
        assert!(matches!(
            inp.effective_roi(),
            Err(OcrError::RoiOutOfBounds { .. })
        ));
    }

    #[test]
    fn effective_roi_empty_bitmap_zero_width() {
        // Bitmap::new(0, h, ...) 会被 k2types 拒（pixel_len=0），但用 from_raw 也可强构造。
        // 这里直接构造一个空 vec 旁路 (走 from_raw 测试)
        let bmp = Bitmap::from_raw(0, 100, 300.0, PixelFormat::Gray8, Vec::new());
        // 不同 BitmapError 路径，先看 new 走 SizeOverflow
        // 这里只验证 effective_roi 在 width=0 时 EmptyBitmap
        if let Ok(bmp) = bmp {
            let inp = OcrPageInput::new(&bmp, 300.0);
            assert!(matches!(inp.effective_roi(), Err(OcrError::EmptyBitmap)));
        }
    }

    #[test]
    fn ocr_engine_info_eq() {
        let a = OcrEngineInfo {
            engine_name: "tesseract-cli".to_string(),
            version: "5.5.0".to_string(),
            data_path: None,
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn error_display_engine_not_found() {
        let e = OcrError::EngineNotFound {
            engine: "tesseract".to_string(),
            message: "PATH lookup failed".to_string(),
        };
        let s = format!("{e}");
        assert!(s.contains("tesseract"));
        assert!(s.contains("PATH"));
    }

    #[test]
    fn error_display_unsupported_version() {
        let e = OcrError::UnsupportedVersion {
            engine: "tesseract".to_string(),
            version: "3.0.0".to_string(),
            required: ">= 4.0".to_string(),
        };
        let s = format!("{e}");
        assert!(s.contains("3.0.0"));
        assert!(s.contains(">= 4.0"));
    }

    #[test]
    fn error_display_feature_disabled() {
        let e = OcrError::FeatureDisabled("leptess feature 关闭");
        let s = format!("{e}");
        assert!(s.contains("leptess"));
    }
}
