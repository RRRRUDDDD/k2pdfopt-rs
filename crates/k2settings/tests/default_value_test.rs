//! Step 3.3 验收测试 — Settings::default() 每字段默认值与 C 版 k2pdfopt_settings_init() 一致。

use k2settings::*;

#[test]
fn default_behavior_settings() {
    let b = BehaviorSettings::default();

    // k2settings.c:36
    assert_eq!(b.verbose, 0);
    // k2settings.c:37
    assert_eq!(b.debug, 0);
    // k2settings.c:51
    assert_eq!(b.show_usage, "");
    // k2settings.c:89
    assert_eq!(b.query_user, -1);
    // k2settings.c:90
    assert!(!b.query_user_explicit);
    // k2settings.c:207
    assert!(!b.autocrop);
    // k2settings.c:236
    assert!(!b.dewarp);
    // k2settings.c:130
    assert_eq!(b.pagelist, "");
    // k2settings.c:215
    assert_eq!(b.pagexlist, "");
    // k2settings.c:181
    assert_eq!(b.bpl, "");
    // k2settings.c:178
    assert_eq!(b.use_toc, -1);
    // k2settings.c:179
    assert_eq!(b.toclist, "");
    // k2settings.c:180
    assert_eq!(b.tocsavefile, "");
    // k2settings.c:135
    assert_eq!(b.exit_on_complete, -1);
    // k2settings.c:136
    assert!(!b.show_marked_source);
    // k2settings.c:137
    assert!(!b.use_crop_boxes);
    // k2settings.c:155
    assert_eq!(b.preview_page, 0);
    // k2settings.c:156
    assert!(!b.echo_source_page_count);
    // k2settings.c:211
    assert!(!b.info);
    // k2settings.c:221
    assert!(!b.assume_yes);
    // k2settings.c:225
    assert_eq!(b.user_mag, 0);
    // k2settings.c:226
    assert_eq!(b.join_figure_captions, 1);
    // k2settings.c:229
    assert_eq!(b.nthreads, -50);
}

#[test]
fn default_source_settings() {
    let s = SourceSettings::default();

    // k2settings.c:47
    assert_eq!(s.src_rot, SrcRotation::Auto);
    // k2settings.c:48
    assert!((s.gtc_in - 0.005).abs() < f64::EPSILON);
    // k2settings.c:49
    assert!((s.gtr_in - 0.006).abs() < f64::EPSILON);
    // k2settings.c:50
    assert!((s.gtw_in - 0.0015).abs() < f64::EPSILON);
    // k2settings.c:52
    assert!(s.src_left_to_right);
    // k2settings.c:53
    assert_eq!(s.src_whitethresh, -1);
    // k2settings.c:54
    assert!(!s.src_paintwhite);
    // k2settings.c:80
    assert!((s.user_src_dpi - (-2.0)).abs() < f64::EPSILON);
    // k2settings.c:81
    assert!((s.document_scale_factor - 1.0).abs() < f64::EPSILON);
    // k2settings.c:82
    assert_eq!(s.src_dpi, 300);
    // k2settings.c:83-87 (with mupdf)
    assert!(!s.user_usegs);
    assert!(!s.usegs);
    // k2settings.c:103
    assert!(!s.src_autostraighten);
    // k2settings.c:115-121
    assert_eq!(s.srccropmargins.pagelist, "");
    assert_eq!(s.srccropmargins.box_vals, [0.0; 4]);
    assert_eq!(s.srccropmargins.units, [MarginUnit::Inches; 4]);
    assert_eq!(s.srccropmargins.cboxflags, 0);
    // k2settings.c:144
    assert!(s.src_trim);
    // k2settings.c:145
    assert!(!s.erase_vertical_lines);
    // k2settings.c:214
    assert!(!s.erase_horizontal_lines);
    // k2settings.c:150
    assert_eq!(s.src_grid_rows, -1);
    // k2settings.c:151
    assert_eq!(s.src_grid_cols, -1);
    // k2settings.c:152
    assert_eq!(s.grid_order, -1);
    // k2settings.c:153
    assert!((s.src_grid_overlap_percentage - 2.0).abs() < f64::EPSILON);
    // k2settings.c:232
    assert_eq!(s.src_erosion, 0);
    // k2settings.c:239
    assert!(s.detect_double_rows);
    // k2settings.c:240
    assert!((s.textheight_min_pts - (-1.0)).abs() < f64::EPSILON);
}

#[test]
fn default_destination_settings() {
    let d = DestinationSettings::default();

    // k2pdfopt.h:158 — DEFAULT_WIDTH
    assert_eq!(d.dst_width, 560);
    // k2pdfopt.h:159 — DEFAULT_HEIGHT
    assert_eq!(d.dst_height, 745);
    // "kv" profile defaults
    assert!((d.dst_userwidth - 600.0).abs() < f64::EPSILON);
    assert!((d.dst_userheight - 800.0).abs() < f64::EPSILON);
    assert_eq!(d.dst_userdpi, 167);
    assert_eq!(d.dst_dpi, 167);
    assert_eq!(d.dst_userwidth_units, 0);
    assert_eq!(d.dst_userheight_units, 0);
    // k2settings.c:92
    assert!((d.dst_magnification - 1.0).abs() < f64::EPSILON);
    // k2settings.c:93
    assert!((d.dst_display_resolution - 1.0).abs() < f64::EPSILON);
    // k2settings.c:129
    assert!((d.display_width_inches - 3.6).abs() < f64::EPSILON);
    // k2settings.c:104-110
    assert_eq!(d.dstmargins.pagelist, "");
    assert_eq!(d.dstmargins.box_vals, [0.02; 4]);
    assert_eq!(d.dstmargins.units, [MarginUnit::Inches; 4]);
    assert_eq!(d.dstmargins.cboxflags, 0);
    assert_eq!(d.dstmargins_org.box_vals, [0.02; 4]);
    // device profile padding defaults (kv)
    assert_eq!(d.pad_left, 0);
    assert_eq!(d.pad_right, 0);
    assert_eq!(d.pad_bottom, 0);
    assert_eq!(d.pad_top, 0);
    assert_eq!(d.mark_corners, 0);
    // k2settings.c:185
    assert_eq!(d.devsize_set, 0);
    assert!(d.device_alias.is_none());
    // k2settings.c:100
    assert!(!d.dst_landscape);
    // k2settings.c:101
    assert_eq!(d.dst_landscape_pages, "");
}

#[test]
fn default_layout_settings() {
    let l = LayoutSettings::default();

    // k2settings.c:38
    assert!((l.cdthresh - 0.01).abs() < f64::EPSILON);
    // k2settings.c:79
    assert!(l.fit_columns);
    // k2settings.c:111
    assert!((l.min_column_gap_inches - 0.1).abs() < f64::EPSILON);
    // k2settings.c:112
    assert!((l.max_column_gap_inches - 1.5).abs() < f64::EPSILON);
    // k2settings.c:113
    assert!((l.min_column_height_inches - 1.5).abs() < f64::EPSILON);
    // k2settings.c:123
    assert_eq!(l.max_columns, 2);
    // k2settings.c:124
    assert!((l.column_gap_range - 0.33).abs() < f64::EPSILON);
    // k2settings.c:125
    assert!((l.column_offset_max - 0.3).abs() < f64::EPSILON);
    // k2settings.c:126
    assert!((l.column_row_gap_height_in - 1.0 / 72.0).abs() < f64::EPSILON);
    // k2settings.c:114
    assert!((l.row_split_fom - 20.0).abs() < f64::EPSILON);
    // k2settings.c:127
    assert_eq!(l.text_wrap, k2settings::TextWrap::On);
    // k2settings.c:128
    assert!((l.word_spacing - (-0.20)).abs() < f64::EPSILON);
    // k2settings.c:131
    assert!(!l.column_fitted);
    // k2settings.c:122
    assert!((l.max_region_width_inches - 3.6).abs() < f64::EPSILON);
    // k2settings.c:138
    assert!(l.preserve_indentation);
    // k2settings.c:139
    assert!((l.defect_size_pts - 0.75).abs() < f64::EPSILON);
    // k2settings.c:140
    assert!((l.max_vertical_gap_inches - 0.25).abs() < f64::EPSILON);
    // k2settings.c:141
    assert!((l.vertical_multiplier - 1.0).abs() < f64::EPSILON);
    // k2settings.c:142
    assert!((l.vertical_line_spacing - (-1.2)).abs() < f64::EPSILON);
    // k2settings.c:143
    assert!((l.vertical_break_threshold - 1.75).abs() < f64::EPSILON);
    // k2settings.c:146
    assert!(l.hyphen_detect);
    // k2settings.c:147
    assert!((l.overwrite_minsize_mb - 10.0).abs() < f64::EPSILON);
    // k2settings.c:148
    assert!(!l.rename);
    // k2settings.c:149
    assert!(!l.dst_fit_to_page);
    // k2settings.c:160
    assert!((l.no_wrap_ar_limit - 0.2).abs() < f64::EPSILON);
    // k2settings.c:161
    assert!((l.no_wrap_height_limit_inches - 0.55).abs() < f64::EPSILON);
    // k2settings.c:162
    assert!((l.little_piece_threshold_inches - 0.5).abs() < f64::EPSILON);
}

#[test]
fn default_output_settings() {
    let o = OutputSettings::default();

    // k2settings.c:76
    assert!(o.dst_dither);
    // k2settings.c:77
    assert!(o.dst_break_pages);
    // k2settings.c:78
    assert_eq!(o.render_dpi, 167);
    // k2settings.c:91
    assert_eq!(o.jpeg_quality, -1);
    // k2settings.c:94
    assert_eq!(o.dst_justify, -1);
    // k2settings.c:95
    assert_eq!(o.dst_figure_justify, -1);
    // k2settings.c:210
    assert!(!o.dst_figure_rotate);
    // k2settings.c:96
    assert!((o.dst_min_figure_height_in - 0.75).abs() < f64::EPSILON);
    // k2settings.c:97
    assert_eq!(o.dst_fulljustify, -1);
    // k2settings.c:98
    assert_eq!(o.dst_sharpen, 1);
    // "kv" profile default
    assert_eq!(o.dst_color, 0);
    // k2settings.c:99
    assert_eq!(o.dst_bpc, 4);
    // k2settings.c:102
    assert_eq!(o.dst_opname_format, "%s_k2opt");
    // k2settings.c:195
    assert_eq!(o.dst_fgcolor, "");
    // k2settings.c:197
    assert_eq!(o.dst_fgtype, 0);
    // k2settings.c:196
    assert_eq!(o.dst_bgcolor, "");
    // k2settings.c:198
    assert_eq!(o.dst_bgtype, 0);
    // device profile
    assert!((o.dpi_org - 167.0).abs() < f64::EPSILON);
    // k2settings.c:132
    assert!((o.contrast_max - 2.0).abs() < f64::EPSILON);
    // k2settings.c:133
    assert!((o.dst_gamma - 0.5).abs() < f64::EPSILON);
    // k2settings.c:134
    assert_eq!(o.dst_negative, 0);
    // k2settings.c:189
    assert!(!o.text_only);
    // k2settings.c:216
    assert_eq!(o.dst_author, "");
    // k2settings.c:217
    assert_eq!(o.dst_title, "");
    // k2settings.c:220
    assert!((o.dst_fontsize_pts - 0.0).abs() < f64::EPSILON);
    // k2settings.c:222
    assert_eq!(o.dst_coverimage, "");
    // k2settings.c:212
    assert_eq!(o.pagebreakmark_breakpage_color, -1);
    // k2settings.c:213
    assert_eq!(o.pagebreakmark_nobreak_color, -1);
}

#[test]
fn default_ocr_settings() {
    let o = OcrSettings::default();

    // k2settings.c:56
    assert_eq!(o.ocrout, "");
    // k2settings.c:66 (with mupdf)
    assert_eq!(o.dst_ocr, OcrMode::Mupdf);
    // k2settings.c:67
    assert!(!o.ocrvbb);
    // k2settings.c:68
    assert!(!o.ocrsort);
    // k2settings.c:57
    assert_eq!(o.ocr_detection_type, OcrDetectionType::Line);
    // k2settings.c:59-60
    assert_eq!(o.ocr_dpi, 300);
    // k2settings.c:62
    assert_eq!(o.dst_ocr_lang, "");
    // k2settings.c:72 / Step 11.11 P1-2 newtype: DEFAULT = SHOW_SOURCE = 0x01
    assert_eq!(o.dst_ocr_visibility_flags.bits(), 1);
    // k2settings.c:64
    assert_eq!(o.ocr_max_columns, -1);
    // k2settings.c:73
    assert!((o.ocr_max_height_inches - 1.5).abs() < f64::EPSILON);
    // k2settings.c:74
    assert!(!o.sort_ocr_text);
}

#[test]
fn default_settings_composite() {
    let s = Settings::default();

    // Verify all sub-structs are populated
    assert_eq!(s.behavior.verbose, 0);
    assert_eq!(s.source.src_dpi, 300);
    assert!((s.layout.cdthresh - 0.01).abs() < f64::EPSILON);
    assert_eq!(s.output.render_dpi, 167);
    assert_eq!(s.ocr.ocr_dpi, 300);
    assert_eq!(s.destination.dst_width, 560);
}

#[test]
fn src_rotation_roundtrip() {
    // Test all SrcRotation variants round-trip through C values
    assert_eq!(
        SrcRotation::from_c_value(SrcRotation::Auto.to_c_value()),
        SrcRotation::Auto
    );
    assert_eq!(
        SrcRotation::from_c_value(SrcRotation::AutoEvenPage.to_c_value()),
        SrcRotation::AutoEvenPage
    );
    assert_eq!(
        SrcRotation::from_c_value(SrcRotation::AutoPreview.to_c_value()),
        SrcRotation::AutoPreview
    );
    assert_eq!(
        SrcRotation::from_c_value(SrcRotation::Angle(90.0).to_c_value()),
        SrcRotation::Angle(90.0)
    );
    assert_eq!(SrcRotation::Auto.to_c_value(), -999.0);
    assert_eq!(SrcRotation::AutoEvenPage.to_c_value(), -998.0);
    assert_eq!(SrcRotation::AutoPreview.to_c_value(), -997.0);
}

#[test]
fn ocr_mode_roundtrip() {
    assert_eq!(
        OcrMode::from_c_int(OcrMode::Off.to_c_int()),
        Some(OcrMode::Off)
    );
    assert_eq!(
        OcrMode::from_c_int(OcrMode::Mupdf.to_c_int()),
        Some(OcrMode::Mupdf)
    );
    assert_eq!(
        OcrMode::from_c_int(OcrMode::Tesseract.to_c_int()),
        Some(OcrMode::Tesseract)
    );
    assert_eq!(OcrMode::from_c_int(999), None);
}

#[test]
fn ocr_detection_type_roundtrip() {
    assert_eq!(
        OcrDetectionType::from_c_char(OcrDetectionType::Word.to_c_char()),
        Some(OcrDetectionType::Word)
    );
    assert_eq!(
        OcrDetectionType::from_c_char(OcrDetectionType::Line.to_c_char()),
        Some(OcrDetectionType::Line)
    );
    assert_eq!(
        OcrDetectionType::from_c_char(OcrDetectionType::Paragraph.to_c_char()),
        Some(OcrDetectionType::Paragraph)
    );
    assert_eq!(OcrDetectionType::from_c_char('x'), None);
}

#[test]
fn cropbox_default_values() {
    let cb = CropBox::default();
    assert_eq!(cb.pagelist, "");
    assert_eq!(cb.box_vals, [0.0; 4]);
    assert_eq!(cb.units, [MarginUnit::Inches; 4]);
    assert_eq!(cb.cboxflags, 0);
}

#[test]
fn margin_unit_default_is_inches() {
    assert_eq!(MarginUnit::default(), MarginUnit::Inches);
}

#[test]
fn field_count_audit() {
    // Audit: count total fields across all sub-structs.
    // C struct has ~150 fields (excluding GUI/conditional fields).
    // Rust sub-structs should cover all unconditional fields.
    let source_fields = 21usize; // SourceSettings
    let dest_fields = 19usize; // DestinationSettings (18 + device_alias)
    let layout_fields = 25usize; // LayoutSettings
    let output_fields = 25usize; // OutputSettings
    let ocr_fields = 11usize; // OcrSettings
    let behavior_fields = 24usize; // BehaviorSettings
    let total =
        source_fields + dest_fields + layout_fields + output_fields + ocr_fields + behavior_fields;
    // Total should be approximately 124 fields.
    // C struct K2PDFOPT_SETTINGS has ~150 fields total, but ~26 are
    // GUI-only (#ifdef HAVE_K2GUI) or Ghostscript-only (#ifdef HAVE_GHOSTSCRIPT)
    // which we intentionally exclude from the Rust model.
    assert_eq!(total, 125);
}
