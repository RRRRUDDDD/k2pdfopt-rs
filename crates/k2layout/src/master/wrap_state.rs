//! `wrap_state` - Reflow / line-wrap 缓冲区桶。
//!
//! 见 [`crate::master`] 模块文档与 `docs/masterinfo-design.md` §2 第 4 行。
//!
//! # C 字段对应
//!
//! 来源：
//! - `k2pdfoptlib/k2pdfopt.h::WRAPBMP`（638-654 行的 struct 定义）
//! - `k2pdfoptlib/k2pdfopt.h::WRECTMAP`/`WRECTMAPS`（571-588 行）
//! - `k2pdfoptlib/k2pdfopt.h::HYPHENINFO`（500-506 行）
//! - `k2pdfoptlib/wrapbmp.c`（854 行实现）
//!
//! # Step 8.1（M6）落地范围
//!
//! 把 WRAPBMP god struct 移植为 Rust 端 [`WrapState`]，并补齐 [`WRectMap`] / [`WRectMaps`] /
//! [`HyphenInfo`] 子结构。落地以下薄方法 + 主算法：
//!
//! - `new` / `reset` / `set_color` / `set_maxgap` / `width` / `is_empty` / `ends_in_hyphen`
//!   / `remaining`（C: wrapbmp_init/reset/set_color/set_maxgap/width/ends_in_hyphen/remaining）
//! - `add_word`：把一个 word region 累积到 wrap 缓冲区（C: wrapbmp_add，~250 行 1:1 复刻）
//! - `flush`：把累积内容转为 [`FlushedLine`] 返回给调用方（C: wrapbmp_flush 的"产出"部分；
//!   bmpregion_add 回调由 ConvertContext 在 Step 8.2/8.3 完成串联）
//! - `hyphen_erase`：擦除尾部 hyphen 像素（C: wrapbmp_hyphen_erase，但 hyphen detect 本身
//!   推迟到 Step 8.2）

use k2types::{Bitmap, BitmapError, PixelFormat};

// ---------------------------------------------------------------------------
// HyphenInfo - 对应 C HYPHENINFO（k2pdfopt.h:500-506）
// ---------------------------------------------------------------------------

/// 行尾 hyphen（连字符）检测信息。
///
/// 对应 C `HYPHENINFO` struct（`k2pdfoptlib/k2pdfopt.h:500-506`）。
/// `ch < 0` 表示无 hyphen。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HyphenInfo {
    /// Hyphen 起始列（< 0 表示无 hyphen）。C `ch`。
    pub ch: i32,
    /// Hyphen 末尾列（删除 hyphen 后行内容延伸到的最右列）。C `c2`。
    pub c2: i32,
    /// Hyphen 顶部行。C `r1`。
    pub r1: i32,
    /// Hyphen 底部行。C `r2`。
    pub r2: i32,
}

impl HyphenInfo {
    /// 构造"无 hyphen"的默认值（ch=-1）。
    #[must_use]
    pub const fn none() -> Self {
        Self {
            ch: -1,
            c2: -1,
            r1: -1,
            r2: -1,
        }
    }

    /// 是否检测到 hyphen。对应 C `wrapbmp_ends_in_hyphen` 的判定 `ch >= 0`。
    #[must_use]
    pub const fn is_hyphen(&self) -> bool {
        self.ch >= 0
    }
}

impl Default for HyphenInfo {
    fn default() -> Self {
        Self::none()
    }
}

// ---------------------------------------------------------------------------
// WRectMap - 对应 C WRECTMAP（k2pdfopt.h:571-582）
// ---------------------------------------------------------------------------

/// 一个 word 矩形映射到源页坐标的 entry。
///
/// 对应 C `WRECTMAP`（`k2pdfopt.h:571-582`）。`coords[0..3]` 与 C 严格一致：
/// - `coords[0]`：源页 bitmap 上的左上角（像素）
/// - `coords[1]`：wrap bitmap 上的左上角（像素）
/// - `coords[2]`：region 的宽 / 高（像素）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WRectMap {
    /// 源页码。C `srcpageno`。
    pub srcpageno: i32,
    /// 源 bitmap 宽度（pixels）。C `srcwidth`。
    pub srcwidth: i32,
    /// 源 bitmap 高度（pixels）。C `srcheight`。
    pub srcheight: i32,
    /// 源 DPI 水平。C `srcdpiw`。
    pub srcdpiw: f64,
    /// 源 DPI 垂直。C `srcdpih`。
    pub srcdpih: f64,
    /// 源旋转角度（degrees）。C `srcrot`。
    pub srcrot: i32,
    /// 三组坐标（x/y）。语义见 struct doc。C `coords[3]`。
    pub coords: [(f64, f64); 3],
}

impl WRectMap {
    /// 构造默认 WRectMap（全 0）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            srcpageno: 0,
            srcwidth: 0,
            srcheight: 0,
            srcdpiw: 0.0,
            srcdpih: 0.0,
            srcrot: 0,
            coords: [(0.0, 0.0); 3],
        }
    }

    /// 判断点 `(xc, yc)` 是否在 wrap bitmap 上的 region 范围内。
    /// 对应 C `wrectmap_inside`（`wrapbmp.c:727-732`）。
    #[must_use]
    pub fn inside(&self, xc: f64, yc: f64) -> bool {
        let (x, y) = self.coords[1];
        let (w, h) = self.coords[2];
        x <= xc && x + w >= xc && y <= yc && y + h >= yc
    }
}

impl Default for WRectMap {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// WRectMaps - 对应 C WRECTMAPS（k2pdfopt.h:584-588）
// ---------------------------------------------------------------------------

/// `WRectMap` 容器。对应 C `WRECTMAPS`（`k2pdfopt.h:584-588`）。
///
/// C 用 `wrectmap*[n,na]` 手动管理容量；Rust 直接用 `Vec`，零额外字段。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WRectMaps {
    /// 已添加的 entries。C `wrectmap[n]`。
    pub items: Vec<WRectMap>,
}

impl WRectMaps {
    /// 构造空容器。对应 C `wrectmaps_init`（`wrapbmp.c:651-656`）。
    #[must_use]
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// 容器是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 当前元素个数。对应 C `wrectmaps.n`。
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 清空但不释放容量。对应 C `wrectmaps_clear`（`wrapbmp.c:669-673`）。
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// 追加一个 `WRectMap`。对应 C `wrectmaps_add_wrectmap`（`wrapbmp.c:676-690`）。
    pub fn add(&mut self, item: WRectMap) {
        self.items.push(item);
    }

    /// 按水平坐标 `coords[1].x` 排序（升序）。
    /// 对应 C `wrectmaps_sort_horizontally`（`wrapbmp.c:735-787`，C 用 heapsort，
    /// Rust 用 stable sort_by；同键时顺序不同对 OCR 输出顺序无影响）。
    pub fn sort_horizontally(&mut self) {
        self.items.sort_by(|a, b| {
            a.coords[1]
                .0
                .partial_cmp(&b.coords[1].0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// 缩放所有 entry 的 wrap-bitmap 坐标（`coords[1]`/`coords[2]`/`srcdpi*`）。
    /// 对应 C `wrectmaps_scale_wrapbmp_coords`（`wrapbmp.c:697-724`）。
    pub fn scale_wrapbmp_coords(&mut self, scalew: f64, scaleh: f64) {
        for m in &mut self.items {
            let w = m.srcwidth as f64;
            m.srcwidth = (w * scalew + 0.5) as i32;
            let scalew_eff = if w > 0.0 {
                m.srcwidth as f64 / w
            } else {
                scalew
            };
            let h = m.srcheight as f64;
            m.srcheight = (h * scaleh + 0.5) as i32;
            let scaleh_eff = if h > 0.0 {
                m.srcheight as f64 / h
            } else {
                scaleh
            };
            m.srcdpiw *= scalew_eff;
            m.srcdpih *= scaleh_eff;
            m.coords[0].0 *= scalew_eff;
            m.coords[0].1 *= scaleh_eff;
            m.coords[1].0 *= scalew_eff;
            m.coords[1].1 *= scaleh_eff;
            m.coords[2].0 *= scalew_eff;
            m.coords[2].1 *= scaleh_eff;
        }
    }
}

// ---------------------------------------------------------------------------
// FlushedLine - flush 的产出（替代 C 版直接回调 bmpregion_add）
// ---------------------------------------------------------------------------

/// `WrapState::flush` 产出的累积行结果。
///
/// **设计动机**：C 版 `wrapbmp_flush` 直接回调 `bmpregion_add`（即 [`super::ConvertContext::add_bitmap`]）
/// 形成强耦合循环；Rust 端拆为 produce-and-return：[`WrapState::flush`] 返回 `FlushedLine`，
/// 调用方（[`super::ConvertContext`] 在 Step 8.2/8.3）拿到后自行 `calc_bbox` 与注入 master canvas。
#[derive(Debug, Clone)]
pub struct FlushedLine {
    /// 已累积好的位图（含整行文本，背景白）。
    pub bitmap: Bitmap,
    /// 基线 y 坐标（pixel from top）。C `wrapbmp.base`。
    pub base: i32,
    /// 文本行 rowheight（与 `wrapbmp.textrow.rowheight` 一致）。
    pub rowheight: i32,
    /// 文本行 gap（rowheight - actual text height）。
    pub gap: i32,
    /// 文本行 gapblank。C `wrapbmp.textrow.gapblank`。
    pub gapblank: i32,
    /// 背景色（0-255）。C `wrapbmp.bgcolor`。
    pub bgcolor: u8,
    /// 段落对齐 flags（经 `allow_full_justification` 处理后）。C `just` 计算结果。
    pub just_flags: i32,
    /// wrap → source page 坐标映射 entries。C `wrapbmp.wrectmaps`。
    pub wrectmaps: WRectMaps,
    /// 强制分页 region gap（pixels）。C `wrapbmp.mandatory_region_gap`。
    pub mandatory_region_gap: i32,
    /// 源页 region gap（inches）。C `wrapbmp.page_region_gap_in`。
    pub page_region_gap_in: f64,
}

// ---------------------------------------------------------------------------
// AddRegion - add_word 的入参描述
// ---------------------------------------------------------------------------

/// `WrapState::add_word` 的入参（描述一个待加入的 word region）。
///
/// 对应 C 版 `BMPREGION *region` 在 `wrapbmp_add` 函数中实际用到的字段子集。
/// 完整 `BMPREGION` 体积巨大（k2pdfopt.h:594-615）且大半字段 wrap 链路用不到，
/// 故 Rust 端只取必要部分（pixels/c1/c2/r1/r2/rowbase/pageno/dpi/...）。
#[derive(Debug, Clone)]
pub struct AddRegion<'a> {
    /// 源 bitmap 像素（行优先，灰度或 RGB）。C `region->bmp8`（gray）或 `region->bmp`（color）。
    pub pixels: &'a [u8],
    /// 源 bitmap 宽（pixels）。C `region->bmp8->width` / `region->bmp->width`。
    pub src_full_width: u32,
    /// 源 bitmap 高（pixels）。C `region->bmp8->height`。
    pub src_full_height: u32,
    /// region 的像素格式（决定 bpp 与 stride）。
    pub format: PixelFormat,
    /// region 左列（inclusive）。C `region->c1`。
    pub c1: i32,
    /// region 右列（inclusive）。C `region->c2`。
    pub c2: i32,
    /// region 顶部行（inclusive）。C `region->r1`。
    pub r1: i32,
    /// region 底部行（inclusive）。C `region->r2`。
    pub r2: i32,
    /// 基线 y。C `region->bbox.rowbase`。
    pub rowbase: i32,
    /// 行高。C `region->bbox.rowheight`。
    pub rowheight: i32,
    /// 行间 gap。C `region->bbox.gap`。
    pub gap: i32,
    /// gapblank。C `region->bbox.gapblank`。
    pub gapblank: i32,
    /// region 背景色（0-255）。C `region->bgcolor`。
    pub bgcolor: u8,
    /// region 源页号（用于 wrectmap）。C `region->pageno`。
    pub pageno: i32,
    /// region 源 DPI。C `region->dpi`。
    pub dpi: f64,
    /// region 源旋转。C `region->rotdeg`。
    pub rotdeg: i32,
    /// hyphen 信息（由调用方在 add_word 之前调 hyphen_detect 填好；
    /// Step 8.1 仅支持调用方手填，hyphen_detect 主算法在 Step 8.2 落地）。
    pub hyphen: HyphenInfo,
}

// ---------------------------------------------------------------------------
// WrapState - 主结构，对应 C WRAPBMP
// ---------------------------------------------------------------------------

/// Reflow / line-wrap 缓冲区桶。对应 C `WRAPBMP`（`k2pdfopt.h:638-654`）。
///
/// 累积 word region 到一个内部 bitmap，达到目标行宽（`max_region_width_inches * src_dpi`）后
/// 由调用方触发 [`Self::flush`] 把累积内容转为 [`FlushedLine`]。
///
/// # Step 8.1 字段对照
///
/// | C 字段 | Rust 字段 | C 行号 |
/// |--------|-----------|--------|
/// | `bmp` (WILLUSBITMAP) | [`Self::bitmap`] | 640 |
/// | `base` | [`Self::base`] | 641 |
/// | `bgcolor` | [`Self::bgcolor`] | 642 |
/// | `just` | [`Self::just`] | 643 |
/// | `rhmax` | [`Self::rhmax`] | 644 |
/// | `thmax` | [`Self::thmax`] | 645 |
/// | `maxgap` | [`Self::maxgap`] | 646 |
/// | `height_extended` | [`Self::height_extended`] | 647 |
/// | `just_flushed_internal` | [`Self::just_flushed_internal`] | 648 |
/// | `mandatory_region_gap` | [`Self::mandatory_region_gap`] | 649 |
/// | `page_region_gap_in` | [`Self::page_region_gap_in`] | 650 |
/// | `textrow.rowheight/gap/gapblank` | [`Self::textrow_rowheight`] / ... | 651 |
/// | `wrectmaps` | [`Self::wrectmaps`] | 652 |
/// | `hyphen` | [`Self::hyphen`] | 653 |
#[derive(Debug, Clone)]
pub struct WrapState {
    /// 内部 bitmap（累积 word 像素）。C `bmp` (WILLUSBITMAP)。
    ///
    /// `None` 表示尚未首次 `add_word`（与 C `bmp.width==0` 等价语义）。
    pub bitmap: Option<Bitmap>,
    /// 基线 y 坐标（pixel from top）。C `base`。
    pub base: i32,
    /// 背景色（0-255；-1 表示未设置）。C `bgcolor`。
    pub bgcolor: i32,
    /// 段落对齐 flags（v2 起 8-bit 编码，0x8f = 全位 default）。C `just`。
    pub just: i32,
    /// 累积行的最大 rh（rowbase-r1+1）。C `rhmax`。
    pub rhmax: i32,
    /// 累积行的最大 th（rh + (r2-rowbase)）。C `thmax`。
    pub thmax: i32,
    /// 最大允许 gap（pixels）。C `maxgap`，默认 2。
    pub maxgap: i32,
    /// height_extended 标志（v2.00 起 unused，保留与 C 一致以便 round-trip）。
    pub height_extended: i32,
    /// "刚 flush 过"标志（避免连续 flush 重复处理）。C `just_flushed_internal`。
    pub just_flushed_internal: i32,
    /// 强制分页 region gap（pixels）。C `mandatory_region_gap`。
    pub mandatory_region_gap: i32,
    /// 源页 region gap（inches）。C `page_region_gap_in`。
    pub page_region_gap_in: f64,
    /// 文本行 rowheight。对应 C `textrow.rowheight`。
    pub textrow_rowheight: i32,
    /// 文本行 gap。对应 C `textrow.gap`。
    pub textrow_gap: i32,
    /// 文本行 gapblank。对应 C `textrow.gapblank`。
    pub textrow_gapblank: i32,
    /// word rectangle → source page 坐标映射容器。C `wrectmaps`。
    pub wrectmaps: WRectMaps,
    /// 行尾 hyphen 信息。C `hyphen`。
    pub hyphen: HyphenInfo,
}

impl WrapState {
    /// 构造默认空 WrapState（无 bitmap，base=0，just=0x8f，maxgap=2）。
    ///
    /// 对应 C `wrapbmp_init` + `wrapbmp_reset`（`wrapbmp.c:29-63`）。
    /// **未设 bpp**：调用方在首次 `add_word` 前应调 [`Self::set_color`]。
    #[must_use]
    pub fn new() -> Self {
        let mut s = Self {
            bitmap: None,
            base: 0,
            bgcolor: -1,
            just: 0x8f,
            rhmax: -1,
            thmax: -1,
            maxgap: 2,
            height_extended: 0,
            just_flushed_internal: 0,
            mandatory_region_gap: -1,
            page_region_gap_in: -1.0,
            textrow_rowheight: -1,
            textrow_gap: -1,
            textrow_gapblank: 0,
            wrectmaps: WRectMaps::new(),
            hyphen: HyphenInfo::none(),
        };
        s.reset_internal();
        s
    }

    /// 重置 wrap 缓冲区到"刚 flush"状态。
    ///
    /// 对应 C `wrapbmp_reset`（`wrapbmp.c:46-63`）。**注意**：与 C 一致，仅清空累积态，
    /// 不动 `just` / `bgcolor`（这两个字段由调用方在 `add_word` 时设）。
    pub fn reset(&mut self) {
        self.reset_internal();
        self.wrectmaps.clear();
        // bitmap 内部状态（width/height/rows）由 reset_internal 处理，但 Vec 容量保留
        if let Some(b) = self.bitmap.as_mut() {
            b.width = 0;
            b.height = 0;
            b.pixels.clear();
        }
    }

    fn reset_internal(&mut self) {
        self.base = 0;
        self.maxgap = 2;
        self.rhmax = -1;
        self.thmax = -1;
        self.hyphen = HyphenInfo::none();
        self.mandatory_region_gap = -1;
        self.page_region_gap_in = -1.0;
        self.textrow_rowheight = -1;
        self.textrow_gap = -1;
        self.textrow_gapblank = 0;
        self.just_flushed_internal = 1;
    }

    /// 设置 wrap bitmap 的颜色（`true` = 24bpp RGB，`false` = 8bpp 灰度）。
    /// 对应 C `wrapbmp_set_color`（`wrapbmp.c:73-77`）。
    pub fn set_color(&mut self, is_color: bool) {
        let format = if is_color {
            PixelFormat::Rgb8
        } else {
            PixelFormat::Gray8
        };
        // 仅在还没有 bitmap 时记录；已有 bitmap 时 C 仅改 bpp 不重 alloc
        if self.bitmap.is_none() {
            // Bitmap::from_raw 在 width=0/height=0 时返回 Ok（不分配）
            self.bitmap = Bitmap::from_raw(0, 0, 1.0, format, Vec::new()).ok();
        } else if let Some(b) = self.bitmap.as_mut() {
            b.format = format;
        }
    }

    /// 设置 maxgap。对应 C `wrapbmp_set_maxgap`（`wrapbmp.c:88-92`）。
    pub fn set_maxgap(&mut self, value: i32) {
        self.maxgap = value;
    }

    /// 获取 wrap bitmap 当前宽度（pixels）。对应 C `wrapbmp_width`（`wrapbmp.c:95-99`）。
    #[must_use]
    pub fn width(&self) -> u32 {
        self.bitmap.as_ref().map(|b| b.width).unwrap_or(0)
    }

    /// 获取 wrap bitmap 当前高度（pixels）。
    #[must_use]
    pub fn height(&self) -> u32 {
        self.bitmap.as_ref().map(|b| b.height).unwrap_or(0)
    }

    /// 检测当前 wrap 是否以 hyphen 收尾。对应 C `wrapbmp_ends_in_hyphen`（`wrapbmp.c:66-70`）。
    #[must_use]
    pub fn ends_in_hyphen(&self) -> bool {
        self.hyphen.is_hyphen()
    }

    /// wrap 缓冲区是否为空（无累积内容）。
    /// C 等价：`wrapbmp->bmp.width == 0` 判断（`wrapbmp.c:143/185/406/412` 多处使用）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width() == 0
    }

    /// wrap 缓冲区剩余可用宽度（pixels）。
    ///
    /// 对应 C `wrapbmp_remaining`（`wrapbmp.c:102-116`）。剩余 = maxpix - 已用宽度，
    /// 其中 maxpix = `max_region_width_inches * src_dpi`。若 wrap 以 hyphen 收尾，
    /// 排除 hyphen 占的宽度。
    #[must_use]
    pub fn remaining(
        &self,
        max_region_width_inches: f64,
        src_dpi: f64,
        src_left_to_right: bool,
    ) -> i32 {
        let maxpix = (max_region_width_inches * src_dpi) as i32;
        let cur_width = self.width() as i32;
        let w = if !self.ends_in_hyphen() {
            cur_width
        } else if src_left_to_right {
            self.hyphen.c2 + 1
        } else {
            cur_width - self.hyphen.c2
        };
        maxpix - w
    }

    /// 把一个 word region 累积到 wrap 缓冲区。
    ///
    /// **1:1 复刻 C `wrapbmp_add`（`wrapbmp.c:125-383`）**。
    ///
    /// # 参数
    ///
    /// - `region`：[`AddRegion`] 描述待加入的 word region（含像素与 bbox）
    /// - `colgap`：本次拼接的横向 gap（pixels）。如果当前 wrap 已以 hyphen 收尾，
    ///   会内部清零（C 行 139-140）
    /// - `just_flags`：段落对齐 flags（写到 `self.just`）
    /// - `src_left_to_right`：源文字方向（影响 RTL 拼接逻辑）
    /// - `mandatory_region_gap_carry` / `page_region_gap_in_carry`：调用方从
    ///   `MASTERINFO` 复制过来的值；首次 `add_word` 时被吸收（C 行 156-161），
    ///   返回 Some(...) 表示调用方应清零自身 MASTERINFO 对应字段
    ///
    /// # 返回
    ///
    /// 错误路径：内部 `Bitmap::from_raw` 在尺寸溢出时返 `BitmapError::SizeOverflow`。
    pub fn add_word(
        &mut self,
        region: &AddRegion<'_>,
        mut colgap: i32,
        just_flags: i32,
        src_left_to_right: bool,
        mandatory_region_gap_carry: i32,
        page_region_gap_in_carry: f64,
    ) -> Result<MasterGapCarry, BitmapError> {
        // hyphen detect 在 Step 8.2 落地；本步骤假设调用方已通过 region.hyphen 提供
        // C 行 138: bmpregion_hyphen_detect(region, k2settings->hyphen_detect, ...)

        // C 行 139-140: wrap 以 hyphen 收尾时清零 colgap
        if self.ends_in_hyphen() {
            colgap = 0;
        }

        // C 行 141: wrapbmp_hyphen_erase（擦除现有 hyphen 像素）
        self.hyphen_erase(src_left_to_right)?;

        // C 行 142: just_flushed_internal = 0
        self.just_flushed_internal = 0;

        // C 行 143-153: 更新 textrow 度量
        if self.is_empty() {
            self.textrow_rowheight = region.rowheight;
            self.textrow_gap = region.gap;
            self.textrow_gapblank = region.gapblank;
        } else {
            if region.rowheight > self.textrow_rowheight {
                self.textrow_rowheight = region.rowheight;
            }
            if region.gap > self.textrow_gap {
                self.textrow_gap = region.gap;
            }
            if region.gapblank > self.textrow_gapblank {
                self.textrow_gapblank = region.gapblank;
            }
        }

        // C 行 154-155
        self.bgcolor = region.bgcolor as i32;
        self.just = just_flags;

        // C 行 156-165: 首次吸收 MASTERINFO 的 mandatory_region_gap
        let carry = if self.mandatory_region_gap < 0 {
            self.mandatory_region_gap = mandatory_region_gap_carry;
            self.page_region_gap_in = page_region_gap_in_carry;
            MasterGapCarry::Absorbed
        } else {
            MasterGapCarry::NotChanged
        };

        // C 行 170-176: 计算 rh / th，更新 rhmax / thmax
        let rh = region.rowbase - region.r1 + 1;
        if rh > self.rhmax {
            self.rhmax = rh;
        }
        let th = rh + (region.r2 - region.rowbase);
        if th > self.thmax {
            self.thmax = th;
        }

        let region_width = (region.c2 - region.c1 + 1).max(0) as u32;
        let _region_height = (region.r2 - region.r1 + 1).max(0);
        let bpp = region.format.bytes_per_pixel();

        if self.is_empty() {
            // C 行 185-257: 首次 add（width==0 路径）
            self.base = rh - 1;
            let new_height = th.max(1) as u32;
            let new_width = region_width;
            let bw = (new_width as usize) * bpp;
            let total = bw * (new_height as usize);
            let mut pixels = vec![255u8; total];
            // 逐行 memcpy region 到 (base + (i - rowbase)) 行
            for i in region.r1..=region.r2 {
                let dst_row = self.base + (i - region.rowbase);
                if dst_row < 0 || (dst_row as u32) >= new_height {
                    continue;
                }
                let dst_offset = (dst_row as usize) * bw;
                let src_offset = (i as usize) * (region.src_full_width as usize) * bpp
                    + (region.c1 as usize) * bpp;
                let copy_len = bw;
                if src_offset + copy_len <= region.pixels.len()
                    && dst_offset + copy_len <= pixels.len()
                {
                    pixels[dst_offset..dst_offset + copy_len]
                        .copy_from_slice(&region.pixels[src_offset..src_offset + copy_len]);
                }
            }
            let bmp = Bitmap::from_raw(new_width, new_height, 1.0, region.format, pixels)?;
            self.bitmap = Some(bmp);

            // C 行 225-232: 拷贝 hyphen 信息 & 相对调整
            self.hyphen = region.hyphen;
            if self.ends_in_hyphen() {
                self.hyphen.r1 += self.base - region.rowbase;
                self.hyphen.r2 += self.base - region.rowbase;
                self.hyphen.ch -= region.c1;
                self.hyphen.c2 -= region.c1;
            }

            // C 行 233-255: 添加 wrectmap
            let mut wrmap = WRectMap::new();
            wrmap.srcpageno = region.pageno;
            wrmap.srcwidth = region.src_full_width as i32;
            wrmap.srcheight = region.src_full_height as i32;
            wrmap.srcdpiw = region.dpi;
            wrmap.srcdpih = region.dpi;
            wrmap.srcrot = region.rotdeg;
            wrmap.coords[0] = (region.c1 as f64, region.r1 as f64);
            wrmap.coords[1] = (0.0, (self.base + (region.r1 - region.rowbase)) as f64);
            wrmap.coords[2] = (
                (region.c2 - region.c1 + 1) as f64,
                (region.r2 - region.r1 + 1) as f64,
            );
            self.wrectmaps.add(wrmap);
            return Ok(carry);
        }

        // C 行 258-383: 拼接路径（width > 0）
        // is_empty() == false 已保证 bitmap.is_some()，但 clippy expect_used 下走 match 模式
        let cur_bmp = match self.bitmap.as_ref() {
            Some(b) => b,
            None => {
                return Err(BitmapError::SizeOverflow {
                    width: 0,
                    height: 0,
                    format: region.format,
                })
            }
        };
        let width0 = cur_bmp.width;
        let cur_height = cur_bmp.height;
        let cur_base = self.base;

        let new_base = if rh > cur_base { rh - 1 } else { cur_base };
        let h2 = if region.r2 - region.rowbase > (cur_height as i32) - 1 - cur_base {
            region.r2 - region.rowbase
        } else {
            (cur_height as i32) - 1 - cur_base
        };
        let new_height = (new_base + h2 + 1).max(1) as u32;
        let new_width = width0 + (colgap as u32) + region_width;

        let bw_new = (new_width as usize) * bpp;
        let total = bw_new * (new_height as usize);
        let mut tmp = vec![255u8; total];

        // C 行 284-290: new_base != cur_base 时调整既有 wrectmaps 的 y 坐标
        // (LTR 时 x 不变；RTL 时 x 右移 (new_width - width0) - region_width = colgap，
        //  C 是 (tmp->width - 1 - wrapbmp->bmp.width) = new_width - 1 - width0)
        let dy = (new_base - cur_base) as f64;
        let rtl_x_shift = (new_width as f64) - 1.0 - (width0 as f64);
        if new_base != cur_base {
            for m in &mut self.wrectmaps.items {
                m.coords[1].1 += dy;
                if !src_left_to_right {
                    m.coords[1].0 += rtl_x_shift;
                }
            }
        } else if !src_left_to_right {
            // C 仅在 new_base!=cur_base 时调整 x。LTR 时新拼接落到右侧；
            // 注意 C 代码 (行 287-290) 把 x 调整放在 new_base!=cur_base 分支内，
            // 故 RTL 且 new_base==cur_base 时 C 端不调整。这里严格按 C 不调整。
        }

        // C 行 291-298: 把旧 wrap bitmap 拷到 tmp 的 (i + new_base - cur_base) 行
        let bw_old_bytes = (width0 as usize) * bpp;
        let x_offset_old = if src_left_to_right {
            0usize
        } else {
            ((new_width - width0) as usize) * bpp
        };
        for i in 0..(cur_height as i32) {
            let dst_y = i + new_base - cur_base;
            if dst_y < 0 || (dst_y as u32) >= new_height {
                continue;
            }
            let dst_offset = (dst_y as usize) * bw_new + x_offset_old;
            let src_offset = (i as usize) * bw_old_bytes;
            if src_offset + bw_old_bytes <= cur_bmp.pixels.len()
                && dst_offset + bw_old_bytes <= tmp.len()
            {
                tmp[dst_offset..dst_offset + bw_old_bytes]
                    .copy_from_slice(&cur_bmp.pixels[src_offset..src_offset + bw_old_bytes]);
            }
        }

        // C 行 299-316: 把 region 拷到 tmp 的 (i + new_base - rowbase) 行
        let region_bytes = (region_width as usize) * bpp;
        let x_offset_new = if src_left_to_right {
            ((width0 + colgap as u32) as usize) * bpp
        } else {
            0usize
        };
        for i in region.r1..=region.r2 {
            let dst_y = i + new_base - region.rowbase;
            if dst_y < 0 || (dst_y as u32) >= new_height {
                continue;
            }
            let dst_offset = (dst_y as usize) * bw_new + x_offset_new;
            let src_offset =
                (i as usize) * (region.src_full_width as usize) * bpp + (region.c1 as usize) * bpp;
            if src_offset + region_bytes <= region.pixels.len()
                && dst_offset + region_bytes <= tmp.len()
            {
                tmp[dst_offset..dst_offset + region_bytes]
                    .copy_from_slice(&region.pixels[src_offset..src_offset + region_bytes]);
            }
        }

        // C 行 317-341: 添加新 wrectmap
        let mut wrmap = WRectMap::new();
        wrmap.srcpageno = region.pageno;
        wrmap.srcwidth = region.src_full_width as i32;
        wrmap.srcheight = region.src_full_height as i32;
        wrmap.srcdpiw = region.dpi;
        wrmap.srcdpih = region.dpi;
        wrmap.srcrot = region.rotdeg;
        wrmap.coords[0] = (region.c1 as f64, region.r1 as f64);
        let x1 = if src_left_to_right {
            (width0 as i32 + colgap) as f64
        } else {
            0.0
        };
        wrmap.coords[1] = (x1, (region.r1 + new_base - region.rowbase) as f64);
        wrmap.coords[2] = (
            (region.c2 - region.c1 + 1) as f64,
            (region.r2 - region.r1 + 1) as f64,
        );
        self.wrectmaps.add(wrmap);

        // C 行 342: bmp_copy(&wrapbmp->bmp, tmp) — 替换 self.bitmap
        let new_bmp = Bitmap::from_raw(new_width, new_height, 1.0, region.format, tmp)?;
        self.bitmap = Some(new_bmp);

        // C 行 352-368: 拷贝 region 的 hyphen 信息，调整坐标
        self.hyphen = region.hyphen;
        if self.ends_in_hyphen() {
            self.hyphen.r1 += new_base - region.rowbase;
            self.hyphen.r2 += new_base - region.rowbase;
            if src_left_to_right {
                self.hyphen.ch += (width0 as i32) + colgap - region.c1;
                self.hyphen.c2 += (width0 as i32) + colgap - region.c1;
            } else {
                self.hyphen.ch -= region.c1;
                self.hyphen.c2 -= region.c1;
            }
        }

        self.base = new_base;

        Ok(carry)
    }

    /// 擦除 wrap 尾部 hyphen 的像素。对应 C `wrapbmp_hyphen_erase`（`wrapbmp.c:579-648`）。
    ///
    /// `ch < 0` 时直接返回（无 hyphen）。否则用 hyphen 信息切割 wrap bitmap，
    /// 把 hyphen 占用的水平区间清白，同步调整 wrectmaps 的坐标。
    pub fn hyphen_erase(&mut self, src_left_to_right: bool) -> Result<(), BitmapError> {
        if !self.ends_in_hyphen() {
            return Ok(());
        }
        let bmp_ref = match self.bitmap.as_ref() {
            Some(b) => b,
            None => return Ok(()),
        };
        let bpp = bmp_ref.format.bytes_per_pixel();
        let cur_w = bmp_ref.width as i32;
        let cur_h = bmp_ref.height;
        let (new_width_i, c0, c1, c2) = if src_left_to_right {
            // C 行 599-604
            let new_w = self.hyphen.c2 + 1;
            (new_w, 0, self.hyphen.ch, new_w - 1)
        } else {
            // C 行 605-611
            let new_w = cur_w - self.hyphen.c2;
            (new_w, self.hyphen.c2, 0, self.hyphen.ch - self.hyphen.c2)
        };
        if new_width_i <= 0 {
            // 异常路径：直接 reset hyphen 不动 bitmap
            self.hyphen = HyphenInfo::none();
            return Ok(());
        }
        let new_width = new_width_i as u32;
        let bw_new = (new_width as usize) * bpp;
        let total = bw_new * (cur_h as usize);
        let mut new_pixels = vec![255u8; total];

        // C 行 618-628: 调整 wrectmaps（RTL 时 x 减 c0；最后一个 entry 的 width 还要减 c0）
        let last_idx = self.wrectmaps.items.len().saturating_sub(1);
        for (i, m) in self.wrectmaps.items.iter_mut().enumerate() {
            if !src_left_to_right {
                m.coords[1].0 -= c0 as f64;
            }
            if i == last_idx {
                m.coords[2].0 -= c0 as f64;
                if !src_left_to_right {
                    m.coords[0].0 += c0 as f64;
                }
            }
        }

        // C 行 629-630: 逐行从旧 bitmap 拷到新（按 c0 偏移）
        let old_bw = (cur_w as usize) * bpp;
        let copy_bw = bw_new;
        for i in 0..(cur_h as usize) {
            let src_offset = i * old_bw + (c0 as usize) * bpp;
            let dst_offset = i * bw_new;
            if src_offset + copy_bw <= bmp_ref.pixels.len()
                && dst_offset + copy_bw <= new_pixels.len()
            {
                new_pixels[dst_offset..dst_offset + copy_bw]
                    .copy_from_slice(&bmp_ref.pixels[src_offset..src_offset + copy_bw]);
            }
        }

        // C 行 631-634: 在 hyphen 行段 (r1..=r2) 上的 [c1, c2] 区间清白
        let erase_bw = (c2 - c1 + 1).max(0) as usize * bpp;
        if erase_bw > 0 {
            for r in self.hyphen.r1..=self.hyphen.r2 {
                if r < 0 || (r as u32) >= cur_h {
                    continue;
                }
                let row_start = (r as usize) * bw_new + (c1 as usize) * bpp;
                let row_end = row_start + erase_bw;
                if row_end <= new_pixels.len() {
                    new_pixels[row_start..row_end].fill(255);
                }
            }
        }

        let new_bmp = Bitmap::from_raw(new_width, cur_h, 1.0, bmp_ref.format, new_pixels)?;
        self.bitmap = Some(new_bmp);
        self.hyphen = HyphenInfo::none();
        Ok(())
    }

    /// 把 wrap 缓冲区的累积内容转为 [`FlushedLine`] 返回给调用方。
    ///
    /// **设计**：替代 C `wrapbmp_flush`（`wrapbmp.c:386-576`）中直接回调 `bmpregion_add`
    /// 的部分。本函数只产出 `FlushedLine`，调用方（Step 8.2/8.3 的 ConvertContext）
    /// 在拿到后自行 `calc_bbox` + 注入 master canvas。
    ///
    /// # 参数
    ///
    /// - `text_wrap`：对应 C `k2settings->text_wrap`。`false` 时直接返回 `None`（C 行 403）
    /// - `allow_full_justification`：是否允许 full-justify（C 行 468-471）。
    ///   `false` 时把 `just` 的 0x30 bits 改为 `0x20`（强制 0x20，即 ragged_right）
    ///
    /// # 返回
    ///
    /// - `Ok(Some(line))`：成功 flush 一行
    /// - `Ok(None)`：text_wrap=false / 已 flush / width<=0
    pub fn flush(
        &mut self,
        text_wrap: bool,
        allow_full_justification: bool,
    ) -> Result<Option<FlushedLine>, BitmapError> {
        // C 行 403-404
        if !text_wrap {
            return Ok(None);
        }
        // C 行 406
        if self.just_flushed_internal != 0 {
            return Ok(None);
        }
        // C 行 412-416
        if self.width() == 0 {
            self.just_flushed_internal = 1;
            return Ok(None);
        }
        let bmp = self
            .bitmap
            .as_ref()
            .ok_or(BitmapError::SizeOverflow {
                width: 0,
                height: 0,
                format: PixelFormat::Gray8,
            })?
            .clone();

        // C 行 462-466: 还原 textrow 度量到 region.bbox
        let rowheight = self.textrow_rowheight;
        let gap = self.textrow_rowheight - (bmp.height as i32); // C: rowheight - (r2-r1+1)
        let gapblank = self.textrow_gapblank;

        // C 行 468-471: just 计算
        let just_flags = if allow_full_justification {
            self.just
        } else {
            (self.just & 0xcf) | 0x20
        };

        let bgcolor = if self.bgcolor < 0 {
            255
        } else {
            self.bgcolor.clamp(0, 255) as u8
        };

        let line = FlushedLine {
            bitmap: bmp,
            base: self.base,
            rowheight,
            gap,
            gapblank,
            bgcolor,
            just_flags,
            wrectmaps: self.wrectmaps.clone(),
            mandatory_region_gap: self.mandatory_region_gap,
            page_region_gap_in: self.page_region_gap_in,
        };

        // C 行 571-575: wrectmaps_clear + wrapbmp_reset
        self.reset();
        // reset 不动 bitmap 的容量，但 width=0；下一次 add_word 会重 alloc
        Ok(Some(line))
    }
}

impl Default for WrapState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// MasterGapCarry - add_word 返回值，告诉调用方是否吸收了 MASTERINFO gap 字段
// ---------------------------------------------------------------------------

/// `WrapState::add_word` 的返回值，描述 MASTERINFO 的 mandatory_region_gap /
/// page_region_gap_in 是否被吸收。
///
/// 对应 C 行 156-161：首次 add 时把 MASTERINFO 的两个字段拷到 wrapbmp，并清零
/// MASTERINFO 的对应字段（调用方负责）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterGapCarry {
    /// wrap 首次吸收，调用方应清零 MASTERINFO 的 mandatory_region_gap / page_region_gap_in。
    Absorbed,
    /// wrap 已经有 carry，本次 add_word 未触发吸收。
    NotChanged,
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn region<'a>(
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

    // --- HyphenInfo ---

    #[test]
    fn hyphen_none_is_not_hyphen() {
        let h = HyphenInfo::none();
        assert!(!h.is_hyphen());
        assert_eq!(h.ch, -1);
    }

    #[test]
    fn hyphen_with_ch_zero_or_more_is_hyphen() {
        let h = HyphenInfo {
            ch: 0,
            c2: 5,
            r1: 0,
            r2: 2,
        };
        assert!(h.is_hyphen());
    }

    // --- WRectMap ---

    #[test]
    fn wrectmap_inside_basic() {
        let mut m = WRectMap::new();
        m.coords[1] = (10.0, 20.0);
        m.coords[2] = (30.0, 40.0);
        assert!(m.inside(15.0, 30.0));
        assert!(!m.inside(5.0, 30.0)); // 左外
        assert!(!m.inside(50.0, 30.0)); // 右外
        assert!(m.inside(10.0, 20.0)); // 左上角
        assert!(m.inside(40.0, 60.0)); // 右下角
    }

    // --- WRectMaps ---

    #[test]
    fn wrectmaps_empty_default() {
        let m = WRectMaps::new();
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn wrectmaps_add_clear() {
        let mut m = WRectMaps::new();
        m.add(WRectMap::new());
        m.add(WRectMap::new());
        assert_eq!(m.len(), 2);
        m.clear();
        assert!(m.is_empty());
    }

    #[test]
    fn wrectmaps_sort_horizontally() {
        let mut m = WRectMaps::new();
        for x in [30.0, 10.0, 20.0] {
            let mut e = WRectMap::new();
            e.coords[1] = (x, 0.0);
            m.add(e);
        }
        m.sort_horizontally();
        assert_eq!(m.items[0].coords[1].0, 10.0);
        assert_eq!(m.items[1].coords[1].0, 20.0);
        assert_eq!(m.items[2].coords[1].0, 30.0);
    }

    #[test]
    fn wrectmaps_scale_wrapbmp_coords() {
        let mut m = WRectMaps::new();
        let mut e = WRectMap::new();
        e.srcwidth = 100;
        e.srcheight = 200;
        e.srcdpiw = 300.0;
        e.srcdpih = 300.0;
        e.coords[0] = (10.0, 20.0);
        e.coords[1] = (30.0, 40.0);
        e.coords[2] = (50.0, 60.0);
        m.add(e);
        m.scale_wrapbmp_coords(2.0, 0.5);
        // round((100*2)+0.5) = 200; round((200*0.5)+0.5) = 100
        assert_eq!(m.items[0].srcwidth, 200);
        assert_eq!(m.items[0].srcheight, 100);
    }

    // --- WrapState 基础 ---

    #[test]
    fn new_defaults_match_c_wrapbmp_init() {
        let w = WrapState::new();
        // C wrapbmp_init: bgcolor=-1, just=0x8f, just_flushed_internal=0
        assert_eq!(w.bgcolor, -1);
        assert_eq!(w.just, 0x8f);
        assert_eq!(w.just_flushed_internal, 1); // reset 后是 1
                                                // C wrapbmp_reset: maxgap=2, rhmax=-1, thmax=-1
        assert_eq!(w.maxgap, 2);
        assert_eq!(w.rhmax, -1);
        assert_eq!(w.thmax, -1);
        assert!(!w.ends_in_hyphen());
        assert!(w.is_empty());
        assert!(w.wrectmaps.is_empty());
    }

    #[test]
    fn set_color_gray_creates_gray_bitmap() {
        let mut w = WrapState::new();
        w.set_color(false);
        assert!(w.bitmap.is_some());
        assert_eq!(w.bitmap.as_ref().unwrap().format, PixelFormat::Gray8);
    }

    #[test]
    fn set_color_rgb_creates_rgb_bitmap() {
        let mut w = WrapState::new();
        w.set_color(true);
        assert!(w.bitmap.is_some());
        assert_eq!(w.bitmap.as_ref().unwrap().format, PixelFormat::Rgb8);
    }

    #[test]
    fn set_maxgap_updates_field() {
        let mut w = WrapState::new();
        w.set_maxgap(5);
        assert_eq!(w.maxgap, 5);
    }

    #[test]
    fn width_height_zero_initially() {
        let w = WrapState::new();
        assert_eq!(w.width(), 0);
        assert_eq!(w.height(), 0);
    }

    #[test]
    fn ends_in_hyphen_reflects_hyphen_info() {
        let mut w = WrapState::new();
        assert!(!w.ends_in_hyphen());
        w.hyphen.ch = 5;
        assert!(w.ends_in_hyphen());
    }

    #[test]
    fn remaining_no_hyphen() {
        let mut w = WrapState::new();
        w.set_color(false);
        // 模拟 width=300
        let bmp = Bitmap::from_raw(300, 10, 1.0, PixelFormat::Gray8, vec![255; 3000]).unwrap();
        w.bitmap = Some(bmp);
        // maxpix = 4.0 * 300 = 1200; 剩 1200 - 300 = 900
        assert_eq!(w.remaining(4.0, 300.0, true), 900);
    }

    #[test]
    fn remaining_with_hyphen_ltr() {
        let mut w = WrapState::new();
        let bmp = Bitmap::from_raw(300, 10, 1.0, PixelFormat::Gray8, vec![255; 3000]).unwrap();
        w.bitmap = Some(bmp);
        w.hyphen.ch = 280;
        w.hyphen.c2 = 290;
        // hyphen c2+1 = 291 占用, 剩 maxpix(1200) - 291 = 909
        assert_eq!(w.remaining(4.0, 300.0, true), 909);
    }

    #[test]
    fn remaining_with_hyphen_rtl() {
        let mut w = WrapState::new();
        let bmp = Bitmap::from_raw(300, 10, 1.0, PixelFormat::Gray8, vec![255; 3000]).unwrap();
        w.bitmap = Some(bmp);
        w.hyphen.ch = 5;
        w.hyphen.c2 = 10;
        // 占用 = 300 - 10 = 290; 剩 1200 - 290 = 910
        assert_eq!(w.remaining(4.0, 300.0, false), 910);
    }

    // --- WrapState::reset ---

    #[test]
    fn reset_clears_accumulators() {
        let mut w = WrapState::new();
        w.rhmax = 50;
        w.thmax = 60;
        w.base = 30;
        w.wrectmaps.add(WRectMap::new());
        w.hyphen = HyphenInfo {
            ch: 1,
            c2: 2,
            r1: 0,
            r2: 0,
        };
        w.reset();
        assert_eq!(w.rhmax, -1);
        assert_eq!(w.thmax, -1);
        assert_eq!(w.base, 0);
        assert!(w.wrectmaps.is_empty());
        assert!(!w.ends_in_hyphen());
        assert_eq!(w.just_flushed_internal, 1);
    }

    // --- WrapState::add_word 首次路径 ---

    #[test]
    fn add_word_first_path_sets_bitmap() {
        let mut w = WrapState::new();
        w.set_color(false);
        // 5x4 灰度源 bitmap，内容 0..20
        let pixels: Vec<u8> = (0u8..20).collect();
        let reg = region(&pixels, 5, 4, 1, 3, 0, 2, 1);
        let carry = w
            .add_word(&reg, 0, 0x88, true, 5, 0.25)
            .expect("add_word ok");
        assert_eq!(carry, MasterGapCarry::Absorbed);
        // region 宽 = c2-c1+1 = 3，高 = r2-r1+1 = 3 (实际 th 计算)
        // rh = rowbase-r1+1 = 2, th = rh + (r2-rowbase) = 2 + 1 = 3
        assert_eq!(w.width(), 3);
        assert_eq!(w.height(), 3);
        assert_eq!(w.base, 1);
        assert_eq!(w.rhmax, 2);
        assert_eq!(w.thmax, 3);
        assert_eq!(w.bgcolor, 255);
        assert_eq!(w.just, 0x88);
        assert_eq!(w.mandatory_region_gap, 5);
        assert!((w.page_region_gap_in - 0.25).abs() < 1e-9);
        assert_eq!(w.wrectmaps.len(), 1);
        assert_eq!(w.just_flushed_internal, 0);
    }

    #[test]
    fn add_word_second_call_does_not_re_absorb_carry() {
        let mut w = WrapState::new();
        w.set_color(false);
        let pixels: Vec<u8> = vec![100; 20];
        let reg = region(&pixels, 5, 4, 0, 2, 0, 2, 1);
        w.add_word(&reg, 0, 0x88, true, 7, 0.5).unwrap();
        let reg2 = region(&pixels, 5, 4, 0, 2, 0, 2, 1);
        let carry = w.add_word(&reg2, 1, 0x88, true, 99, 9.9).unwrap();
        assert_eq!(carry, MasterGapCarry::NotChanged);
        assert_eq!(w.mandatory_region_gap, 7); // 不变
    }

    // --- WrapState::add_word 拼接路径 ---

    #[test]
    fn add_word_second_concat_ltr() {
        let mut w = WrapState::new();
        w.set_color(false);
        let pixels: Vec<u8> = vec![100; 20]; // 5x4
        let reg = region(&pixels, 5, 4, 0, 2, 0, 2, 1);
        w.add_word(&reg, 0, 0x88, true, 0, 0.0).unwrap();
        // 第二次 add 同尺寸 region，colgap=2
        let reg2 = region(&pixels, 5, 4, 0, 2, 0, 2, 1);
        w.add_word(&reg2, 2, 0x88, true, 0, 0.0).unwrap();
        // 拼接后 width = 3 + 2 + 3 = 8，高度仍 = 3
        assert_eq!(w.width(), 8);
        assert_eq!(w.height(), 3);
        assert_eq!(w.wrectmaps.len(), 2);
        // 新 wrectmap x1 = width0 + colgap = 3 + 2 = 5
        assert_eq!(w.wrectmaps.items[1].coords[1].0, 5.0);
    }

    #[test]
    fn add_word_concat_grows_height_when_new_rh_larger() {
        let mut w = WrapState::new();
        w.set_color(false);
        // 第一次 add: rh=2, th=3
        let p1: Vec<u8> = vec![100; 20];
        let r1 = region(&p1, 5, 4, 0, 2, 0, 2, 1);
        w.add_word(&r1, 0, 0x88, true, 0, 0.0).unwrap();
        // 第二次 add: rh=4, th=5（更高）
        let p2: Vec<u8> = vec![100; 30]; // 5x6
        let r2 = AddRegion {
            pixels: &p2,
            src_full_width: 5,
            src_full_height: 6,
            format: PixelFormat::Gray8,
            c1: 0,
            c2: 2,
            r1: 0,
            r2: 4,
            rowbase: 3,
            rowheight: 8,
            gap: 2,
            gapblank: 1,
            bgcolor: 255,
            pageno: 0,
            dpi: 300.0,
            rotdeg: 0,
            hyphen: HyphenInfo::none(),
        };
        w.add_word(&r2, 1, 0x88, true, 0, 0.0).unwrap();
        // new_base = max(cur_base=1, rh-1=3) = 3
        // h2 = max(r2-rowbase=1, height-1-base = 3-1-1=1) = 1
        // new_height = new_base + h2 + 1 = 3 + 1 + 1 = 5
        assert_eq!(w.base, 3);
        assert_eq!(w.height(), 5);
        // 旧 wrectmap 的 y 被偏移 (new_base - cur_base) = 2
    }

    // --- WrapState::flush ---

    #[test]
    fn flush_returns_none_when_text_wrap_off() {
        let mut w = WrapState::new();
        w.set_color(false);
        let p: Vec<u8> = vec![100; 20];
        let r = region(&p, 5, 4, 0, 2, 0, 2, 1);
        w.add_word(&r, 0, 0x88, true, 0, 0.0).unwrap();
        let res = w.flush(false, true).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn flush_returns_none_when_empty() {
        let mut w = WrapState::new();
        let res = w.flush(true, true).unwrap();
        assert!(res.is_none());
        assert_eq!(w.just_flushed_internal, 1);
    }

    #[test]
    fn flush_produces_line_and_resets() {
        let mut w = WrapState::new();
        w.set_color(false);
        let p: Vec<u8> = vec![100; 20];
        let r = region(&p, 5, 4, 0, 2, 0, 2, 1);
        w.add_word(&r, 0, 0x88, true, 5, 0.5).unwrap();
        let line = w.flush(true, true).unwrap().expect("Some line");
        assert_eq!(line.bitmap.width, 3);
        assert_eq!(line.bitmap.height, 3);
        assert_eq!(line.base, 1);
        assert_eq!(line.mandatory_region_gap, 5);
        assert!((line.page_region_gap_in - 0.5).abs() < 1e-9);
        assert_eq!(line.just_flags, 0x88);
        // 已 reset
        assert!(w.is_empty());
        assert_eq!(w.just_flushed_internal, 1);
    }

    #[test]
    fn flush_disables_full_justification_when_not_allowed() {
        let mut w = WrapState::new();
        w.set_color(false);
        let p: Vec<u8> = vec![100; 20];
        let r = region(&p, 5, 4, 0, 2, 0, 2, 1);
        // just=0xff (含 full justify bits)
        w.add_word(&r, 0, 0xff, true, 0, 0.0).unwrap();
        let line = w.flush(true, false).unwrap().unwrap();
        // (0xff & 0xcf) | 0x20 = 0xcf | 0x20 = 0xef
        assert_eq!(line.just_flags, 0xef);
    }

    #[test]
    fn flush_twice_second_returns_none() {
        let mut w = WrapState::new();
        w.set_color(false);
        let p: Vec<u8> = vec![100; 20];
        let r = region(&p, 5, 4, 0, 2, 0, 2, 1);
        w.add_word(&r, 0, 0x88, true, 0, 0.0).unwrap();
        let _ = w.flush(true, true).unwrap();
        let res = w.flush(true, true).unwrap();
        assert!(res.is_none());
    }

    // --- hyphen_erase ---

    #[test]
    fn hyphen_erase_noop_when_no_hyphen() {
        let mut w = WrapState::new();
        let bmp = Bitmap::from_raw(10, 5, 1.0, PixelFormat::Gray8, vec![128; 50]).unwrap();
        w.bitmap = Some(bmp);
        w.hyphen_erase(true).unwrap();
        assert_eq!(w.bitmap.as_ref().unwrap().width, 10);
        assert!(w.bitmap.as_ref().unwrap().pixels.iter().all(|&p| p == 128));
    }

    #[test]
    fn hyphen_erase_ltr_trims_right_and_whites_segment() {
        let mut w = WrapState::new();
        // 10x4 bitmap，全 100
        let mut bmp = Bitmap::from_raw(10, 4, 1.0, PixelFormat::Gray8, vec![100; 40]).unwrap();
        // 在 ch=6..c2=8 行 1..=2 处放 hyphen 像素 200
        for r in 1..=2 {
            for c in 6..=8 {
                bmp.pixels[(r * 10 + c) as usize] = 200;
            }
        }
        w.bitmap = Some(bmp);
        w.hyphen = HyphenInfo {
            ch: 6,
            c2: 8,
            r1: 1,
            r2: 2,
        };
        // 添加最近的 wrectmap（last_idx = 0）
        w.wrectmaps.add(WRectMap::new());
        w.hyphen_erase(true).unwrap();
        let new_bmp = w.bitmap.as_ref().unwrap();
        // new_width = c2+1 = 9
        assert_eq!(new_bmp.width, 9);
        // 在 hyphen 行段被擦白：[ch=6 .. c2=8] 之间应是 255
        for r in 1..=2 {
            for c in 6..=8 {
                assert_eq!(new_bmp.pixels[(r * 9 + c) as usize], 255);
            }
        }
        // 其他位置应保留 100
        assert_eq!(new_bmp.pixels[0], 100);
        // hyphen 已清空
        assert!(!w.ends_in_hyphen());
    }
}
