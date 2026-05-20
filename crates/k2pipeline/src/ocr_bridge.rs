//! `ocr_bridge` - OcrSettings → OcrPageInput 映射 + master 坐标系平移 helper（Step 9.3）。
//!
//! 这一模块兑现 Open Question 9.1.G：把 [`k2settings::OcrSettings`] 桥接到
//! [`k2ocr::OcrPageInput`]，避免 k2ocr crate 反向依赖 k2settings（与 Step 6.x /
//! 8.x / 9.1 同源约定一致：OCR/layout/wrap 子系统的 settings 都用独立 input struct，
//! 调用方负责映射）。
//!
//! 同时提供 `recognize_for_master` 便利函数：跑 OCR → mapping::offset 一步完成。
//!
//! # C 来源对照
//!
//! - C `ocrtess_ocrwords_from_bmp8` 入参（lang/segmode/...）→ [`build_ocr_input`]
//! - C `ocrwords_offset(words, dw, masterinfo->rows + gap_start)`
//!   (`k2master.c:744`) → [`recognize_for_master`] 末段 `mapping::offset`
//!
//! 详见 `rust-rewrite-execution-plan.md` Step 9.3。

use k2ocr::{
    lang::{self, LangResolution, LangResolveError, LangSpec, ResolveOptions},
    mapping, OcrEngine, OcrError, OcrPageInput, PageSegmentationMode,
};
use k2settings::ocr::{OcrDetectionType, OcrMode, OcrSettings};
use k2types::{Bitmap, OcrWord};

/// 从 [`OcrSettings`] 构造 [`OcrPageInput`]，关联到指定的 source bitmap。
///
/// 仅当 `s.dst_ocr == OcrMode::Tesseract` 时返回 `Some`；
/// `Off` / `Mupdf`（mupdf native text extraction，M8+ 才支持）都返回 `None`。
///
/// # 字段映射
///
/// - `dst_ocr_lang`：空串 → 默认 `"eng"`（与 C `ocrtess_ocrwords_from_bmp8`
///   传 lang 空时回退一致）
/// - `ocr_detection_type`：
///   - [`OcrDetectionType::Word`] → [`PageSegmentationMode::SingleWord`]（PSM 8）
///   - [`OcrDetectionType::Line`] → [`PageSegmentationMode::SingleTextLine`]（PSM 7）
///   - [`OcrDetectionType::Paragraph`] → [`PageSegmentationMode::SingleColumnVarSize`]（PSM 4）
/// - `ocr_dpi`：>0 → `OcrPageInput::dpi`；否则用 `bitmap.dpi`
/// - `ocr_min_confidence`（Step 11.11 P1-4 / P1-5 新增）：直接透传到
///   [`OcrPageInput::min_confidence`]，由引擎内部 [`k2ocr::tsv_parser::parse_tsv`]
///   过滤低于阈值的 word。默认 `0.0` = 不过滤。
///
/// # 推迟项（Step 11.11 P1-3 ROI 切分）
///
/// `OcrSettings::ocr_max_columns` / `OcrSettings::ocr_max_height_inches` 描述的
/// ROI 切分（C `k2ocr.c` 的 `ocr_max_columns` 多列拆分 + `ocr_max_height_inches`
/// 高瘦区域纵向拆分）是**多 OcrPageInput** 生成逻辑，不在单次 `build_ocr_input`
/// 调用范围内。完整 ROI 切分流水线推迟 v0.3 P2-x ADR-019（Open Q 11.11.D）。
/// 本步仅消费 `ocr_min_confidence`，保证字段在 P1-5 "OcrPageInput ↔ OcrSettings
/// 完整字段映射" 字面要求范围内已落地。
///
/// # 返回
///
/// `Some(OcrPageInput)` 表示需要跑 OCR；`None` 表示本配置不需要跑（pipeline 应跳过）。
#[must_use]
pub fn build_ocr_input<'a>(s: &OcrSettings, bitmap: &'a Bitmap) -> Option<OcrPageInput<'a>> {
    if !matches!(s.dst_ocr, OcrMode::Tesseract) {
        return None;
    }
    let dpi = if s.ocr_dpi > 0 {
        s.ocr_dpi as f32
    } else {
        bitmap.dpi
    };
    let lang = if s.dst_ocr_lang.is_empty() {
        "eng".to_string()
    } else {
        s.dst_ocr_lang.clone()
    };
    let psm = match s.ocr_detection_type {
        OcrDetectionType::Word => PageSegmentationMode::SingleWord,
        OcrDetectionType::Line => PageSegmentationMode::SingleTextLine,
        OcrDetectionType::Paragraph => PageSegmentationMode::SingleColumnVarSize,
    };
    Some(
        OcrPageInput::new(bitmap, dpi)
            .with_lang(lang)
            .with_psm(psm)
            .with_min_confidence(s.ocr_min_confidence),
    )
}

/// 跑 OCR + 把 word 平移到 master canvas 坐标系（一步完成）。
///
/// 等价 C 调用链：
/// ```text
/// ocrtess_ocrwords_from_bmp8(words, src, x1, y1, x2, y2, lang, ...);
/// ocrwords_offset(words, dw, masterinfo->rows + gap_start);
/// ```
/// （`k2master.c:740-745`）
///
/// 返回的 Vec<OcrWord> 应直接送入 [`k2layout::master::OcrStaging::concatenate`]。
pub fn recognize_for_master(
    engine: &dyn OcrEngine,
    input: &OcrPageInput<'_>,
    dx: f64,
    dy: f64,
) -> Result<Vec<OcrWord>, OcrError> {
    let mut words = engine.recognize(input)?;
    if dx != 0.0 || dy != 0.0 {
        mapping::offset(&mut words, dx, dy);
    }
    Ok(words)
}

/// 解析 [`OcrSettings::dst_ocr_lang`] 在当前引擎下的实际可用语言（Step 9.4 多语言落地）。
///
/// 用 [`OcrEngine::list_langs`] 取系统已装语言包，调用 [`k2ocr::lang::resolve`] 做缺失检测
/// + fallback 推导。返回的 [`LangResolution`] 描述：
///
/// - `resolved_arg`：实际传给 tesseract `-l` 的字串（保证非空 / 全部命中 available）
/// - `missing`：用户请求但 available 没有的段
/// - `fallback_used`：是否走了 "全 missing → eng" 的兜底路径
///
/// 调用方应：
/// 1. 若 `fallback_used` 或 `has_missing()` → 打 warning（可附 [`download_hint`] 给用户）
/// 2. 用 `resolved_arg` 覆盖 [`OcrPageInput::lang`] 后再 `engine.recognize(...)`
///
/// `opts` 控制 strict / partial 模式；通常用 [`ResolveOptions::default`]（允许部分缺失，
/// 全部缺失时落 `eng`），与 ADR-017 MVP 行为一致。
///
/// # Errors
///
/// - [`ResolveLangError::EngineQuery`]：引擎 `list_langs` 失败（如 tesseract 不可用）
/// - [`ResolveLangError::Resolve`]：严格模式下检测到缺失，或 `eng` 也缺失没法 fallback
pub fn resolve_lang_via_engine(
    engine: &dyn OcrEngine,
    settings: &OcrSettings,
    opts: &ResolveOptions,
) -> Result<LangResolution, ResolveLangError> {
    let available = engine.list_langs().map_err(ResolveLangError::EngineQuery)?;
    let spec = LangSpec::parse(&settings.dst_ocr_lang);
    lang::resolve(&spec, &available, opts).map_err(ResolveLangError::Resolve)
}

/// [`resolve_lang_via_engine`] 的复合错误类型。
#[derive(Debug, thiserror::Error)]
pub enum ResolveLangError {
    /// 引擎 `list_langs` 失败。
    #[error("OCR 引擎查询可用语言失败: {0}")]
    EngineQuery(#[source] OcrError),

    /// 语言解析失败（缺失 + 严格模式 或 全 missing 且无 eng）。
    #[error(transparent)]
    Resolve(#[from] LangResolveError),
}

/// 单语言短名 → 默认下载 URL（[`k2ocr::lang::download_hint_default`] 的薄包装），
/// 便于 main.rs 在 warning 中给用户提示。
#[must_use]
pub fn download_hint(lang_short: &str) -> String {
    lang::download_hint_default(lang_short)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    #![allow(clippy::field_reassign_with_default)]

    use super::*;
    use k2settings::ocr::OcrSettings;
    use k2types::PixelFormat;

    fn make_bmp() -> Bitmap {
        Bitmap::new(100, 100, 300.0, PixelFormat::Gray8).unwrap()
    }

    #[test]
    fn build_ocr_input_off_returns_none() {
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Off;
        let bmp = make_bmp();
        assert!(build_ocr_input(&s, &bmp).is_none());
    }

    #[test]
    fn build_ocr_input_mupdf_returns_none() {
        // 默认即为 Mupdf
        let s = OcrSettings::default();
        let bmp = make_bmp();
        assert!(build_ocr_input(&s, &bmp).is_none());
    }

    #[test]
    fn build_ocr_input_tesseract_returns_some() {
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        let bmp = make_bmp();
        let inp = build_ocr_input(&s, &bmp).expect("Some");
        assert_eq!(inp.lang, "eng");
        assert_eq!(inp.psm, PageSegmentationMode::SingleTextLine); // Line 默认
        assert!((inp.dpi - 300.0_f32).abs() < 1e-6);
    }

    #[test]
    fn build_ocr_input_lang_override() {
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        s.dst_ocr_lang = "chi_sim+eng".to_string();
        let bmp = make_bmp();
        let inp = build_ocr_input(&s, &bmp).expect("Some");
        assert_eq!(inp.lang, "chi_sim+eng");
    }

    #[test]
    fn build_ocr_input_dpi_override_from_settings() {
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        s.ocr_dpi = 600;
        let bmp = make_bmp(); // bmp.dpi=300
        let inp = build_ocr_input(&s, &bmp).expect("Some");
        assert!((inp.dpi - 600.0_f32).abs() < 1e-6);
    }

    #[test]
    fn build_ocr_input_dpi_falls_back_to_bitmap_when_zero_or_negative() {
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        s.ocr_dpi = 0;
        let bmp = make_bmp();
        let inp = build_ocr_input(&s, &bmp).expect("Some");
        assert!((inp.dpi - bmp.dpi).abs() < 1e-6);
        s.ocr_dpi = -1;
        let inp = build_ocr_input(&s, &bmp).expect("Some");
        assert!((inp.dpi - bmp.dpi).abs() < 1e-6);
    }

    #[test]
    fn build_ocr_input_psm_word_line_paragraph() {
        let bmp = make_bmp();
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;

        s.ocr_detection_type = OcrDetectionType::Word;
        assert_eq!(
            build_ocr_input(&s, &bmp).unwrap().psm,
            PageSegmentationMode::SingleWord
        );

        s.ocr_detection_type = OcrDetectionType::Line;
        assert_eq!(
            build_ocr_input(&s, &bmp).unwrap().psm,
            PageSegmentationMode::SingleTextLine
        );

        s.ocr_detection_type = OcrDetectionType::Paragraph;
        assert_eq!(
            build_ocr_input(&s, &bmp).unwrap().psm,
            PageSegmentationMode::SingleColumnVarSize
        );
    }

    // ── Step 11.11 P1-4 / P1-5：ocr_min_confidence + ROI 字段映射 ──

    /// 默认 `ocr_min_confidence = 0.0` 透传到 [`OcrPageInput::min_confidence`]。
    /// 与 v0.1.0 行为兼容（0.0 = 不过滤）。
    #[test]
    fn build_ocr_input_propagates_default_min_confidence_zero() {
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        let bmp = make_bmp();
        let inp = build_ocr_input(&s, &bmp).expect("Some");
        assert!((inp.min_confidence - 0.0).abs() < f32::EPSILON);
    }

    /// 显式设置 `ocr_min_confidence = 0.5` 透传到 [`OcrPageInput::min_confidence`]
    /// （tsv_parser 内部 `(0.5 * 100).clamp(0, 100)` = 50% confidence 阈值）。
    /// 端到端过滤行为由 `k2ocr::tsv_parser::tests::filters_by_min_confidence` 兜底；
    /// 本测试只验证 OcrSettings → OcrPageInput 字段映射不丢值。
    #[test]
    fn build_ocr_input_propagates_explicit_min_confidence() {
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        s.ocr_min_confidence = 0.5;
        let bmp = make_bmp();
        let inp = build_ocr_input(&s, &bmp).expect("Some");
        assert!((inp.min_confidence - 0.5).abs() < f32::EPSILON);
    }

    /// `OcrSettings` 默认 ROI 字段 `ocr_max_columns = -1` 与
    /// `ocr_max_height_inches = 1.5`，与 C `k2settings.c:64/73` 字面默认一致。
    /// 本测试断言字段存在 + 默认值正确（ROI 切分完整流水线推迟 v0.3 P2-x，
    /// Open Q 11.11.D）。
    #[test]
    fn ocr_settings_roi_fields_have_c_default_values() {
        let s = OcrSettings::default();
        assert_eq!(s.ocr_max_columns, -1);
        assert!((s.ocr_max_height_inches - 1.5).abs() < 1e-9);
    }

    // ---- recognize_for_master ----

    /// 一个最小可控的 OcrEngine mock：返回固定 word 列表。
    struct FakeEngine {
        words: Vec<OcrWord>,
    }
    impl OcrEngine for FakeEngine {
        fn engine_name(&self) -> &'static str {
            "fake"
        }
        fn probe(&self) -> Result<k2ocr::OcrEngineInfo, OcrError> {
            Ok(k2ocr::OcrEngineInfo {
                engine_name: "fake".into(),
                version: "0.0".into(),
                data_path: None,
            })
        }
        fn list_langs(&self) -> Result<Vec<String>, OcrError> {
            Ok(vec!["eng".into()])
        }
        fn recognize(&self, _input: &OcrPageInput<'_>) -> Result<Vec<OcrWord>, OcrError> {
            Ok(self.words.clone())
        }
    }

    #[test]
    fn recognize_for_master_applies_offset() {
        let engine = FakeEngine {
            words: vec![
                OcrWord::new("a", 10.0, 20.0, 30.0, 12.0),
                OcrWord::new("b", 100.0, 50.0, 30.0, 12.0),
            ],
        };
        let bmp = make_bmp();
        let inp = OcrPageInput::new(&bmp, 300.0);
        let out = recognize_for_master(&engine, &inp, 0.0, 500.0).unwrap();
        assert_eq!(out.len(), 2);
        assert!((out[0].x - 10.0).abs() < 1e-9);
        assert!((out[0].y - 520.0).abs() < 1e-9); // 20 + 500
        assert!((out[1].x - 100.0).abs() < 1e-9);
        assert!((out[1].y - 550.0).abs() < 1e-9);
    }

    #[test]
    fn recognize_for_master_zero_offset_unchanged() {
        let engine = FakeEngine {
            words: vec![OcrWord::new("x", 5.0, 5.0, 5.0, 5.0)],
        };
        let bmp = make_bmp();
        let inp = OcrPageInput::new(&bmp, 300.0);
        let out = recognize_for_master(&engine, &inp, 0.0, 0.0).unwrap();
        assert!((out[0].x - 5.0).abs() < 1e-9);
        assert!((out[0].y - 5.0).abs() < 1e-9);
    }

    #[test]
    fn recognize_for_master_empty_words_pass_through() {
        let engine = FakeEngine { words: vec![] };
        let bmp = make_bmp();
        let inp = OcrPageInput::new(&bmp, 300.0);
        let out = recognize_for_master(&engine, &inp, 100.0, 200.0).unwrap();
        assert!(out.is_empty());
    }

    // ---- resolve_lang_via_engine + download_hint (Step 9.4) ----

    /// 可控 list_langs 的 mock 引擎，用于 lang 解析测试。
    struct LangsMock {
        langs: Vec<String>,
        fail_list: bool,
    }
    impl OcrEngine for LangsMock {
        fn engine_name(&self) -> &'static str {
            "langs-mock"
        }
        fn probe(&self) -> Result<k2ocr::OcrEngineInfo, OcrError> {
            Ok(k2ocr::OcrEngineInfo {
                engine_name: "langs-mock".into(),
                version: "0.0".into(),
                data_path: None,
            })
        }
        fn list_langs(&self) -> Result<Vec<String>, OcrError> {
            if self.fail_list {
                Err(OcrError::OutputParse("mock fail".into()))
            } else {
                Ok(self.langs.clone())
            }
        }
        fn recognize(&self, _input: &OcrPageInput<'_>) -> Result<Vec<OcrWord>, OcrError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn resolve_lang_all_present_no_fallback() {
        let engine = LangsMock {
            langs: vec!["eng".into(), "chi_sim".into(), "osd".into()],
            fail_list: false,
        };
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        s.dst_ocr_lang = "chi_sim+eng".into();
        let res =
            resolve_lang_via_engine(&engine, &s, &ResolveOptions::default()).expect("resolve ok");
        assert_eq!(res.resolved_arg, "chi_sim+eng");
        assert!(res.missing.is_empty());
        assert!(!res.fallback_used);
    }

    #[test]
    fn resolve_lang_partial_drops_missing_keeps_eng() {
        let engine = LangsMock {
            langs: vec!["eng".into(), "osd".into()],
            fail_list: false,
        };
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        s.dst_ocr_lang = "chi_sim+eng".into();
        let res =
            resolve_lang_via_engine(&engine, &s, &ResolveOptions::default()).expect("resolve ok");
        assert_eq!(res.resolved_arg, "eng");
        assert_eq!(res.missing, vec!["chi_sim".to_string()]);
        assert!(!res.fallback_used);
        assert!(res.has_missing());
    }

    #[test]
    fn resolve_lang_all_missing_falls_back_to_eng() {
        let engine = LangsMock {
            langs: vec!["eng".into(), "osd".into()],
            fail_list: false,
        };
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        s.dst_ocr_lang = "chi_sim+jpn".into();
        let res =
            resolve_lang_via_engine(&engine, &s, &ResolveOptions::default()).expect("resolve ok");
        assert_eq!(res.resolved_arg, "eng");
        assert!(res.fallback_used);
        assert_eq!(res.missing, vec!["chi_sim".to_string(), "jpn".to_string()]);
    }

    #[test]
    fn resolve_lang_empty_settings_lang_falls_back_to_eng() {
        let engine = LangsMock {
            langs: vec!["eng".into()],
            fail_list: false,
        };
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        s.dst_ocr_lang = String::new();
        let res =
            resolve_lang_via_engine(&engine, &s, &ResolveOptions::default()).expect("resolve ok");
        assert_eq!(res.resolved_arg, "eng");
        assert!(res.fallback_used);
        assert!(res.requested_parts.is_empty());
    }

    #[test]
    fn resolve_lang_strict_returns_missing_error() {
        let engine = LangsMock {
            langs: vec!["eng".into()],
            fail_list: false,
        };
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        s.dst_ocr_lang = "chi_sim+eng".into();
        let opts = ResolveOptions {
            fallback_to_eng: true,
            allow_partial: false,
        };
        let r = resolve_lang_via_engine(&engine, &s, &opts);
        let err = r.unwrap_err();
        match err {
            ResolveLangError::Resolve(LangResolveError::MissingLang { .. }) => {}
            other => panic!("应为 Resolve(MissingLang), 实际 {other:?}"),
        }
    }

    #[test]
    fn resolve_lang_engine_query_failure_propagates() {
        let engine = LangsMock {
            langs: vec![],
            fail_list: true,
        };
        let mut s = OcrSettings::default();
        s.dst_ocr = OcrMode::Tesseract;
        s.dst_ocr_lang = "eng".into();
        let r = resolve_lang_via_engine(&engine, &s, &ResolveOptions::default());
        let err = r.unwrap_err();
        assert!(matches!(err, ResolveLangError::EngineQuery(_)));
    }

    #[test]
    fn download_hint_re_exports_predictable_url() {
        let url = download_hint("chi_sim");
        assert!(url.contains("chi_sim.traineddata"));
        assert!(url.contains("github.com"));
    }
}
