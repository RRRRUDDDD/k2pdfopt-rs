//! Structured help text for k2pdfopt-rs, organized by sections.
//!
//! Source C: `k2usage.c` (1341 lines) — hardcode help text.
//! Rust version: section-organized, markdown-style, covering M1 high-frequency
//! parameters plus context for the full C parameter set.

/// Usage introduction — mirrors C `usageintro`.
pub const USAGE_INTRO: &str = "\
USAGE:
    k2pdfopt [OPTIONS] <input pdf/djvu | folder>...

DESCRIPTION:
    Optimizes PDF/DJVU files for e-reader display by detecting rectangular
    regions and re-paginating without margins and excess white space.
    Works best on native PDF files (not scanned). Output is always PDF.

    If given a folder, k2pdfopt first looks for bitmaps and converts those,
    then processes any PDF files in sequence.

ENVIRONMENT:
    K2PDFOPT — command-line options via environment variable.
    Example: set K2PDFOPT=--ui- -x -j 0 -m 0.25
    CLI options take precedence over environment variable.";

/// Device section help.
pub const SECTION_DEVICE: &str = "\
DEVICE SELECTION:
  --dev <PROFILE>     Select device profile (sets width, height, dpi, etc.)
                      Partial names accepted if unique. Use --list-devices
                      to see all profiles. Default: kv (Kindle Voyage/PW3+)
  --list-devices      List all device profiles and exit";

/// Layout section help.
pub const SECTION_LAYOUT: &str = "\
LAYOUT & MARGINS:
  -m, --margins <M>   Source crop margins. Comma-separated L,T,R,B values.
                      Single value applies to all four. Default unit: inches.
                      Suffix: s=source-relative, t=trimmed-relative, cm, in.
                      Negative values (no suffix) = source-relative.
  --om <M>            Output margins (same format as -m)
  -t, --trim          Trim source margins [default: on]
      --no-t          Disable trimming
      --fc            Fit columns to screen width [default: on]
      --no-fc         Disable fit-columns
      --wrap          Enable text wrapping
      --wrap-extra    Extra text wrapping (C's -wrap+)
      --no-wrap       Disable text wrapping
      --ls            Landscape orientation
      --ls-pages <P>  Landscape for specific pages
      --no-ls         Disable landscape
  -j, --justify <M>   Justification: 0=left, 1=center. Suffix + for
                      full-justify, - for no full-justify. Examples: 0, 1+, 0-";

/// Page selection help.
pub const SECTION_PAGES: &str = "\
PAGE SELECTION:
  -p, --pages <RANGE>  Pages to process (e.g. 1-10, 1,3,5, even, odd)
                       Prefix 'e' for even, 'o' for odd: e1-10, o5-20
  --px <RANGE>         Pages to exclude (same format as -p)
  -c, --cover          Include cover page in output";

/// Output section help.
pub const SECTION_OUTPUT: &str = "\
OUTPUT SETTINGS:
  -o, --output <FMT>   Output file name format (%s = source name)
      --c              Color output
      --no-c           Grayscale output [default]
      --dpi <N>        Set both input and output DPI
      --odpi <N>       Set output DPI only
  -w, --width <W>      Output width with optional unit suffix (px/in/cm/s/t)
      --height <H>     Output height with optional unit suffix";

/// Behavior section help.
pub const SECTION_BEHAVIOR: &str = "\
BEHAVIOR:
  -x, --exit           Exit on complete (no interactive prompt)
      --no-x           Don't exit on complete
  -y, --yes            Assume yes to all prompts
      --no-y           Don't assume yes
  -v, --verbose        Verbose output (repeat: -v, -vv, -vvv, -vvvv)
      --ui-            Non-interactive mode (batch processing)
      --ui             Force interactive mode";

/// Meta/Debug section help.
pub const SECTION_META: &str = "\
META COMMANDS:
      --echo-cmd       Echo equivalent command line and exit
      --dry-run        Show conversion plan without processing
      --compat-report  Show compatibility report vs. C version";

/// Full parameter compatibility note — points users to compat-matrix.md.
pub const COMPAT_NOTE: &str = "\
NOTES:
    This is k2pdfopt-rs v0.1.0 (M7 milestone — release). Not all C v2.55
    options are supported yet. Use --compat-report to see the full comparison.
    Full docs: https://www.willus.com/k2pdfopt/help/";

/// Assemble all sections into the clap `after_long_help` string.
pub fn long_help() -> String {
    let mut s = String::new();
    for section in [
        USAGE_INTRO,
        SECTION_DEVICE,
        SECTION_LAYOUT,
        SECTION_PAGES,
        SECTION_OUTPUT,
        SECTION_BEHAVIOR,
        SECTION_META,
        COMPAT_NOTE,
    ] {
        s.push_str(section);
        s.push_str("\n\n");
    }
    s
}
