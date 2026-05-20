//! Destination settings — fields from K2PDFOPT_SETTINGS related to output device.
//!
//! Source C: `k2pdfopt.h:237-442` (K2PDFOPT_SETTINGS struct)
//! Default init: `k2settings.c:31-241` (k2pdfopt_settings_init)

use crate::source::{CropBox, MarginUnit};

/// Destination settings — output device dimensions, DPI, margins, padding.
#[derive(Debug, Clone, PartialEq)]
pub struct DestinationSettings {
    // from k2pdfopt.h:297
    /// User-specified device DPI (dst_userdpi). Default set by device profile.
    pub dst_userdpi: i32,

    // from k2pdfopt.h:298
    /// Device virtual DPI (dst_dpi). Default set by device profile.
    pub dst_dpi: i32,

    // from k2pdfopt.h:311
    /// Full device width in pixels (dst_width). Default from device profile.
    pub dst_width: i32,

    // from k2pdfopt.h:312
    /// Full device height in pixels (dst_height). Default from device profile.
    pub dst_height: i32,

    // from k2pdfopt.h:313
    /// User-specified width in user units (dst_userwidth). Default from device profile.
    pub dst_userwidth: f64,

    // from k2pdfopt.h:314
    /// User-specified height in user units (dst_userheight). Default from device profile.
    pub dst_userheight: f64,

    // from k2pdfopt.h:317
    /// Units for user width (dst_userwidth_units). Default 0 = pixels.
    pub dst_userwidth_units: i32,

    // from k2pdfopt.h:318
    /// Units for user height (dst_userheight_units). Default 0 = pixels.
    pub dst_userheight_units: i32,

    // from k2pdfopt.h:315
    /// Magnification factor (dst_magnification). Default 1.0.
    pub dst_magnification: f64,

    // from k2pdfopt.h:316
    /// Display resolution factor (dst_display_resolution). Default 1.0.
    pub dst_display_resolution: f64,

    // from k2pdfopt.h:367
    /// Device width in inches (display_width_inches). Default 3.6.
    pub display_width_inches: f64,

    // from k2pdfopt.h:342
    /// Destination margins (dstmargins). Default 0.02 inches each.
    pub dstmargins: CropBox,

    // from k2pdfopt.h:343
    /// Original destination margins before overrides (dstmargins_org).
    pub dstmargins_org: CropBox,

    // from k2pdfopt.h:344
    /// Left padding in pixels (pad_left). Default set by device profile.
    pub pad_left: i32,

    // from k2pdfopt.h:345
    /// Right padding in pixels (pad_right). Default set by device profile.
    pub pad_right: i32,

    // from k2pdfopt.h:346
    /// Bottom padding in pixels (pad_bottom). Default set by device profile.
    pub pad_bottom: i32,

    // from k2pdfopt.h:347
    /// Top padding in pixels (pad_top). Default set by device profile.
    pub pad_top: i32,

    // from k2pdfopt.h:348
    /// Mark corner pixels (mark_corners). Default set by device profile.
    pub mark_corners: i32,

    // from k2pdfopt.h:417
    /// Device size set flag (devsize_set). Default 0.
    pub devsize_set: i32,

    /// Canonical alias of the device profile applied via --dev.
    /// `None` if no device profile was explicitly selected.
    /// Used by `Settings::to_args()` for compact roundtrip serialization.
    pub device_alias: Option<String>,

    // from k2pdfopt.h:327
    /// Landscape mode (dst_landscape). Default 0.
    pub dst_landscape: bool,

    // from k2pdfopt.h:328
    /// Pages to render in landscape (dst_landscape_pages). Default empty.
    pub dst_landscape_pages: String,
}

impl Default for DestinationSettings {
    fn default() -> Self {
        // Default values from k2settings.c:31-241
        // Note: Device profile defaults are applied via k2pdfopt_settings_set_to_device()
        // which sets dst_userdpi, dst_dpi, dst_width, dst_height, dst_userwidth, dst_userheight,
        // pad_left, pad_right, pad_bottom, pad_top, mark_corners, display_width_inches.
        // We use the default "kv" profile values here.
        Self {
            // k2pdfopt.h:158 — DEFAULT_WIDTH = 560
            dst_width: 560,
            // k2pdfopt.h:159 — DEFAULT_HEIGHT = 745
            dst_height: 745,
            // "kv" profile: 600x800 (from device.rs, index 0)
            dst_userwidth: 600.0,
            dst_userheight: 800.0,
            // "kv" profile: dpi = 167
            dst_userdpi: 167,
            // dst_dpi = dst_userdpi initially
            dst_dpi: 167,
            dst_userwidth_units: 0,
            dst_userheight_units: 0,
            // k2settings.c:92
            dst_magnification: 1.0,
            // k2settings.c:93
            dst_display_resolution: 1.0,
            // k2settings.c:129
            display_width_inches: 3.6,
            // k2settings.c:104-110
            dstmargins: CropBox {
                pagelist: String::new(),
                box_vals: [0.02; 4],
                units: [MarginUnit::Inches; 4],
                cboxflags: 0,
            },
            dstmargins_org: CropBox {
                pagelist: String::new(),
                box_vals: [0.02; 4],
                units: [MarginUnit::Inches; 4],
                cboxflags: 0,
            },
            // "kv" profile: pad_left/right/bottom/top from device
            pad_left: 0,
            pad_right: 0,
            pad_bottom: 0,
            pad_top: 0,
            // "kv" profile: mark_corners from device
            mark_corners: 0,
            // k2settings.c:185
            devsize_set: 0,
            device_alias: None,
            // k2settings.c:100
            dst_landscape: false,
            // k2settings.c:101
            dst_landscape_pages: String::new(),
        }
    }
}
