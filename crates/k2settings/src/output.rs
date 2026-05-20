//! Output settings — fields from K2PDFOPT_SETTINGS related to PDF/bitmap output generation.
//!
//! Source C: `k2pdfopt.h:237-442` (K2PDFOPT_SETTINGS struct)
//! Default init: `k2settings.c:31-241` (k2pdfopt_settings_init)

/// Output settings — output format, DPI, quality, color, margins, page breaks.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputSettings {
    // from k2pdfopt.h:299
    /// Dither output (dst_dither). Default 1.
    pub dst_dither: bool,

    // from k2pdfopt.h:300
    /// Break pages (dst_break_pages). Default 1.
    pub dst_break_pages: bool,

    // from k2pdfopt.h:301
    /// Render DPI (render_dpi). Default 167.
    pub render_dpi: i32,

    // from k2pdfopt.h:310
    /// JPEG quality (jpeg_quality). Default -1 (auto).
    pub jpeg_quality: i32,

    // from k2pdfopt.h:319
    /// Justification mode (dst_justify). -1 = default (left). 0 = left, 1 = center.
    pub dst_justify: i32,

    // from k2pdfopt.h:320
    /// Figure justification (dst_figure_justify). -1 = same as dst_justify.
    pub dst_figure_justify: i32,

    // from k2pdfopt.h:321
    /// Rotate figures to landscape (dst_figure_rotate). Default 0.
    pub dst_figure_rotate: bool,

    // from k2pdfopt.h:322
    /// Minimum figure height in inches (dst_min_figure_height_in). Default 0.75.
    pub dst_min_figure_height_in: f64,

    // from k2pdfopt.h:323
    /// Full justification (dst_fulljustify). -1 = default (no). 0 = no, 1 = yes.
    pub dst_fulljustify: i32,

    // from k2pdfopt.h:324
    /// Sharpen output (dst_sharpen). Default 1.
    pub dst_sharpen: i32,

    // from k2pdfopt.h:325
    /// Color output (dst_color). Default set by device profile.
    pub dst_color: i32,

    // from k2pdfopt.h:326
    /// Bits per component (dst_bpc). Default 4.
    pub dst_bpc: i32,

    // from k2pdfopt.h:329
    /// Output name format (dst_opname_format). Default "%s_k2opt".
    pub dst_opname_format: String,

    // from k2pdfopt.h:262
    /// Foreground color (dst_fgcolor). Default empty.
    pub dst_fgcolor: String,

    // from k2pdfopt.h:263
    /// Foreground type (dst_fgtype). 0=none, 1=gray, 2=color, 3=bitmap. Default 0.
    pub dst_fgtype: i32,

    // from k2pdfopt.h:264
    /// Background color (dst_bgcolor). Default empty.
    pub dst_bgcolor: String,

    // from k2pdfopt.h:265
    /// Background type (dst_bgtype). 0=none, 1=gray, 2=color, 3=bitmap. Default 0.
    pub dst_bgtype: i32,

    // from k2pdfopt.h:375
    /// Original DPI (dpi_org). Default set by device profile.
    pub dpi_org: f64,

    // from k2pdfopt.h:376
    /// Maximum contrast (contrast_max). Default 2.0.
    pub contrast_max: f64,

    // from k2pdfopt.h:377
    /// Gamma correction (dst_gamma). Default 0.5.
    pub dst_gamma: f64,

    // from k2pdfopt.h:378
    /// Negative mode (dst_negative). 0=off, 1=text only, 2=all. Default 0.
    pub dst_negative: i32,

    // from k2pdfopt.h:268
    /// Text only mode — don't send figures (text_only). Default 0.
    pub text_only: bool,

    // from k2pdfopt.h:425
    /// Output author metadata (dst_author). Default empty.
    pub dst_author: String,

    // from k2pdfopt.h:426
    /// Output title metadata (dst_title). Default empty.
    pub dst_title: String,

    // from k2pdfopt.h:428
    /// Output font size in points (dst_fontsize_pts). 0 = not used. Default 0.
    pub dst_fontsize_pts: f64,

    // from k2pdfopt.h:430
    /// Cover image path (dst_coverimage). Default empty.
    pub dst_coverimage: String,

    // from k2pdfopt.h:423
    /// Page break mark color for break (pagebreakmark_breakpage_color). -1 = no mark.
    pub pagebreakmark_breakpage_color: i32,

    // from k2pdfopt.h:424
    /// Page break mark color for no-break (pagebreakmark_nobreak_color). -1 = no mark.
    pub pagebreakmark_nobreak_color: i32,
}

impl Default for OutputSettings {
    fn default() -> Self {
        // Default values from k2settings.c:31-241
        Self {
            // k2settings.c:76
            dst_dither: true,
            // k2settings.c:77
            dst_break_pages: true,
            // k2settings.c:78
            render_dpi: 167,
            // k2settings.c:91
            jpeg_quality: -1,
            // k2settings.c:94
            dst_justify: -1,
            // k2settings.c:95
            dst_figure_justify: -1,
            // k2settings.c:210
            dst_figure_rotate: false,
            // k2settings.c:96
            dst_min_figure_height_in: 0.75,
            // k2settings.c:97
            dst_fulljustify: -1,
            // k2settings.c:98
            dst_sharpen: 1,
            // "kv" profile sets dst_color via device
            dst_color: 0,
            // k2settings.c:99
            dst_bpc: 4,
            // k2settings.c:102
            dst_opname_format: String::from("%s_k2opt"),
            // k2settings.c:195
            dst_fgcolor: String::new(),
            // k2settings.c:197
            dst_fgtype: 0,
            // k2settings.c:196
            dst_bgcolor: String::new(),
            // k2settings.c:198
            dst_bgtype: 0,
            // Set by device profile via k2pdfopt_settings_set_to_device
            dpi_org: 167.0,
            // k2settings.c:132
            contrast_max: 2.0,
            // k2settings.c:133
            dst_gamma: 0.5,
            // k2settings.c:134
            dst_negative: 0,
            // k2settings.c:189
            text_only: false,
            // k2settings.c:216
            dst_author: String::new(),
            // k2settings.c:217
            dst_title: String::new(),
            // k2settings.c:220
            dst_fontsize_pts: 0.0,
            // k2settings.c:222
            dst_coverimage: String::new(),
            // k2settings.c:212
            pagebreakmark_breakpage_color: -1,
            // k2settings.c:213
            pagebreakmark_nobreak_color: -1,
        }
    }
}
