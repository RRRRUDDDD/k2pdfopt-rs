//! Source settings — fields from K2PDFOPT_SETTINGS related to source document processing.
//!
//! Source C: `k2pdfopt.h:237-442` (K2PDFOPT_SETTINGS struct)
//! Default init: `k2settings.c:31-241` (k2pdfopt_settings_init)

// from k2pdfopt.h:258
/// Gap threshold between columns, in inches.
/// Default 0.005 from k2settings.c:48.
const DEFAULT_GTC_IN: f64 = 0.005;

// from k2pdfopt.h:257
/// Gap threshold between rows, in inches.
/// Default 0.006 from k2settings.c:49.
const DEFAULT_GTR_IN: f64 = 0.006;

// from k2pdfopt.h:258
/// Gap threshold between words, in inches.
/// Default 0.0015 from k2settings.c:50.
const DEFAULT_GTW_IN: f64 = 0.0015;

/// Source rotation mode.
/// C uses SRCROT_AUTO = -999.0 (k2pdfopt.h:161).
/// Rust uses an enum for clarity.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SrcRotation {
    /// Auto-detect rotation (SRCROT_AUTO = -999.0)
    // from k2pdfopt.h:161
    #[default]
    Auto,
    /// Auto-detect with even-page priority (SRCROT_AUTOEP = -998.0)
    // from k2pdfopt.h:162
    AutoEvenPage,
    /// Auto-detect with preview (SRCROT_AUTOPREV = -997.0)
    // from k2pdfopt.h:163
    AutoPreview,
    /// Explicit angle in degrees
    Angle(f64),
}

impl SrcRotation {
    /// Convert to the C-compatible f64 value.
    // from k2pdfopt.h:161-163
    pub fn to_c_value(&self) -> f64 {
        match self {
            SrcRotation::Auto => -999.0,
            SrcRotation::AutoEvenPage => -998.0,
            SrcRotation::AutoPreview => -997.0,
            SrcRotation::Angle(a) => *a,
        }
    }

    /// Convert from C-compatible f64 value.
    pub fn from_c_value(v: f64) -> Self {
        if (v - (-999.0)).abs() < 0.5 {
            SrcRotation::Auto
        } else if (v - (-998.0)).abs() < 0.5 {
            SrcRotation::AutoEvenPage
        } else if (v - (-997.0)).abs() < 0.5 {
            SrcRotation::AutoPreview
        } else {
            SrcRotation::Angle(v)
        }
    }
}

/// Unit type for margin/box dimensions.
/// Maps to UNITS_* constants in k2pdfopt.h:151-156.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum MarginUnit {
    /// UNITS_PIXELS = 0
    Pixels = 0,
    /// UNITS_INCHES = 1
    #[default]
    Inches = 1,
    /// UNITS_CM = 2
    Cm = 2,
    /// UNITS_SOURCE = 3
    Source = 3,
    /// UNITS_TRIMMED = 4
    Trimmed = 4,
    /// UNITS_OCRLAYER = 5
    OcrLayer = 5,
}

/// Crop box — corresponds to K2CROPBOX in k2pdfopt.h:179-185.
#[derive(Debug, Clone, PartialEq)]
pub struct CropBox {
    /// Page list filter (pagelist[256] in C).
    // from k2pdfopt.h:181
    pub pagelist: String,
    /// box[4]: left, top, width, height.
    // from k2pdfopt.h:182
    pub box_vals: [f64; 4],
    /// units[4]: margin unit for each box dimension.
    // from k2pdfopt.h:183
    pub units: [MarginUnit; 4],
    /// cboxflags — see K2CROPBOX_FLAGS_*.
    // from k2pdfopt.h:184
    pub cboxflags: i32,
}

impl Default for CropBox {
    fn default() -> Self {
        // from k2settings.c:104-110 (dstmargins init)
        // and k2settings.c:115-121 (srccropmargins init)
        Self {
            pagelist: String::new(),
            box_vals: [0.0; 4],
            units: [MarginUnit::Inches; 4],
            cboxflags: 0,
        }
    }
}

/// Source settings — fields controlling how source documents are read and pre-processed.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceSettings {
    // from k2pdfopt.h:255
    /// Source rotation mode (src_rot). Default SRCROT_AUTO.
    pub src_rot: SrcRotation,

    // from k2pdfopt.h:256
    /// Gap threshold between columns, in inches (gtc_in). Default 0.005.
    pub gtc_in: f64,

    // from k2pdfopt.h:257
    /// Gap threshold between rows, in inches (gtr_in). Default 0.006.
    pub gtr_in: f64,

    // from k2pdfopt.h:258
    /// Gap threshold between words, in inches (gtw_in). Default 0.0015.
    pub gtw_in: f64,

    // from k2pdfopt.h:260
    /// Read source left-to-right (src_left_to_right). Default 1.
    pub src_left_to_right: bool,

    // from k2pdfopt.h:261
    /// White threshold for source (src_whitethresh). Default -1 (auto).
    pub src_whitethresh: i32,

    // from k2pdfopt.h:265
    /// Paint everything above whitethresh white (src_paintwhite). Default 0.
    pub src_paintwhite: bool,

    // from k2pdfopt.h:303
    /// User-specified source DPI (user_src_dpi). Default -2.0.
    pub user_src_dpi: f64,

    // from k2pdfopt.h:304
    /// Document scale factor. Default 1.0.
    pub document_scale_factor: f64,

    // from k2pdfopt.h:305
    /// Source DPI (src_dpi). Default 300.
    pub src_dpi: i32,

    // from k2pdfopt.h:306
    /// User preference: use Ghostscript (user_usegs). Default platform-dependent.
    pub user_usegs: bool,

    // from k2pdfopt.h:307
    /// Active: use Ghostscript (usegs). Default same as user_usegs.
    pub usegs: bool,

    // from k2pdfopt.h:330
    /// Auto-straighten source pages (src_autostraighten). Default 0.
    pub src_autostraighten: bool,

    // from k2pdfopt.h:352
    /// Source crop margins (srccropmargins). Default all-zero.
    pub srccropmargins: CropBox,

    // from k2pdfopt.h:387
    /// Trim source margins (src_trim). Default 1.
    pub src_trim: bool,

    // from k2pdfopt.h:388
    /// Erase vertical lines (erase_vertical_lines). Default 0.
    pub erase_vertical_lines: bool,

    // from k2pdfopt.h:389
    /// Erase horizontal lines (erase_horizontal_lines). Default 0.
    pub erase_horizontal_lines: bool,

    // from k2pdfopt.h:395-396
    /// Grid rows (src_grid_rows). Default -1 (not used).
    pub src_grid_rows: i32,

    /// Grid columns (src_grid_cols). Default -1 (not used).
    pub src_grid_cols: i32,

    // from k2pdfopt.h:397
    /// Grid order (grid_order). Two-digit code. Default -1.
    pub grid_order: i32,

    // from k2pdfopt.h:399
    /// Grid overlap percentage (src_grid_overlap_percentage). Default 2.0.
    pub src_grid_overlap_percentage: f64,

    // from k2pdfopt.h:438
    /// Source erosion filter (src_erosion). Default 0.
    pub src_erosion: i32,

    // from k2pdfopt.h:440
    /// Detect double/triple text rows (detect_double_rows). Default 1.
    pub detect_double_rows: bool,

    // from k2pdfopt.h:441
    /// Minimum text row height in pts (textheight_min_pts). Default -1.0 (not used).
    pub textheight_min_pts: f64,
}

impl Default for SourceSettings {
    fn default() -> Self {
        // Default values from k2settings.c:31-241
        Self {
            // k2settings.c:47
            src_rot: SrcRotation::Auto,
            // k2settings.c:48
            gtc_in: DEFAULT_GTC_IN,
            // k2settings.c:49
            gtr_in: DEFAULT_GTR_IN,
            // k2settings.c:50
            gtw_in: DEFAULT_GTW_IN,
            // k2settings.c:52
            src_left_to_right: true,
            // k2settings.c:53
            src_whitethresh: -1,
            // k2settings.c:54
            src_paintwhite: false,
            // k2settings.c:80
            user_src_dpi: -2.0,
            // k2settings.c:81
            document_scale_factor: 1.0,
            // k2settings.c:82
            src_dpi: 300,
            // k2settings.c:83-87 — with mupdf (which we use), default is 0 (false)
            user_usegs: false,
            // k2settings.c:88
            usegs: false,
            // k2settings.c:103
            src_autostraighten: false,
            // k2settings.c:115-121
            srccropmargins: CropBox {
                pagelist: String::new(),
                box_vals: [0.0; 4],
                units: [MarginUnit::Inches; 4],
                cboxflags: 0,
            },
            // k2settings.c:144
            src_trim: true,
            // k2settings.c:145
            erase_vertical_lines: false,
            // k2settings.c:214
            erase_horizontal_lines: false,
            // k2settings.c:150
            src_grid_rows: -1,
            // k2settings.c:151
            src_grid_cols: -1,
            // k2settings.c:152
            grid_order: -1,
            // k2settings.c:153
            src_grid_overlap_percentage: 2.0,
            // k2settings.c:232
            src_erosion: 0,
            // k2settings.c:239
            detect_double_rows: true,
            // k2settings.c:240
            textheight_min_pts: -1.0,
        }
    }
}
