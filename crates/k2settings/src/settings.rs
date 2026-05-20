//! Top-level Settings — composition of 6 sub-structs.
//!
//! Maps to C `K2PDFOPT_SETTINGS` (k2pdfopt.h:237-442).
//! Default init maps to `k2pdfopt_settings_init()` (k2settings.c:31-241).
//!
//! v2.1 plan §8.1 groups the ~150 fields into:
//! source / destination / layout / output / ocr / behavior

use crate::behavior::BehaviorSettings;
use crate::destination::DestinationSettings;
use crate::device::DeviceProfile;
use crate::layout::LayoutSettings;
use crate::ocr::OcrSettings;
use crate::output::OutputSettings;
use crate::source::SourceSettings;

/// Top-level settings container — all user-configurable processing parameters.
///
/// Field grouping per v2.1 plan §8.1:
/// - `source`: source document reading & pre-processing
/// - `destination`: output device dimensions, DPI, margins, padding
/// - `layout`: column detection, text wrapping, indentation
/// - `output`: PDF/bitmap output format, quality, color
/// - `ocr`: OCR engine, language, visibility
/// - `behavior`: runtime flags, page lists, threading
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub source: SourceSettings,
    pub destination: DestinationSettings,
    pub layout: LayoutSettings,
    pub output: OutputSettings,
    pub ocr: OcrSettings,
    pub behavior: BehaviorSettings,
}

// Note: Settings::default() uses Default auto-derivation which calls
// Default::default() on each sub-struct. Each sub-struct's Default impl
// matches k2pdfopt_settings_init() field-for-field.

impl Settings {
    /// Apply a device profile to the settings, mirroring C's
    /// `k2pdfopt_settings_set_to_device()`. Overwrites destination dimensions,
    /// DPI, color, padding, and mark_corners from the device profile.
    pub fn apply_device_profile(&mut self, dp: &DeviceProfile) {
        self.destination.dst_userwidth = f64::from(dp.width);
        self.destination.dst_userwidth_units = 0; // UNITS_PIXELS
        self.destination.dst_userheight = f64::from(dp.height);
        self.destination.dst_userheight_units = 0; // UNITS_PIXELS
        self.destination.dst_userdpi = i32::from(dp.dpi);
        self.destination.dst_dpi = i32::from(dp.dpi);
        self.destination.mark_corners = i32::from(dp.mark_corners);
        self.destination.pad_left = i32::from(dp.padding[0]);
        self.destination.pad_top = i32::from(dp.padding[1]);
        self.destination.pad_right = i32::from(dp.padding[2]);
        self.destination.pad_bottom = i32::from(dp.padding[3]);
        self.output.dst_color = i32::from(dp.color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_smoke() {
        let s = Settings::default();
        // Spot-check a few values from each sub-struct to verify wiring
        assert_eq!(s.behavior.verbose, 0);
        assert_eq!(s.source.src_dpi, 300);
        assert!((s.layout.cdthresh - 0.01).abs() < f64::EPSILON);
        assert_eq!(s.output.render_dpi, 167);
        assert_eq!(s.ocr.ocr_dpi, 300);
        assert_eq!(s.destination.dst_width, 560);
    }
}
