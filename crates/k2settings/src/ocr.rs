//! OCR settings — fields from K2PDFOPT_SETTINGS related to OCR processing.
//!
//! Source C: `k2pdfopt.h:237-442` (K2PDFOPT_SETTINGS struct)
//! Default init: `k2settings.c:31-241` (k2pdfopt_settings_init)
//!
//! Note: C code gates many OCR fields behind `#ifdef HAVE_OCR_LIB` / `#ifdef HAVE_TESSERACT_LIB`.
//! Rust version always includes these fields (Rust build always has OCR support).

/// OCR detection type — maps to ocr_detection_type char in C.
/// 'w' = word, 'l' = line (default), 'p' = paragraph.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum OcrDetectionType {
    /// Word-level detection ('w')
    Word,
    /// Line-level detection ('l') — default
    #[default]
    Line,
    /// Paragraph-level detection ('p')
    Paragraph,
}

impl OcrDetectionType {
    pub fn to_c_char(&self) -> char {
        match self {
            OcrDetectionType::Word => 'w',
            OcrDetectionType::Line => 'l',
            OcrDetectionType::Paragraph => 'p',
        }
    }

    pub fn from_c_char(c: char) -> Option<Self> {
        match c {
            'w' => Some(OcrDetectionType::Word),
            'l' => Some(OcrDetectionType::Line),
            'p' => Some(OcrDetectionType::Paragraph),
            _ => None,
        }
    }
}

/// OCR 严格模式 —— 控制缺失语言包时的行为（Step 11.9 P0-6）。
///
/// 与 `k2ocr::lang::ResolveOptions` 解耦：本 enum 仅暴露 Rust 层语义，
/// 调用方（`app/k2pdfopt/src/main.rs::resolve_ocr_lang_or_warn`）通过
/// [`OcrStrictMode::to_resolve_bools`] 拿到 `(fallback_to_eng, allow_partial)`
/// 后构造 `ResolveOptions`。这避免 k2settings 反向依赖 k2ocr（与 Step 6.x /
/// 8.x / 9.x "settings 子系统不引 engine" 同源约定）。
///
/// 与 C 版关系：C `k2pdfopt` 无对应 flag —— 这是 Rust 版的安全增强项
/// （v0.1.0 时 OCR 缺语言必定 fallback eng 且打 warning，用户无控制权）。
///
/// 默认 [`Fallback`](OcrStrictMode::Fallback) 与 v0.1.0 行为一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OcrStrictMode {
    /// 严格模式：任一请求语言缺失即报错退出。
    ///
    /// 映射 `ResolveOptions { fallback_to_eng: false, allow_partial: false }`。
    /// 适用：双盲转换流水线，丢字宁可失败不要静默 fallback。
    Strict,

    /// 部分允许：丢弃缺失段保留命中段，全部缺失才报错。
    ///
    /// 映射 `ResolveOptions { fallback_to_eng: false, allow_partial: true }`。
    /// 适用：明确知道部分语言可能缺失，希望"能 OCR 多少算多少"。
    Partial,

    /// Fallback 模式（默认）：丢弃缺失段，全部缺失自动落回 `eng`。
    ///
    /// 映射 `ResolveOptions::default()`（`fallback_to_eng=true, allow_partial=true`）。
    /// 与 v0.1.0 行为完全一致，是默认值。
    #[default]
    Fallback,
}

/// OCR 可见性 bit mask —— C `dst_ocr_visibility_flags`（Step 11.11 P1-2）。
///
/// 移植自 `k2pdfoptlib/k2ocr.c::dst_ocr_visibility_flags` 与 `k2pdfopt.h:291`，
/// 5 bit 控制最终 PDF 的 OCR 文字层渲染方式：
///
/// | bit | 常量 | 含义 |
/// |-----|------|------|
/// | 0x01 | [`SHOW_SOURCE`](Self::SHOW_SOURCE) | 在 PDF 中保留 source bitmap（vs 白底）|
/// | 0x02 | [`SHOW_OCR_TEXT`](Self::SHOW_OCR_TEXT) | 注入不可见 OCR 文字层（PDF Tr 3）|
/// | 0x04 | [`SHOW_BOXES`](Self::SHOW_BOXES) | 在 OCR word 外画矩形边框（调试可视化）|
/// | 0x08 | [`USE_SPACES`](Self::USE_SPACES) | 用空格分隔 word（vs 行内紧排）|
/// | 0x10 | [`OPTIMIZED`](Self::OPTIMIZED) | 优化的空格策略（仅 C 引擎实现）|
///
/// # 默认值
///
/// [`Self::DEFAULT`] = `Self::SHOW_SOURCE`（C `k2settings.c:72` 字面 = 1）。
/// 偏离 post-release-fix-plan.md §6.2 字面 `DEFAULT = Self(2)` —— 后者实际是
/// "仅 OCR 文字层，无 source bitmap" 的特殊场景，与 C 默认行为不同。本步保 C
/// 默认 = SHOW_SOURCE，与 v0.1.0 行为兼容。记 Open Question 11.11.A。
///
/// # 与 PDF Tr 模式映射（[`crate::ocr::OcrVisibility::pdf_text_render_mode`]）
///
/// 不含 [`SHOW_OCR_TEXT`](Self::SHOW_OCR_TEXT) → `None`（跳过整个 BT...ET）；
/// 含 [`SHOW_OCR_TEXT`](Self::SHOW_OCR_TEXT) → `Some(3)`（invisible，可被复制粘贴 + 搜索）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrVisibility(pub u8);

impl OcrVisibility {
    /// bit 0x01：保留 source bitmap（默认 on）。
    pub const SHOW_SOURCE: Self = Self(1);
    /// bit 0x02：注入不可见 OCR 文字层（PDF Tr 3）。
    pub const SHOW_OCR_TEXT: Self = Self(2);
    /// bit 0x04：绘制 OCR word 边框矩形（调试）。
    pub const SHOW_BOXES: Self = Self(4);
    /// bit 0x08：用空格分隔 word（避免 word 紧贴）。
    pub const USE_SPACES: Self = Self(8);
    /// bit 0x10：优化的空格策略（仅 C 引擎实现）。
    pub const OPTIMIZED: Self = Self(16);

    /// 默认值 = [`SHOW_SOURCE`](Self::SHOW_SOURCE) = 0x01。
    /// 与 C `k2settings.c:72` 字面一致；偏离 plan §6.2 字面 `DEFAULT = Self(2)`
    /// （Open Q 11.11.A 推迟评估）。
    pub const DEFAULT: Self = Self::SHOW_SOURCE;

    /// 全部合法 bit 的并集（用于 clap value_parser 范围校验 0..=31）。
    pub const ALL_BITS_MAX: u8 = 0x1F;

    /// 从原始 u8 bit mask 构造。
    /// 不校验是否在 [`ALL_BITS_MAX`](Self::ALL_BITS_MAX) 范围内 —— 调用方负责（如 clap value_parser）。
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// 返回原始 u8 bit mask。
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// 是否包含指定 flag（位与判断）。
    ///
    /// ```ignore
    /// use k2settings::ocr::OcrVisibility;
    /// let v = OcrVisibility::from_bits(0x07); // SHOW_SOURCE | SHOW_OCR_TEXT | SHOW_BOXES
    /// assert!(v.contains(OcrVisibility::SHOW_SOURCE));
    /// assert!(v.contains(OcrVisibility::SHOW_OCR_TEXT));
    /// assert!(v.contains(OcrVisibility::SHOW_BOXES));
    /// assert!(!v.contains(OcrVisibility::USE_SPACES));
    /// ```
    #[must_use]
    pub const fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) == flag.0
    }

    /// 计算应该写入 PDF content stream 的 Tr 模式。
    ///
    /// | visibility | 返回 | 含义 |
    /// |-----|------|------|
    /// | 不含 SHOW_OCR_TEXT | `None` | 跳过整个 OCR 文字层（不写 BT...ET）|
    /// | 含 SHOW_OCR_TEXT | `Some(3)` | invisible（Tr 3，可复制粘贴 + 搜索）|
    ///
    /// 注：bit 1/4/8/16 不影响 Tr 模式选择，仅控制 source bitmap 是否保留、
    /// 是否额外画 box 等。
    #[must_use]
    pub const fn pdf_text_render_mode(self) -> Option<i64> {
        if self.contains(Self::SHOW_OCR_TEXT) {
            Some(3)
        } else {
            None
        }
    }
}

impl Default for OcrVisibility {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl OcrStrictMode {
    /// 把 enum 映射为 `(fallback_to_eng, allow_partial)` 两个 bool。
    ///
    /// 调用方用这两个 bool 构造 `k2ocr::lang::ResolveOptions`，
    /// 避免 k2settings 反向依赖 k2ocr。
    ///
    /// | Mode      | fallback_to_eng | allow_partial |
    /// |-----------|-----------------|---------------|
    /// | Strict    | false           | false         |
    /// | Partial   | false           | true          |
    /// | Fallback  | true            | true          |
    #[must_use]
    pub fn to_resolve_bools(self) -> (bool, bool) {
        match self {
            OcrStrictMode::Strict => (false, false),
            OcrStrictMode::Partial => (false, true),
            OcrStrictMode::Fallback => (true, true),
        }
    }

    /// 把 CLI flag `--ocr-mode <MODE>` 的字符串解析为 enum。
    ///
    /// 接受：`"strict"` / `"partial"` / `"fallback"`（不区分大小写）。
    /// 其他字串返 `None`（clap value_parser 通常已在 CLI 层兜底拦截）。
    #[must_use]
    pub fn from_arg(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "strict" => Some(OcrStrictMode::Strict),
            "partial" => Some(OcrStrictMode::Partial),
            "fallback" => Some(OcrStrictMode::Fallback),
            _ => None,
        }
    }

    /// 反序列化为 `--ocr-mode <MODE>` CLI 字串（与 [`from_arg`](Self::from_arg) 对偶）。
    ///
    /// Default 模式 [`Fallback`](Self::Fallback) 仍返 `"fallback"`；
    /// serialize 层是否输出由 `serialize::push_overrides` 决定（仅 != base 才输出）。
    #[must_use]
    pub fn as_arg(self) -> &'static str {
        match self {
            OcrStrictMode::Strict => "strict",
            OcrStrictMode::Partial => "partial",
            OcrStrictMode::Fallback => "fallback",
        }
    }
}

/// OCR settings — OCR engine, language, visibility, and detection parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrSettings {
    // from k2pdfopt.h:271
    /// OCR output filename (ocrout). Default empty.
    pub ocrout: String,

    // from k2pdfopt.h:272
    /// Enable OCR on output (dst_ocr). Default 'm' (mupdf).
    /// C uses int: 0=off, 'm'=mupdf native text, 't'=tesseract.
    /// Rust uses an enum for clarity.
    pub dst_ocr: OcrMode,

    // from k2pdfopt.h:273
    /// OCR visibility bounding boxes (ocrvbb). Default 0.
    pub ocrvbb: bool,

    // from k2pdfopt.h:274
    /// OCR sort (ocrsort). Default 0.
    pub ocrsort: bool,

    // from k2pdfopt.h:275
    /// OCR detection type (ocr_detection_type). Default 'l'.
    pub ocr_detection_type: OcrDetectionType,

    // from k2pdfopt.h:276
    /// OCR DPI (ocr_dpi). Default 300. 0=use input DPI, negative=letter height in px.
    pub ocr_dpi: i32,

    // from k2pdfopt.h:282
    /// OCR language (dst_ocr_lang). Default empty (uses eng).
    pub dst_ocr_lang: String,

    // from k2pdfopt.h:291 / Step 11.11 P1-2
    /// OCR visibility flags (dst_ocr_visibility_flags) —— newtype 形式（Step 11.11）。
    ///
    /// 类型升级：v0.1.0 时是裸 i32，Step 11.11 升级为 [`OcrVisibility`] newtype 增强
    /// 可读性 + 类型安全（含 5 const + `contains` helper）。**Breaking change**：
    /// v0.2-alpha 阶段允许（与 v0.1.0 API 不向后兼容）。
    ///
    /// 默认 [`OcrVisibility::DEFAULT`] = `SHOW_SOURCE` = 0x01（与 C `k2settings.c:72` 字面一致）。
    pub dst_ocr_visibility_flags: OcrVisibility,

    // from k2pdfopt.h:292
    /// Max columns for OCR (ocr_max_columns). -1 = use max_columns. Default -1.
    ///
    /// 保留 `i32` 类型而非 plan §6.2 字面 `u8`：C 字段语义中 `-1` 表示
    /// "委托 [`crate::LayoutSettings::max_columns`] 决定"，u8 无法表达 -1 哨兵。
    /// 偏离 plan §6.2 记 Open Question 11.11.B。
    pub ocr_max_columns: i32,

    // from k2pdfopt.h:293
    /// Max OCR region height in inches (ocr_max_height_inches). Default 1.5.
    pub ocr_max_height_inches: f64,

    // from k2pdfopt.h:294
    /// Sort OCR text (sort_ocr_text). Default 0.
    pub sort_ocr_text: bool,

    // Step 11.11 P1-4 —— OCR word 置信度过滤阈值。
    /// 低于此值的 OCR word 被丢弃；范围 `0.0..=1.0`。`0.0` = 不过滤（默认）。
    ///
    /// 默认 `0.0` 与 [`k2ocr::OcrPageInput::min_confidence`] 默认值对齐保 v0.1.0
    /// 行为不破坏；偏离 plan §6.2 字面 `0.5` 默认值（Open Q 11.11.C 推迟评估）。
    /// 通过 [`crate::serialize::Settings::to_args`] 反向序列化为 `--ocr-min-confidence <F>`。
    pub ocr_min_confidence: f32,

    // Step 11.9 P0-6 —— 缺语言时的行为开关（Rust 安全增强项，C 版无对应字段）。
    /// OCR 缺语言策略：Strict / Partial / Fallback (默认 = v0.1.0 行为)。
    pub ocr_strict_mode: OcrStrictMode,
}

/// OCR mode — replaces C int dst_ocr.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum OcrMode {
    /// No OCR (dst_ocr = 0)
    Off,
    /// MuPDF native text extraction (dst_ocr = 'm' = 109)
    #[default]
    Mupdf,
    /// Tesseract OCR (dst_ocr = 't' = 116)
    Tesseract,
}

impl OcrMode {
    pub fn to_c_int(&self) -> i32 {
        match self {
            OcrMode::Off => 0,
            OcrMode::Mupdf => 'm' as i32,
            OcrMode::Tesseract => 't' as i32,
        }
    }

    pub fn from_c_int(v: i32) -> Option<Self> {
        match v {
            0 => Some(OcrMode::Off),
            109 => Some(OcrMode::Mupdf),     // 'm'
            116 => Some(OcrMode::Tesseract), // 't'
            _ => None,
        }
    }
}

impl Default for OcrSettings {
    fn default() -> Self {
        // Default values from k2settings.c:55-75
        Self {
            // k2settings.c:56
            ocrout: String::new(),
            // k2settings.c:66 — with mupdf
            dst_ocr: OcrMode::Mupdf,
            // k2settings.c:67
            ocrvbb: false,
            // k2settings.c:68
            ocrsort: false,
            // k2settings.c:57
            ocr_detection_type: OcrDetectionType::Line,
            // k2settings.c:59-60
            ocr_dpi: 300,
            // k2settings.c:62
            dst_ocr_lang: String::new(),
            // k2settings.c:72 → Step 11.11 P1-2 newtype DEFAULT = SHOW_SOURCE (= 1)
            dst_ocr_visibility_flags: OcrVisibility::DEFAULT,
            // k2settings.c:64
            ocr_max_columns: -1,
            // k2settings.c:73
            ocr_max_height_inches: 1.5,
            // k2settings.c:74
            sort_ocr_text: false,
            // Step 11.11 P1-4 默认 0.0 与 OcrPageInput::min_confidence 对齐保 v0.1.0 行为
            ocr_min_confidence: 0.0,
            // Step 11.9 P0-6 默认 = v0.1.0 行为（fallback to eng + 部分允许）
            ocr_strict_mode: OcrStrictMode::default(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // ── OcrStrictMode 默认值与 PartialEq ──

    /// OcrStrictMode::default() 必须为 Fallback —— 兑现 Step 11.9 "默认 = v0.1.0 行为" 承诺。
    #[test]
    fn ocr_strict_mode_default_is_fallback() {
        assert_eq!(OcrStrictMode::default(), OcrStrictMode::Fallback);
        assert_eq!(
            OcrSettings::default().ocr_strict_mode,
            OcrStrictMode::Fallback
        );
    }

    // ── to_resolve_bools 三个变体映射 (Step 11.9 #5 单测 1-3) ──
    //
    // 这三个测试对应 execution-plan §11.9 操作清单 #5 字面：
    //   "strict 模式缺即 Err / partial 模式丢失 / fallback 默认"
    //
    // 端到端验证（缺 → Err、丢失 → drop、默认 → eng 兜底）由
    //   - k2ocr::lang 13+ 单测覆盖 ResolveOptions 三态行为
    //   - k2pipeline::ocr_bridge 7+ 单测覆盖 resolve_lang_via_engine 端到端
    //   - 验收命令 #1/#2/#3 实地 CLI 兜底
    // 三层共同保证。本层仅验"OcrStrictMode → (fallback_to_eng, allow_partial)"
    // 字面对照表。

    /// Strict → (false, false)：缺即 Err，不 fallback。
    #[test]
    fn ocr_strict_mode_strict_maps_to_no_fallback_no_partial() {
        let (fallback_to_eng, allow_partial) = OcrStrictMode::Strict.to_resolve_bools();
        assert!(!fallback_to_eng, "Strict 必须 fallback_to_eng=false");
        assert!(!allow_partial, "Strict 必须 allow_partial=false");
    }

    /// Partial → (false, true)：丢弃缺失段保留命中段，全缺才报错（不 fallback）。
    #[test]
    fn ocr_strict_mode_partial_maps_to_allow_partial_no_fallback() {
        let (fallback_to_eng, allow_partial) = OcrStrictMode::Partial.to_resolve_bools();
        assert!(!fallback_to_eng, "Partial 必须 fallback_to_eng=false");
        assert!(allow_partial, "Partial 必须 allow_partial=true");
    }

    /// Fallback → (true, true)：与 v0.1.0 完全一致（ResolveOptions::default）。
    #[test]
    fn ocr_strict_mode_fallback_maps_to_v010_default() {
        let (fallback_to_eng, allow_partial) = OcrStrictMode::Fallback.to_resolve_bools();
        assert!(fallback_to_eng, "Fallback 必须 fallback_to_eng=true");
        assert!(allow_partial, "Fallback 必须 allow_partial=true");
    }

    // ── from_arg / as_arg 对偶 ──

    #[test]
    fn ocr_strict_mode_from_arg_parses_three_modes_case_insensitive() {
        assert_eq!(
            OcrStrictMode::from_arg("strict"),
            Some(OcrStrictMode::Strict)
        );
        assert_eq!(
            OcrStrictMode::from_arg("STRICT"),
            Some(OcrStrictMode::Strict)
        );
        assert_eq!(
            OcrStrictMode::from_arg("partial"),
            Some(OcrStrictMode::Partial)
        );
        assert_eq!(
            OcrStrictMode::from_arg("Fallback"),
            Some(OcrStrictMode::Fallback)
        );
        assert_eq!(OcrStrictMode::from_arg("nope"), None);
        assert_eq!(OcrStrictMode::from_arg(""), None);
    }

    #[test]
    fn ocr_strict_mode_as_arg_returns_canonical_lowercase() {
        assert_eq!(OcrStrictMode::Strict.as_arg(), "strict");
        assert_eq!(OcrStrictMode::Partial.as_arg(), "partial");
        assert_eq!(OcrStrictMode::Fallback.as_arg(), "fallback");
    }

    #[test]
    fn ocr_strict_mode_from_arg_as_arg_roundtrip() {
        for mode in [
            OcrStrictMode::Strict,
            OcrStrictMode::Partial,
            OcrStrictMode::Fallback,
        ] {
            let s = mode.as_arg();
            assert_eq!(OcrStrictMode::from_arg(s), Some(mode));
        }
    }

    // ── OcrVisibility (Step 11.11 P1-2) ──
    //
    // 这一组测试覆盖 execution-plan §11.11 操作清单 #5 字面：
    //   "8 单测：visibility default + show_source + show_boxes + 各 ROI +
    //    min_confidence 过滤 + roundtrip"
    // 其中 visibility default / show_source / show_boxes / 位与判断 4 个在
    // ocr.rs 单测；ROI + min_confidence 过滤 + roundtrip 4 个分布到
    // ocr_bridge.rs / roundtrip_test.rs。

    /// 默认值 = SHOW_SOURCE = 0x01（与 C `k2settings.c:72` 字面 一致 + 与 v0.1.0 行为兼容）。
    /// 偏离 plan §6.2 字面 `DEFAULT = Self(2)`，Open Q 11.11.A 推迟评估。
    #[test]
    fn ocr_visibility_default_is_show_source() {
        assert_eq!(OcrVisibility::default(), OcrVisibility::SHOW_SOURCE);
        assert_eq!(OcrVisibility::DEFAULT.bits(), 1);
        assert_eq!(OcrSettings::default().dst_ocr_visibility_flags.bits(), 1);
    }

    /// 5 个 const 的字面 bit 值与 C `k2pdfopt.h:291` 注释一致。
    #[test]
    fn ocr_visibility_const_bit_values_match_c() {
        assert_eq!(OcrVisibility::SHOW_SOURCE.bits(), 0x01);
        assert_eq!(OcrVisibility::SHOW_OCR_TEXT.bits(), 0x02);
        assert_eq!(OcrVisibility::SHOW_BOXES.bits(), 0x04);
        assert_eq!(OcrVisibility::USE_SPACES.bits(), 0x08);
        assert_eq!(OcrVisibility::OPTIMIZED.bits(), 0x10);
        assert_eq!(OcrVisibility::ALL_BITS_MAX, 0x1F);
    }

    /// `contains` 位与判断：组合 SHOW_SOURCE | SHOW_OCR_TEXT | SHOW_BOXES (= 7)
    /// 时三 bit 都命中，未设置的 USE_SPACES / OPTIMIZED 都未命中。
    #[test]
    fn ocr_visibility_contains_bit_combinations() {
        let v = OcrVisibility::from_bits(0x07);
        assert!(v.contains(OcrVisibility::SHOW_SOURCE));
        assert!(v.contains(OcrVisibility::SHOW_OCR_TEXT));
        assert!(v.contains(OcrVisibility::SHOW_BOXES));
        assert!(!v.contains(OcrVisibility::USE_SPACES));
        assert!(!v.contains(OcrVisibility::OPTIMIZED));

        // 默认 SHOW_SOURCE 仅命中自己
        let d = OcrVisibility::default();
        assert!(d.contains(OcrVisibility::SHOW_SOURCE));
        assert!(!d.contains(OcrVisibility::SHOW_OCR_TEXT));
        assert!(!d.contains(OcrVisibility::SHOW_BOXES));
    }

    /// `pdf_text_render_mode` 映射：含 SHOW_OCR_TEXT → Some(3) Tr 3 invisible；
    /// 不含 → None（PDF writer 应跳过整个 BT...ET 段，零开销 + 0 byte content stream）。
    #[test]
    fn ocr_visibility_pdf_text_render_mode_maps_correctly() {
        // 不含 SHOW_OCR_TEXT → None
        assert_eq!(OcrVisibility::SHOW_SOURCE.pdf_text_render_mode(), None);
        assert_eq!(OcrVisibility::SHOW_BOXES.pdf_text_render_mode(), None);
        assert_eq!(OcrVisibility::from_bits(0).pdf_text_render_mode(), None);

        // 含 SHOW_OCR_TEXT → Some(3)
        assert_eq!(OcrVisibility::SHOW_OCR_TEXT.pdf_text_render_mode(), Some(3));
        assert_eq!(
            OcrVisibility::from_bits(0x07).pdf_text_render_mode(),
            Some(3)
        );
        assert_eq!(
            OcrVisibility::from_bits(OcrVisibility::ALL_BITS_MAX).pdf_text_render_mode(),
            Some(3)
        );
    }

    /// `from_bits` / `bits` 对偶 roundtrip。
    #[test]
    fn ocr_visibility_from_bits_bits_roundtrip() {
        for raw in 0u8..=OcrVisibility::ALL_BITS_MAX {
            let v = OcrVisibility::from_bits(raw);
            assert_eq!(v.bits(), raw);
        }
    }

    /// OcrSettings 新增字段 `ocr_min_confidence` 默认 0.0（与
    /// OcrPageInput::min_confidence 默认对齐保 v0.1.0 行为）。
    /// 偏离 plan §6.2 字面 `0.5` 默认，Open Q 11.11.C 推迟评估。
    #[test]
    fn ocr_settings_min_confidence_default_is_zero() {
        let s = OcrSettings::default();
        assert!((s.ocr_min_confidence - 0.0).abs() < f32::EPSILON);
    }
}
