//! `k2settings` - Settings data model, device profiles, page list, and Settings <-> args bidirectional serialization
//!
//! Source C files: `k2settings.c` (1123 lines), `k2settings2cmd.c` (928 lines),
//! `devprofile.c` (166 lines), `pagelist.c` (409 lines).
//!
//! v2.1 key change: `k2settings2cmd.c` lives exclusively here (not split into `k2cli`).
//!
//! See `rust-rewrite-plan.md` v2.1 sections 5.2 / 8.1.
//! This crate is a placeholder in M0; real implementation in M1 (Step 3.1 - 3.5).

#![forbid(unsafe_code)]

pub mod behavior;
pub mod destination;
pub mod device;
pub mod layout;
pub mod ocr;
pub mod output;
pub mod pagelist;
pub mod serialize;
pub mod settings;
pub mod source;

pub use behavior::BehaviorSettings;
pub use destination::DestinationSettings;
pub use device::{count, find_by_alias, list_devices, DeviceProfile, DEVICES};
pub use layout::{LayoutSettings, ReflowMode, TextWrap};
pub use ocr::{OcrDetectionType, OcrMode, OcrSettings, OcrStrictMode, OcrVisibility};
pub use output::OutputSettings;
pub use pagelist::{
    count as pagelist_count, includes_cover, includes_page, is_valid, parse as pagelist_parse,
    PageRangeItem, Parity,
};
pub use settings::Settings;
pub use source::{CropBox, MarginUnit, SourceSettings, SrcRotation};
