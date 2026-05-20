//! `k2ocr::lang` —— 多语言 OCR 语言包发现与 fallback 工具（Step 9.4）。
//!
//! 设计目标（与 Step 6.1/6.2/8.1/8.3 同源 "独立 struct 不反向依赖" 约定）：
//! - **纯函数 + 注入式可用列表**：本模块不直接调用任何 `OcrEngine`；
//!   `resolve` 把 `available_langs` 作为参数注入（调用端先 `engine.list_langs()` 再传进来）。
//!   单测因此无需启动 tesseract 子进程。
//! - **不反向依赖 `k2settings`**：与 `OcrPageInput` 同源原则。
//!   `k2settings::OcrSettings::dst_ocr_lang` → `LangSpec` 的映射放在 `k2pipeline::ocr_bridge`（Step 9.3 已落地）。
//! - **C 版同源**：复刻 `willuslib/ocrtess.c::ocrtess_init_check`（line 496-528）
//!   + `ocrtess_download_lang_message`（line 936-1015）的语言包发现与下载提示。
//!
//! 算法溯源：
//!
//! | Rust API | C 来源 |
//! |----------|--------|
//! | `LangSpec::parse` | `k2pdfopt.h:282 dst_ocr_lang` 字串 `+` 分割 |
//! | `resolve` 缺失语言 + fallback | `ocrtess.c:511-528` `TESSDATA_PREFIX` + `defurl` |
//! | `download_hint_default` | `ocrtess.c:159 defurl="raw.githubusercontent.com/tesseract-ocr/tessdata_%s/master"` |
//!
//! # 典型使用流程
//!
//! ```no_run
//! use k2ocr::lang::{LangSpec, ResolveOptions, resolve};
//! use k2ocr::{OcrEngine, TesseractCliEngine};
//!
//! let engine = TesseractCliEngine::new();
//! let available = engine.list_langs().unwrap_or_default();
//! let spec = LangSpec::parse("chi_sim+eng");
//! let res = resolve(&spec, &available, &ResolveOptions::default()).unwrap();
//! if res.fallback_used {
//!     eprintln!("⚠ 缺失语言 {:?}，已降级到 {}", res.missing, res.resolved_arg);
//! }
//! // 把 res.resolved_arg 传给 OcrPageInput::with_lang(...)
//! ```

use thiserror::Error;

/// 默认 fallback 语言（与 C 版 `ocrtess.c:366 langdef="eng"` 同源）。
pub const DEFAULT_FALLBACK_LANG: &str = "eng";

/// 默认语言包下载 URL 模板（与 C 版 `ocrtess.c:159 defurl` 同源）。
///
/// 调用 [`download_hint_default`] 时把 `%s` 替换为语言短名。
/// 与 C 版唯一区别：C 用 `tessdata_<lang>/master/<lang>.traineddata`，
/// Rust 同源 + 完整文件名（C 版 GitHub 仓库已 rename 为 `tessdata_fast/main`，
/// 详见 Open Question 9.4.B）。
pub const DEFAULT_TESSDATA_URL_TEMPLATE: &str =
    "https://github.com/tesseract-ocr/tessdata/blob/main/{lang}.traineddata";

/// 解析后的语言规范：把 `chi_sim+eng+osd` 拆为 `["chi_sim", "eng", "osd"]`。
///
/// 与 C 版 `dst_ocr_lang` 字串语义一致：
/// - 空字串 → `parts.is_empty() == true`（resolve 时落回 `DEFAULT_FALLBACK_LANG`）
/// - 单语言 `eng` → `parts == ["eng"]`
/// - 复合 `chi_sim+eng` → `parts == ["chi_sim", "eng"]`
///
/// 解析时会 `trim()` 每段并丢弃空段（如 `eng++osd` → `["eng", "osd"]`）。
/// 同名段会去重，保留首次出现的顺序（与 tesseract CLI 行为同源）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LangSpec {
    parts: Vec<String>,
}

impl LangSpec {
    /// 从用户字符串构造（如 `"chi_sim+eng"` / `"eng"` / `""`）。
    #[must_use]
    pub fn parse(input: &str) -> Self {
        let mut parts = Vec::new();
        for piece in input.split('+') {
            let trimmed = piece.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !parts.iter().any(|p: &String| p == trimmed) {
                parts.push(trimmed.to_string());
            }
        }
        Self { parts }
    }

    /// 从已分解的语言段构造（用于程序化场景）。会自动 trim + 去重 + 丢空段。
    #[must_use]
    pub fn from_parts<I, S>(parts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut acc = Vec::new();
        for p in parts {
            let trimmed = p.as_ref().trim();
            if trimmed.is_empty() {
                continue;
            }
            if !acc.iter().any(|x: &String| x == trimmed) {
                acc.push(trimmed.to_string());
            }
        }
        Self { parts: acc }
    }

    /// 真实可用的语言段切片。
    #[must_use]
    pub fn parts(&self) -> &[String] {
        &self.parts
    }

    /// 是否为空（调用端应据此走 fallback 路径）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// 序列化为 tesseract `-l` 参数（多语言用 `+` 连接）。
    ///
    /// 空 spec → 空字串（**不**自动塞 fallback；那是 [`resolve`] 的职责）。
    #[must_use]
    pub fn to_arg(&self) -> String {
        self.parts.join("+")
    }
}

/// [`resolve`] 的可调参数。
///
/// 默认值（与 ADR-017 MVP 路径同源）：
/// - `fallback_to_eng = true`：所有段都缺失时退到 `eng`
/// - `allow_partial = true`：部分段缺失时丢弃缺失段保留可用段
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolveOptions {
    /// 若所有请求的语言段都缺失，是否落回 [`DEFAULT_FALLBACK_LANG`]。
    pub fallback_to_eng: bool,
    /// 是否允许部分缺失（true=丢弃缺失段保留可用段；false=任何缺失即 [`LangResolveError::MissingLang`]）。
    pub allow_partial: bool,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            fallback_to_eng: true,
            allow_partial: true,
        }
    }
}

/// [`resolve`] 的结果（成功路径，描述实际可用的语言子集）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LangResolution {
    /// 调用端真正应传给 tesseract `-l` 的字串（保证非空，且每段都在 `available` 中）。
    pub resolved_arg: String,
    /// `resolved_arg` 按 `+` 切分的列表，便于程序化判断。
    pub resolved_parts: Vec<String>,
    /// 用户请求的全部语言段（按用户输入顺序）。
    pub requested_parts: Vec<String>,
    /// 用户请求但 `available` 中没有的段（保留请求顺序）。
    pub missing: Vec<String>,
    /// `resolved_arg` 是否经过了"全 missing → eng" 的兜底（true 时调用端建议打 warning）。
    pub fallback_used: bool,
}

impl LangResolution {
    /// 是否有任何缺失（不论是否被 fallback / 部分降级吸收）。
    #[must_use]
    pub fn has_missing(&self) -> bool {
        !self.missing.is_empty()
    }
}

/// `resolve` 失败枚举（`allow_partial=false` 时缺失即报错）。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LangResolveError {
    /// 严格模式下检测到缺失语言。
    #[error("缺失 OCR 语言包: 请求 [{requested}], 缺失 [{missing}], 可用 [{available}]")]
    MissingLang {
        requested: String,
        missing: String,
        available: String,
    },

    /// 所有请求段都缺失且 `fallback_to_eng=false`（或者 `eng` 自身也缺）。
    #[error("没有可用 OCR 语言: 请求 [{requested}], 可用 [{available}], fallback 'eng' 也缺失")]
    NoUsableLang {
        requested: String,
        available: String,
    },
}

/// 解析用户期望的语言包与系统实际可用的语言列表，返回最终送给引擎的 `-l` 参数。
///
/// 算法（按 C 版 `ocrtess.c:494-560` 同源精神改写为纯函数）：
///
/// 1. 把 `spec.parts` 按 `available` 划分为 `keep`（命中）和 `missing`（未命中）
/// 2. 严格模式 (`!opts.allow_partial`) 下任何缺失即返 [`LangResolveError::MissingLang`]
/// 3. 若 `keep.is_empty()`：
///     - `opts.fallback_to_eng && available.contains("eng")` → resolved = `["eng"]` + `fallback_used = true`
///     - 否则返 [`LangResolveError::NoUsableLang`]
/// 4. 否则 resolved = `keep` 原顺序
///
/// 边界：spec 为空（用户没填 lang）走与"全 missing"相同的路径——会落到 fallback 或 NoUsable。
pub fn resolve(
    spec: &LangSpec,
    available: &[String],
    opts: &ResolveOptions,
) -> Result<LangResolution, LangResolveError> {
    let requested_parts: Vec<String> = spec.parts().to_vec();

    let mut keep: Vec<String> = Vec::with_capacity(requested_parts.len());
    let mut missing: Vec<String> = Vec::new();
    for p in &requested_parts {
        if available.iter().any(|a| a == p) {
            keep.push(p.clone());
        } else {
            missing.push(p.clone());
        }
    }

    if !opts.allow_partial && !missing.is_empty() {
        return Err(LangResolveError::MissingLang {
            requested: requested_parts.join("+"),
            missing: missing.join(", "),
            available: format_available(available),
        });
    }

    let mut fallback_used = false;
    if keep.is_empty() {
        if opts.fallback_to_eng && available.iter().any(|a| a == DEFAULT_FALLBACK_LANG) {
            keep.push(DEFAULT_FALLBACK_LANG.to_string());
            fallback_used = true;
        } else {
            return Err(LangResolveError::NoUsableLang {
                requested: requested_parts.join("+"),
                available: format_available(available),
            });
        }
    }

    let resolved_arg = keep.join("+");
    Ok(LangResolution {
        resolved_arg,
        resolved_parts: keep,
        requested_parts,
        missing,
        fallback_used,
    })
}

/// 为单个语言短名生成默认的语言包下载提示 URL。
///
/// 与 C 版 `ocrtess.c:159 defurl` 同源（仓库地址同源；URL 路径 C 用 `tessdata_<lang>/master`，
/// 这里直接指向 `tessdata/blob/main/<lang>.traineddata` 减少层级 + 跟现行 GitHub 默认分支名）。
#[must_use]
pub fn download_hint_default(lang: &str) -> String {
    DEFAULT_TESSDATA_URL_TEMPLATE.replace("{lang}", lang.trim())
}

/// 把可用语言列表格式化为人类可读字串（用于错误消息）。
fn format_available(available: &[String]) -> String {
    if available.is_empty() {
        "<empty>".to_string()
    } else {
        available.join(", ")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn av(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    // === LangSpec::parse ===

    #[test]
    fn parse_empty_string_yields_empty_spec() {
        let s = LangSpec::parse("");
        assert!(s.is_empty());
        assert_eq!(s.parts(), &[] as &[String]);
        assert_eq!(s.to_arg(), "");
    }

    #[test]
    fn parse_single_lang_eng() {
        let s = LangSpec::parse("eng");
        assert_eq!(s.parts(), &["eng".to_string()]);
        assert_eq!(s.to_arg(), "eng");
        assert!(!s.is_empty());
    }

    #[test]
    fn parse_multi_lang_chi_sim_plus_eng() {
        let s = LangSpec::parse("chi_sim+eng");
        assert_eq!(s.parts(), &["chi_sim".to_string(), "eng".to_string()]);
        assert_eq!(s.to_arg(), "chi_sim+eng");
    }

    #[test]
    fn parse_trims_whitespace_around_parts() {
        let s = LangSpec::parse("  chi_sim  +  eng  ");
        assert_eq!(s.parts(), &["chi_sim".to_string(), "eng".to_string()]);
    }

    #[test]
    fn parse_drops_empty_segments() {
        let s = LangSpec::parse("eng++osd+");
        assert_eq!(s.parts(), &["eng".to_string(), "osd".to_string()]);
    }

    #[test]
    fn parse_deduplicates_preserving_first_order() {
        let s = LangSpec::parse("eng+chi_sim+eng");
        assert_eq!(s.parts(), &["eng".to_string(), "chi_sim".to_string()]);
        assert_eq!(s.to_arg(), "eng+chi_sim");
    }

    #[test]
    fn from_parts_builds_same_invariants_as_parse() {
        let s = LangSpec::from_parts(["chi_sim", "  ", "", "eng", "chi_sim"]);
        assert_eq!(s.parts(), &["chi_sim".to_string(), "eng".to_string()]);
    }

    // === resolve 成功路径 ===

    #[test]
    fn resolve_single_lang_present_passes_through() {
        let spec = LangSpec::parse("eng");
        let avail = av(&["eng", "osd"]);
        let res = resolve(&spec, &avail, &ResolveOptions::default()).unwrap();
        assert_eq!(res.resolved_arg, "eng");
        assert_eq!(res.resolved_parts, vec!["eng".to_string()]);
        assert_eq!(res.requested_parts, vec!["eng".to_string()]);
        assert!(res.missing.is_empty());
        assert!(!res.fallback_used);
        assert!(!res.has_missing());
    }

    #[test]
    fn resolve_multi_lang_all_present() {
        let spec = LangSpec::parse("chi_sim+eng");
        let avail = av(&["chi_sim", "eng", "osd"]);
        let res = resolve(&spec, &avail, &ResolveOptions::default()).unwrap();
        assert_eq!(res.resolved_arg, "chi_sim+eng");
        assert_eq!(res.resolved_parts, vec!["chi_sim", "eng"]);
        assert!(res.missing.is_empty());
        assert!(!res.fallback_used);
    }

    #[test]
    fn resolve_partial_drops_missing_lang() {
        let spec = LangSpec::parse("chi_sim+eng");
        let avail = av(&["eng", "osd"]);
        let res = resolve(&spec, &avail, &ResolveOptions::default()).unwrap();
        // chi_sim 缺失 → 丢弃，剩 eng 可用
        assert_eq!(res.resolved_arg, "eng");
        assert_eq!(res.resolved_parts, vec!["eng".to_string()]);
        assert_eq!(res.missing, vec!["chi_sim".to_string()]);
        assert!(res.has_missing());
        // 仍有命中段，不算 fallback
        assert!(!res.fallback_used);
    }

    #[test]
    fn resolve_all_missing_falls_back_to_eng_when_available() {
        let spec = LangSpec::parse("chi_sim+jpn");
        let avail = av(&["eng", "osd"]);
        let res = resolve(&spec, &avail, &ResolveOptions::default()).unwrap();
        assert_eq!(res.resolved_arg, "eng");
        assert_eq!(res.resolved_parts, vec!["eng".to_string()]);
        assert_eq!(res.missing, vec!["chi_sim".to_string(), "jpn".to_string()]);
        assert!(res.fallback_used);
    }

    #[test]
    fn resolve_empty_spec_falls_back_to_eng() {
        let spec = LangSpec::default();
        let avail = av(&["eng", "osd"]);
        let res = resolve(&spec, &avail, &ResolveOptions::default()).unwrap();
        assert_eq!(res.resolved_arg, "eng");
        assert!(res.requested_parts.is_empty());
        assert!(res.fallback_used);
    }

    // === resolve 失败路径 ===

    #[test]
    fn resolve_strict_mode_returns_error_on_any_missing() {
        let spec = LangSpec::parse("chi_sim+eng");
        let avail = av(&["eng", "osd"]);
        let opts = ResolveOptions {
            fallback_to_eng: true,
            allow_partial: false,
        };
        let r = resolve(&spec, &avail, &opts).unwrap_err();
        assert!(matches!(r, LangResolveError::MissingLang { .. }));
        // 错误消息含必要 context
        let msg = r.to_string();
        assert!(msg.contains("chi_sim"));
        assert!(msg.contains("eng"));
    }

    #[test]
    fn resolve_no_fallback_when_disabled() {
        let spec = LangSpec::parse("chi_sim+jpn");
        let avail = av(&["eng", "osd"]);
        let opts = ResolveOptions {
            fallback_to_eng: false,
            allow_partial: true,
        };
        let r = resolve(&spec, &avail, &opts).unwrap_err();
        assert!(matches!(r, LangResolveError::NoUsableLang { .. }));
    }

    #[test]
    fn resolve_no_usable_when_eng_also_missing() {
        let spec = LangSpec::parse("chi_sim+jpn");
        let avail = av(&["fra", "deu"]);
        // 即使 fallback 开启，eng 不在 available → NoUsableLang
        let r = resolve(&spec, &avail, &ResolveOptions::default()).unwrap_err();
        match r {
            LangResolveError::NoUsableLang {
                requested,
                available,
            } => {
                assert_eq!(requested, "chi_sim+jpn");
                assert!(available.contains("fra"));
                assert!(available.contains("deu"));
            }
            _ => panic!("应该是 NoUsableLang"),
        }
    }

    #[test]
    fn resolve_empty_spec_no_fallback_disabled_returns_no_usable() {
        let spec = LangSpec::default();
        let avail = av(&["eng"]);
        let opts = ResolveOptions {
            fallback_to_eng: false,
            allow_partial: true,
        };
        let r = resolve(&spec, &avail, &opts).unwrap_err();
        assert!(matches!(r, LangResolveError::NoUsableLang { .. }));
    }

    #[test]
    fn resolve_empty_available_returns_no_usable() {
        let spec = LangSpec::parse("eng");
        let avail: Vec<String> = Vec::new();
        let r = resolve(&spec, &avail, &ResolveOptions::default());
        assert!(r.is_err());
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("<empty>"));
    }

    // === download_hint_default ===

    #[test]
    fn download_hint_substitutes_lang_short_name() {
        let url = download_hint_default("chi_sim");
        assert!(url.contains("github.com/tesseract-ocr/tessdata"));
        assert!(url.contains("chi_sim.traineddata"));
    }

    #[test]
    fn download_hint_trims_input() {
        let a = download_hint_default("  eng  ");
        let b = download_hint_default("eng");
        assert_eq!(a, b);
    }

    #[test]
    fn download_hint_returns_predictable_template() {
        let url = download_hint_default("osd");
        assert_eq!(
            url,
            "https://github.com/tesseract-ocr/tessdata/blob/main/osd.traineddata"
        );
    }

    // === has_missing / fallback_used 路径完整覆盖 ===

    #[test]
    fn has_missing_true_when_partial_kept() {
        let spec = LangSpec::parse("chi_sim+eng");
        let avail = av(&["eng"]);
        let res = resolve(&spec, &avail, &ResolveOptions::default()).unwrap();
        assert!(res.has_missing());
        assert!(!res.fallback_used);
    }

    #[test]
    fn fallback_used_recorded_when_all_missing_dropped_to_eng() {
        let spec = LangSpec::parse("jpn");
        let avail = av(&["eng"]);
        let res = resolve(&spec, &avail, &ResolveOptions::default()).unwrap();
        assert!(res.fallback_used);
        assert!(res.has_missing());
        assert_eq!(res.resolved_arg, "eng");
    }

    #[test]
    fn requested_parts_preserves_input_order_and_dedup_state() {
        let spec = LangSpec::parse("chi_sim+eng+chi_sim");
        let avail = av(&["eng", "chi_sim"]);
        let res = resolve(&spec, &avail, &ResolveOptions::default()).unwrap();
        // requested 与去重后的 spec.parts 一致
        assert_eq!(res.requested_parts, vec!["chi_sim", "eng"]);
        assert_eq!(res.resolved_parts, vec!["chi_sim", "eng"]);
    }

    #[test]
    fn resolve_options_default_matches_documented_defaults() {
        let opts = ResolveOptions::default();
        assert!(opts.fallback_to_eng);
        assert!(opts.allow_partial);
    }
}
