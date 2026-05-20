//! Behavior settings — fields from K2PDFOPT_SETTINGS related to runtime behavior,
//! user interaction, and miscellaneous flags.
//!
//! Source C: `k2pdfopt.h:237-442` (K2PDFOPT_SETTINGS struct)
//! Default init: `k2settings.c:31-241` (k2pdfopt_settings_init)

/// Behavior settings — debugging, user queries, page lists, TOC, threading, etc.
#[derive(Debug, Clone, PartialEq)]
pub struct BehaviorSettings {
    // from k2pdfopt.h:240
    /// Verbosity level (verbose). Default 0.
    pub verbose: i32,

    // from k2pdfopt.h:241
    /// Debug level (debug). Default 0.
    pub debug: i32,

    // from k2pdfopt.h:259
    /// Show usage string (show_usage). Default empty.
    pub show_usage: String,

    // from k2pdfopt.h:308
    /// Query user for input (query_user). Default -1 (auto).
    pub query_user: i32,

    // from k2pdfopt.h:309
    /// Query user was explicitly set (query_user_explicit). Default 0.
    pub query_user_explicit: bool,

    // from k2pdfopt.h:338
    /// Auto-crop mode (autocrop). Default 0.
    pub autocrop: bool,

    // from k2pdfopt.h:341 (ifdef HAVE_LEPTONICA_LIB — always available in Rust)
    /// Dewarp mode (dewarp). Default 0.
    pub dewarp: bool,

    // from k2pdfopt.h:368
    /// Page list filter (pagelist). Default empty.
    pub pagelist: String,

    // from k2pdfopt.h:369
    /// Exclude page list (pagexlist). Default empty.
    pub pagexlist: String,

    // from k2pdfopt.h:370
    /// Page break list (bpl). Default empty.
    pub bpl: String,

    // from k2pdfopt.h:371
    /// Use table of contents (use_toc). Default -1.
    pub use_toc: i32,

    // from k2pdfopt.h:372
    /// TOC list (toclist). Default empty.
    pub toclist: String,

    // from k2pdfopt.h:373
    /// TOC save file (tocsavefile). Default empty.
    pub tocsavefile: String,

    // from k2pdfopt.h:379
    /// Exit on complete (exit_on_complete). Default -1.
    pub exit_on_complete: i32,

    // from k2pdfopt.h:380
    /// Show marked source (show_marked_source). Default 0.
    pub show_marked_source: bool,

    // from k2pdfopt.h:381
    /// Use crop boxes (use_crop_boxes). Default 0.
    pub use_crop_boxes: bool,

    // from k2pdfopt.h:406
    /// Preview page number (preview_page). 0 = no preview. Default 0.
    pub preview_page: i32,

    // from k2pdfopt.h:407
    /// Echo source page count (echo_source_page_count). Default 0.
    pub echo_source_page_count: bool,

    // from k2pdfopt.h:422
    /// Info-only mode (info). Default 0.
    pub info: bool,

    // from k2pdfopt.h:429
    /// Assume yes to overwrite (assume_yes). Default 0.
    pub assume_yes: bool,

    // from k2pdfopt.h:432
    /// User has adjusted magnification (user_mag). 0=no, 1=-odpi, 2=-fs. Default 0.
    pub user_mag: i32,

    // from k2pdfopt.h:433-434
    /// Join figure captions (join_figure_captions). 1=yes, 2=even multi-column. Default 1.
    pub join_figure_captions: i32,

    // from k2pdfopt.h:436
    /// Number of threads (nthreads). Negative = percent of CPUs. Default -50.
    pub nthreads: i32,
}

impl Default for BehaviorSettings {
    fn default() -> Self {
        // Default values from k2settings.c:31-241
        Self {
            // k2settings.c:36
            verbose: 0,
            // k2settings.c:37
            debug: 0,
            // k2settings.c:51
            show_usage: String::new(),
            // k2settings.c:89
            query_user: -1,
            // k2settings.c:90
            query_user_explicit: false,
            // k2settings.c:207
            autocrop: false,
            // k2settings.c:236-237 (HAVE_LEPTONICA_LIB)
            dewarp: false,
            // k2settings.c:130
            pagelist: String::new(),
            // k2settings.c:215
            pagexlist: String::new(),
            // k2settings.c:181
            bpl: String::new(),
            // k2settings.c:178
            use_toc: -1,
            // k2settings.c:179
            toclist: String::new(),
            // k2settings.c:180
            tocsavefile: String::new(),
            // k2settings.c:135
            exit_on_complete: -1,
            // k2settings.c:136
            show_marked_source: false,
            // k2settings.c:137
            use_crop_boxes: false,
            // k2settings.c:155
            preview_page: 0,
            // k2settings.c:156
            echo_source_page_count: false,
            // k2settings.c:211
            info: false,
            // k2settings.c:221
            assume_yes: false,
            // k2settings.c:225
            user_mag: 0,
            // k2settings.c:226
            join_figure_captions: 1,
            // k2settings.c:229
            nthreads: -50,
        }
    }
}
