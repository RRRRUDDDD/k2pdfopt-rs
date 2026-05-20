//! `wrap` - 文本行 reflow 高层入口（Step 8.1 / M6）
//!
//! 提供 [`WrapPipeline`] 把 [`crate::master::WrapState`] 的低层 `add_word` / `flush`
//! API 升级为「输入一个 [`crate::words::TextRow`]，输出 [`crate::master::wrap_state::FlushedLine`]」
//! 的高层流程，并维护 src_left_to_right / max_region_width / text_wrap 等 [`WrapPipelineSettings`]。
//!
//! # 设计目标
//!
//! - 不直接回调 [`crate::master::ConvertContext::add_bitmap`]（避免 Rust 借用循环）
//! - flush 返回 `FlushedLine` 让调用方自行注入 master canvas
//! - hyphen detect 留 stub（Step 8.2 落地），但 `add_word` 已支持调用方手填 [`crate::master::wrap_state::HyphenInfo`]
//!
//! # C 对照
//!
//! - C 入口：`k2master.c::masterinfo_add_bitmap` 中的 wrap 分支（约 k2master.c:600-700）
//! - C wrapbmp_add：`wrapbmp.c:125-383`
//! - C wrapbmp_flush：`wrapbmp.c:386-576`

use crate::master::wrap_state::{AddRegion, FlushedLine, MasterGapCarry, WrapState};
use k2types::BitmapError;

/// `WrapPipeline` 的运行时配置。
///
/// 对应 C 版 `K2PDFOPT_SETTINGS` 中与 wrap 相关的字段子集：
/// - `text_wrap`：是否启用 reflow（`bool` 形态，C 是 `int` 0/1/2，详见 Step 3.4）
/// - `max_region_width_inches`：行最大宽度（英寸）
/// - `src_dpi`：源 DPI
/// - `src_left_to_right`：源文字方向
/// - `allow_full_justification`：是否允许 full-justify
#[derive(Debug, Clone, Copy)]
pub struct WrapPipelineSettings {
    /// 是否启用 reflow（C `k2settings->text_wrap`）。
    pub text_wrap: bool,
    /// 最大行宽（英寸）。C `k2settings->max_region_width_inches`。
    pub max_region_width_inches: f64,
    /// 源 DPI。C `k2settings->src_dpi`。
    pub src_dpi: f64,
    /// 文字方向：true=LTR，false=RTL。C `k2settings->src_left_to_right`。
    pub src_left_to_right: bool,
    /// 是否允许 full-justify。C 函数 `wrapbmp_flush` 的 `allow_full_justification` 参数。
    pub allow_full_justification: bool,
}

impl Default for WrapPipelineSettings {
    fn default() -> Self {
        Self {
            text_wrap: true,
            max_region_width_inches: 3.4, // C 版默认（与 k2settings.c 一致）
            src_dpi: 300.0,
            src_left_to_right: true,
            allow_full_justification: true,
        }
    }
}

/// 添加 word region 时的结果（包含 carry 状态 + 是否触发了内部 flush）。
#[derive(Debug, Clone, Copy)]
pub struct AddOutcome {
    /// MASTERINFO carry 状态（调用方需根据此清零 mandatory_region_gap）。
    pub carry: MasterGapCarry,
    /// 当前 word 加入后是否已经超出 `max_region_width`，建议立即调 [`WrapPipeline::flush`]。
    pub should_flush: bool,
}

/// 文本行 reflow 高层流水线。
///
/// 内部持有 [`WrapState`] 实例。每次调 [`Self::add_word`] 后通过 `AddOutcome::should_flush`
/// 提示是否应该立即 flush 出一行。调用方应在 word 流末尾再调一次 [`Self::flush`] 兜底。
#[derive(Debug)]
pub struct WrapPipeline {
    /// 内部 wrap 状态（暴露给调用方读，写入仅通过 add_word/flush）。
    pub state: WrapState,
    /// 配置。
    pub settings: WrapPipelineSettings,
}

impl WrapPipeline {
    /// 构造 pipeline。
    ///
    /// `is_color` 决定 [`WrapState`] 的内部 bitmap 是否用 RGB（true）还是灰度（false）。
    /// 对应 C `wrapbmp_set_color(&wrapbmp, k2settings->dst_color)`（`k2master.c:96`）。
    #[must_use]
    pub fn new(settings: WrapPipelineSettings, is_color: bool) -> Self {
        let mut state = WrapState::new();
        state.set_color(is_color);
        Self { state, settings }
    }

    /// 把一个 [`AddRegion`] 加入 wrap 缓冲区。
    ///
    /// 内部调 [`WrapState::add_word`]，并基于 [`WrapState::remaining`] 决定是否需要 flush。
    /// 与 C 版 `wrapbmp_add` 的语义一致：调用方负责在收到 `should_flush=true` 后手动 flush。
    ///
    /// # 参数
    ///
    /// - `region`：待加入的 word region
    /// - `colgap`：与前一个 word 的横向 gap（pixels）
    /// - `just_flags`：段落对齐 flags
    /// - `mandatory_region_gap_carry` / `page_region_gap_in_carry`：调用方从 MASTERINFO
    ///   带过来的 gap，若 carry 返回 `Absorbed`，调用方应清零 MASTERINFO 对应字段
    pub fn add_word(
        &mut self,
        region: &AddRegion<'_>,
        colgap: i32,
        just_flags: i32,
        mandatory_region_gap_carry: i32,
        page_region_gap_in_carry: f64,
    ) -> Result<AddOutcome, BitmapError> {
        let carry = self.state.add_word(
            region,
            colgap,
            just_flags,
            self.settings.src_left_to_right,
            mandatory_region_gap_carry,
            page_region_gap_in_carry,
        )?;
        let remaining = self.state.remaining(
            self.settings.max_region_width_inches,
            self.settings.src_dpi,
            self.settings.src_left_to_right,
        );
        // remaining <= 0 表示已超出最大行宽
        Ok(AddOutcome {
            carry,
            should_flush: remaining <= 0,
        })
    }

    /// Flush 当前 wrap 累积内容，返回 [`FlushedLine`]。
    pub fn flush(&mut self) -> Result<Option<FlushedLine>, BitmapError> {
        self.state.flush(
            self.settings.text_wrap,
            self.settings.allow_full_justification,
        )
    }

    /// 强制重置 pipeline（不产出 [`FlushedLine`]，丢弃累积内容）。
    pub fn reset(&mut self) {
        self.state.reset();
    }

    /// 是否当前以 hyphen 收尾。代理 [`WrapState::ends_in_hyphen`]。
    #[must_use]
    pub fn ends_in_hyphen(&self) -> bool {
        self.state.ends_in_hyphen()
    }

    /// 是否当前 wrap 为空。代理 [`WrapState::is_empty`]。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state.is_empty()
    }
}

/// Hyphen 检测：见 [`crate::hyphen::detect_hyphen`]（Step 8.2 落地）。
///
/// 本模块保留 hyphen detect 的"调用位"对照说明：C `wrapbmp_add` 在
/// `wrapbmp.c:138` 内部调 `bmpregion_hyphen_detect`，但 Rust 端为避免循环借用，
/// 改为由调用方在 `WrapState::add_word` **之前** 调 [`crate::hyphen::detect_hyphen`]
/// 并把结果填入 [`AddRegion::hyphen`]。
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::master::wrap_state::HyphenInfo;
    use k2types::PixelFormat;

    fn mk_region<'a>(
        pixels: &'a [u8],
        w: u32,
        h: u32,
        c1: i32,
        c2: i32,
        r1: i32,
        r2: i32,
        rowbase: i32,
    ) -> AddRegion<'a> {
        AddRegion {
            pixels,
            src_full_width: w,
            src_full_height: h,
            format: PixelFormat::Gray8,
            c1,
            c2,
            r1,
            r2,
            rowbase,
            rowheight: r2 - r1 + 4,
            gap: 2,
            gapblank: 1,
            bgcolor: 255,
            pageno: 0,
            dpi: 300.0,
            rotdeg: 0,
            hyphen: HyphenInfo::none(),
        }
    }

    #[test]
    fn pipeline_default_settings() {
        let s = WrapPipelineSettings::default();
        assert!(s.text_wrap);
        assert!((s.max_region_width_inches - 3.4).abs() < 1e-9);
        assert_eq!(s.src_dpi, 300.0);
        assert!(s.src_left_to_right);
    }

    #[test]
    fn pipeline_new_initializes_empty() {
        let p = WrapPipeline::new(WrapPipelineSettings::default(), false);
        assert!(p.is_empty());
        assert!(!p.ends_in_hyphen());
    }

    #[test]
    fn pipeline_add_word_first_absorbs_carry() {
        let mut p = WrapPipeline::new(WrapPipelineSettings::default(), false);
        let pixels = vec![100u8; 30];
        let r = mk_region(&pixels, 5, 6, 0, 2, 0, 2, 1);
        let out = p.add_word(&r, 0, 0x88, 5, 0.25).unwrap();
        assert_eq!(out.carry, MasterGapCarry::Absorbed);
        assert!(!p.is_empty());
    }

    #[test]
    fn pipeline_should_flush_when_remaining_zero_or_negative() {
        // max_region = 4 inch * 300 dpi = 1200 px。把 region 调到 1300 px
        let settings = WrapPipelineSettings {
            text_wrap: true,
            max_region_width_inches: 4.0,
            src_dpi: 300.0,
            src_left_to_right: true,
            allow_full_justification: true,
        };
        let mut p = WrapPipeline::new(settings, false);
        let h = 4u32;
        let w = 1300u32;
        let pixels = vec![100u8; (w * h) as usize];
        let r = mk_region(&pixels, w, h, 0, (w - 1) as i32, 0, 2, 1);
        let out = p.add_word(&r, 0, 0x88, 0, 0.0).unwrap();
        assert!(out.should_flush);
    }

    #[test]
    fn pipeline_flush_returns_flushed_line() {
        let mut p = WrapPipeline::new(WrapPipelineSettings::default(), false);
        let pixels = vec![100u8; 30];
        let r = mk_region(&pixels, 5, 6, 0, 2, 0, 2, 1);
        p.add_word(&r, 0, 0x88, 0, 0.0).unwrap();
        let line = p.flush().unwrap().expect("Some line");
        assert_eq!(line.bitmap.width, 3);
        assert!(p.is_empty());
    }

    #[test]
    fn pipeline_flush_off_when_text_wrap_disabled() {
        let settings = WrapPipelineSettings {
            text_wrap: false,
            ..WrapPipelineSettings::default()
        };
        let mut p = WrapPipeline::new(settings, false);
        let pixels = vec![100u8; 30];
        let r = mk_region(&pixels, 5, 6, 0, 2, 0, 2, 1);
        p.add_word(&r, 0, 0x88, 0, 0.0).unwrap();
        assert!(p.flush().unwrap().is_none());
    }

    #[test]
    fn pipeline_reset_clears_state() {
        let mut p = WrapPipeline::new(WrapPipelineSettings::default(), false);
        let pixels = vec![100u8; 30];
        let r = mk_region(&pixels, 5, 6, 0, 2, 0, 2, 1);
        p.add_word(&r, 0, 0x88, 0, 0.0).unwrap();
        p.reset();
        assert!(p.is_empty());
    }
}
