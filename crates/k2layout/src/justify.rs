//! `justify` - 段落对齐（justification）核心。
//!
//! Step 8.2 / M6 落地 C 版 `bmp_fully_justify`（`k2pdfoptlib/k2master.c:2031-2124`）的
//! Rust 移植，并配套 [`JustFlags`] / [`JustifyMode`] 编解码工具。
//!
//! # JustFlags 8-bit 编码（C 版约定）
//!
//! 来源：`k2pdfopt.h:217-219` + `k2master.c:524-546` decode 路径。
//!
//! | 位段 | 含义 |
//! |------|------|
//! | bits 0-1（`& 0x03`） | 水平对齐 mandate：0=mandatory left, 1=center, 2=mandatory right, 3=use user `dst_justify` |
//! | bits 2-3（`& 0x0c`） | 推荐水平对齐：0=left, 8=right（仅当 mandate==3 && user==-1 时使用） |
//! | bits 4-5（`& 0x30`） | full-justify mandate：0=use user `dst_fulljustify`, 0x10=mandatory full, 0x20=mandatory no-full |
//! | bits 6-7（`& 0xc0`） | 推荐 full-justify：0=no, 0x40=yes（仅当 mandate==0 && user==-1 时使用） |
//!
//! C 默认 `just=0x8f`：bits 0,1,2,3,7 全设，含义"use user for both H + recommend right + recommend full"。
//!
//! `wrapbmp_flush` 在 `allow_full_justification=false` 时把 just 改写为
//! `(just & 0xcf) | 0x20`：屏蔽 bits 4-5 + 设为 mandatory no-full。
//!
//! # 算法
//!
//! [`fully_justify_with_gaps`] 把一张 src bitmap 按已知 word gap 位置 spread 到
//! jbmpwidth 宽度的输出 bitmap。`just=Left` 时 piece 紧贴左侧（spread 后右侧留白），
//! `Center` 时整体居中，`Right` 时紧贴右侧。
//!
//! 当 word 数 = 1（无 gap）时不 spread，按 just 决定整段位置。
//!
//! # Step 8.2 与 Step 8.3 的边界
//!
//! 本步不接入 [`crate::words::one_row_find_textwords`]（需要 WordSettings + dbase 的
//! 完整上下文）。Step 8.3 串联 ConvertContext::add_bitmap 时再做高层包装。

use crate::master::wrap_state::{WRectMap, WRectMaps};
use k2types::{Bitmap, BitmapError, PixelFormat};

/// 段落对齐 flags（i32 newtype）。
///
/// 与 C 版 `int justification_flags` 同源 8-bit 编码（详见模块级文档）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JustFlags(pub i32);

impl JustFlags {
    /// C 版 `wrapbmp_init` 默认值 `0x8f`。
    ///
    /// 含义：水平 mandate=3（use user）+ 水平 recommend=right（0xc）+ full mandate=0（use user）+ full recommend=yes（0x80）。
    pub const DEFAULT: Self = Self(0x8f);

    /// 取原始 i32 值。
    #[must_use]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// bits 0-1：水平对齐 mandate。0/1/2 = left/center/right，3 = use user。
    #[must_use]
    pub const fn h_mandate(self) -> i32 {
        self.0 & 0x03
    }

    /// bits 2-3：水平对齐 recommend（仅在 mandate=3 && user=-1 时使用）。
    #[must_use]
    pub const fn h_recommend(self) -> i32 {
        self.0 & 0x0c
    }

    /// bits 4-5：full-justify mandate。0 = use user, 0x10 = mandatory full, 0x20 = mandatory no-full。
    #[must_use]
    pub const fn f_mandate(self) -> i32 {
        self.0 & 0x30
    }

    /// bits 6-7：full-justify recommend（仅在 f_mandate=0 && user=-1 时使用）。
    #[must_use]
    pub const fn f_recommend(self) -> i32 {
        self.0 & 0xc0
    }

    /// 屏蔽 full-justify bits 设为"强制 no-full"（`(self & 0xcf) | 0x20`）。
    ///
    /// 对应 C `wrapbmp_flush` 在 `allow_full_justification=false` 时的位运算
    /// （`wrapbmp.c:469-471`）。
    #[must_use]
    pub const fn disable_full(self) -> Self {
        Self((self.0 & 0xcf) | 0x20)
    }
}

impl Default for JustFlags {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl From<i32> for JustFlags {
    fn from(v: i32) -> Self {
        Self(v)
    }
}

impl From<JustFlags> for i32 {
    fn from(v: JustFlags) -> Self {
        v.0
    }
}

/// 解码后的水平对齐 mode。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JustifyMode {
    /// 左对齐
    Left,
    /// 居中
    Center,
    /// 右对齐
    Right,
}

/// 解码水平对齐。
///
/// 对应 C 版 `k2master.c:524-535` decode 路径。
///
/// # 参数
///
/// - `flags`：段落 [`JustFlags`]
/// - `user_dst_justify`：用户设置 `k2settings->dst_justify`，0=left, 1=center, 2=right, -1=auto
#[must_use]
pub fn classify_horizontal(flags: JustFlags, user_dst_justify: i32) -> JustifyMode {
    let m = flags.h_mandate();
    let rec = flags.h_recommend();
    // C 行 524-528: mandatory left || (use_user && (user==0 || (user<0 && rec==0)))
    if m == 0 || (m == 3 && (user_dst_justify == 0 || (user_dst_justify < 0 && rec == 0))) {
        JustifyMode::Left
    } else if m == 2 || (m == 3 && (user_dst_justify == 2 || (user_dst_justify < 0 && rec == 8))) {
        // C 行 529-533: mandatory right || (use_user && (user==2 || (user<0 && rec==8)))
        JustifyMode::Right
    } else {
        // C 行 534-535: 其余 → center
        JustifyMode::Center
    }
}

/// 解码是否应做 full-justify（行内字间距 spread）。
///
/// 对应 C 版 `k2master.c:541-545` decode 路径。
///
/// # 参数
///
/// - `flags`：段落 [`JustFlags`]
/// - `user_dst_fulljustify`：用户设置 `k2settings->dst_fulljustify`，0=off, 1=on, -1=auto
#[must_use]
pub fn should_full_justify(flags: JustFlags, user_dst_fulljustify: i32) -> bool {
    let fm = flags.f_mandate();
    let f_rec = flags.f_recommend();
    // C 行 541-545:
    //   ((fm==0x10) || (fm==0 && (user==1 || (user<0 && f_rec==0x40))))
    fm == 0x10
        || (fm == 0 && (user_dst_fulljustify == 1 || (user_dst_fulljustify < 0 && f_rec == 0x40)))
}

/// 用预计算的 word gap 位置把一张 src bitmap spread 到 `jbmpwidth` 宽度的输出 bitmap。
///
/// 严格 1:1 复刻 C 版 `bmp_fully_justify`（`k2pdfoptlib/k2master.c:2031-2124`）的几何变换部分。
/// 不调用 `find_word_gaps_using_textrow`——gap 位置由调用方传入（Step 8.3 接入
/// [`crate::words::one_row_find_textwords`] 后做高层包装）。
///
/// # 参数
///
/// - `src`：源 bitmap（一行文字的累积像素）
/// - `gap_pos`：每个 word gap 的左缘像素列（即 word i 的 c2+1）。空切片表示无 gap（单 word）。
/// - `jbmpwidth`：输出 bitmap 宽度
/// - `just`：水平对齐
/// - `wrectmaps`：可选，传入时算法会修正每个 wrectmap 的 x 位置以反映 spread 后的坐标
///
/// # 输出
///
/// 新分配的 [`Bitmap`]（与 `src` 同 `format` 同 `height` 同 `dpi`）。全白底，src 内容按 piece spread 后落入。
///
/// # 算法
///
/// 1. ng = `gap_pos.len()`
/// 2. `newwidth = ng>0 ? min(src.width * 1.25, jbmpwidth) : src.width`
/// 3. `destx0` 由 `just` 决定：Left=0 / Center=(jbmp-new)/2 / Right=jbmp-new
/// 4. jbmp 全白
/// 5. 对每个 piece i ∈ [0..ng]：
///     - 计算 piece 在 src 的起始列 `sx0[i]` 和宽度 `dx`
///     - 计算 piece 在 jbmp 的起始列 `dx0[i] = destx0 + sx0[i] + (newwidth - src.width) * i / ng`
///     - 逐行 memcpy
/// 6. （可选）调用 [`wrectmaps_add_gap`] 修正 wrectmap
pub fn fully_justify_with_gaps(
    src: &Bitmap,
    gap_pos: &[i32],
    jbmpwidth: u32,
    just: JustifyMode,
    wrectmaps: Option<&mut WRectMaps>,
) -> Result<Bitmap, BitmapError> {
    let ng = gap_pos.len() as i32;
    let bpp = src.format.bytes_per_pixel();

    // C 行 2068-2076: newwidth
    let src_w_i = src.width as i32;
    let newwidth: i32 = if ng > 0 {
        let raw = src_w_i + src_w_i / 4; // 1.25x，C 用 src->width * 1.25
        raw.min(jbmpwidth as i32)
    } else {
        src_w_i
    };

    // C 行 2079-2084: destx0
    let jbmp_w_i = jbmpwidth as i32;
    let destx0: i32 = match just {
        JustifyMode::Center => (jbmp_w_i - newwidth) / 2,
        JustifyMode::Right => jbmp_w_i - newwidth,
        JustifyMode::Left => 0,
    };

    // 输出 bitmap：全白
    let mut jbmp = Bitmap::new(jbmpwidth, src.height, src.dpi, src.format)?;
    let white_byte = white_byte_for(src.format);
    match src.format {
        PixelFormat::Gray8 => jbmp.fill_byte(255),
        PixelFormat::Rgb8 => jbmp.fill_rgb(255, 255, 255),
        PixelFormat::Rgba8 => {
            // 全白 + alpha=255（与 fill_byte 等价但语义清晰）
            jbmp.fill_byte(255);
        }
    }
    let _ = white_byte;

    // C 行 2096-2112: 对每个 piece spread + memcpy
    let mut sx0_vec = Vec::with_capacity((ng + 1) as usize);
    let mut dx0_vec = Vec::with_capacity((ng + 1) as usize);
    for i in 0..=ng {
        // C 行 2103-2104: dx
        let dx: i32 = if i < ng {
            if i > 0 {
                gap_pos[i as usize] - gap_pos[(i - 1) as usize]
            } else {
                gap_pos[0] + 1
            }
        } else {
            // i == ng (last piece)
            if i > 0 {
                src_w_i - (gap_pos[(i - 1) as usize] + 1)
            } else {
                // i==0 && ng==0
                src_w_i
            }
        };

        // C 行 2106: sx0
        let sx0: i32 = if i == 0 {
            0
        } else {
            gap_pos[(i - 1) as usize] + 1
        };

        // C 行 2107: dx0（用 i64 防溢出）
        let spread_offset: i64 = if i == 0 {
            0
        } else {
            (i64::from(newwidth - src_w_i) * i64::from(i)) / i64::from(ng)
        };
        let dx0: i32 = destx0 + sx0 + (spread_offset as i32);

        sx0_vec.push(sx0);
        dx0_vec.push(dx0);

        if dx <= 0 {
            continue;
        }

        // 逐行 memcpy。注意像素是 byte，按 bpp 转换列单位 → byte 单位
        let copy_bytes = (dx as usize) * bpp;
        let src_byte_start = (sx0 as usize) * bpp;
        let dst_byte_start = (dx0 as usize) * bpp;

        // 越界保护：src 和 jbmp 行宽度
        let src_bpr = src.bytes_per_row();
        let dst_bpr = jbmp.bytes_per_row();
        if src_byte_start + copy_bytes > src_bpr {
            return Err(BitmapError::PixelLenMismatch {
                expected: src_byte_start + copy_bytes,
                actual: src_bpr,
            });
        }
        if dst_byte_start >= dst_bpr {
            // 完全越界 → 跳过此 piece（极端 spread 情况下可能发生）
            continue;
        }
        // 末尾 clip
        let effective_copy = (dst_bpr - dst_byte_start).min(copy_bytes);
        for y in 0..src.height {
            // src.row(y) 在 y<src.height 时 invariant 永远 Some；jbmp 同理。
            // 若违反 invariant 跳过此行以保持 fail-soft（不返错），与 C `memcpy` 行为对齐。
            let (Some(src_row), Some(dst_row)) = (src.row(y), jbmp.row_mut(y)) else {
                continue;
            };
            dst_row[dst_byte_start..dst_byte_start + effective_copy]
                .copy_from_slice(&src_row[src_byte_start..src_byte_start + effective_copy]);
        }
    }

    // C 行 2113-2120: 调整 wrectmaps（必须 ng>0 时才有意义）
    if let Some(wrmaps) = wrectmaps {
        if !wrmaps.is_empty() && ng >= 0 {
            for i in 0..=ng as usize {
                let prev_dx0 = if i == 0 { 0 } else { dx0_vec[i - 1] };
                let prev_sx0 = if i == 0 { 0 } else { sx0_vec[i - 1] };
                let x0 = if i == 0 {
                    0
                } else {
                    prev_dx0 + (sx0_vec[i] - prev_sx0)
                };
                let dx_gap = dx0_vec[i] - x0;
                if dx_gap > 0 {
                    wrectmaps_add_gap(wrmaps, x0, dx_gap);
                }
            }
        }
    }

    Ok(jbmp)
}

/// 在已有 [`WRectMaps`] 中"插入"一个 gap：从 wrap bitmap 列 `x0` 起插入 `dx` 像素宽的空白，
/// 跨过 gap 的 wrmap 拆成两段，gap 之后的 wrmap 整体右移 `dx`。
///
/// 1:1 复刻 C 版 `wrectmaps_add_gap`（`k2pdfoptlib/k2master.c:2130-2175`）。
pub fn wrectmaps_add_gap(wrectmaps: &mut WRectMaps, x0: i32, dx: i32) {
    if wrectmaps.items.is_empty() {
        return;
    }
    let x0_f = f64::from(x0);
    let dx_f = f64::from(dx);

    // C 用 wrmap2 临时变量，coords[2].x = -1 表示"未触发插入"
    let mut wrmap2: Option<WRectMap> = None;

    // 遍历所有 wrmap，调整 coords[1].x（wrap bitmap 左上角列）
    for wrmap in wrectmaps.items.iter_mut() {
        let left = wrmap.coords[1].0; // x in wrap bmp
        let width = wrmap.coords[2].0; // width

        // C 行 2145-2146: wrmap 在 gap 之前（不影响）
        if left + width < x0_f {
            continue;
        }
        // C 行 2148-2163: gap 在 wrmap 内部 → split
        if left < x0_f + 0.5 {
            let len1 = x0_f - left;
            let len2 = width - len1;
            // wrmap2 = wrmap 副本，调整成第二段
            let mut new_map = *wrmap;
            new_map.coords[0].0 = wrmap.coords[0].0 + len1;
            new_map.coords[1].0 = wrmap.coords[1].0 + len1 + dx_f;
            new_map.coords[2].0 = len2;
            wrmap2 = Some(new_map);
            // 原 wrmap 缩短到 len1
            wrmap.coords[2].0 = len1;
        } else {
            // C 行 2164-2168: gap 在 wrmap 之前 → 整体右移
            wrmap.coords[1].0 += dx_f;
        }
    }

    // C 行 2170-2174: 若 wrmap2 被填充则添加 + 重排
    if let Some(map2) = wrmap2 {
        wrectmaps.add(map2);
        wrectmaps.sort_horizontally();
    }
}

#[inline]
const fn white_byte_for(_fmt: PixelFormat) -> u8 {
    255
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_range_loop)]
    use super::*;

    // --- JustFlags decoding ---

    #[test]
    fn default_flags_match_c_init() {
        let f = JustFlags::default();
        assert_eq!(f.raw(), 0x8f);
        assert_eq!(f.h_mandate(), 3); // use user
        assert_eq!(f.h_recommend(), 0xc); // recommend right
        assert_eq!(f.f_mandate(), 0); // use user for full
        assert_eq!(f.f_recommend(), 0x80); // recommend yes
    }

    #[test]
    fn disable_full_sets_no_full_mandate() {
        let f = JustFlags(0x8f);
        let d = f.disable_full();
        // 0x8f & 0xcf = 0x8f & 0xcf = 0x8f & 11001111 = 10001111 & 11001111 = 10001111 = 0x8f? 让我重算
        // 0x8f = 10001111
        // 0xcf = 11001111
        // & = 10001111 = 0x8f
        // 不对，0x8f 第 4 位是 0 第 5 位是 0，0xcf 第 4 位是 0 第 5 位是 0，所以 AND 完全保留 0x8f
        // 因为 0x8f 本来就没设 bits 4-5
        // | 0x20 → 10101111 = 0xaf
        assert_eq!(d.raw(), 0xaf);
        assert_eq!(d.f_mandate(), 0x20); // mandatory no-full
    }

    #[test]
    fn disable_full_clears_existing_full_bit() {
        // bits 4-5 = 0x10（mandatory full）→ disable 后变 0x20
        let f = JustFlags(0x10);
        let d = f.disable_full();
        // 0x10 & 0xcf = 00010000 & 11001111 = 0
        // 0 | 0x20 = 0x20
        assert_eq!(d.raw(), 0x20);
    }

    #[test]
    fn classify_horizontal_mandatory_left() {
        let f = JustFlags(0); // bits 0-1 = 0 → mandatory left
        assert_eq!(classify_horizontal(f, -1), JustifyMode::Left);
        assert_eq!(classify_horizontal(f, 1), JustifyMode::Left); // mandate 优先
        assert_eq!(classify_horizontal(f, 2), JustifyMode::Left);
    }

    #[test]
    fn classify_horizontal_mandatory_right() {
        let f = JustFlags(2); // bits 0-1 = 2 → mandatory right
        assert_eq!(classify_horizontal(f, -1), JustifyMode::Right);
        assert_eq!(classify_horizontal(f, 0), JustifyMode::Right);
    }

    #[test]
    fn classify_horizontal_user_left() {
        let f = JustFlags(3); // use user
        assert_eq!(classify_horizontal(f, 0), JustifyMode::Left);
    }

    #[test]
    fn classify_horizontal_user_center() {
        let f = JustFlags(3); // use user
        assert_eq!(classify_horizontal(f, 1), JustifyMode::Center);
    }

    #[test]
    fn classify_horizontal_user_right() {
        let f = JustFlags(3); // use user
        assert_eq!(classify_horizontal(f, 2), JustifyMode::Right);
    }

    #[test]
    fn classify_horizontal_user_auto_with_recommend() {
        // user=-1, recommend 决定: rec=0 → Left, rec=8 → Right, 其他 → Center
        let f_left = JustFlags(3); // bits 2-3 = 0
        assert_eq!(classify_horizontal(f_left, -1), JustifyMode::Left);

        let f_right = JustFlags(3 | 8); // bits 2-3 = 8
        assert_eq!(classify_horizontal(f_right, -1), JustifyMode::Right);

        let f_center = JustFlags(3 | 4); // bits 2-3 = 4 (非 0 非 8)
        assert_eq!(classify_horizontal(f_center, -1), JustifyMode::Center);
    }

    #[test]
    fn should_full_justify_mandatory_full() {
        let f = JustFlags(0x10); // bits 4-5 = 0x10
        assert!(should_full_justify(f, -1));
        assert!(should_full_justify(f, 0)); // mandate 覆盖 user
    }

    #[test]
    fn should_full_justify_mandatory_no() {
        let f = JustFlags(0x20); // bits 4-5 = 0x20
        assert!(!should_full_justify(f, -1));
        assert!(!should_full_justify(f, 1));
    }

    #[test]
    fn should_full_justify_user_on() {
        let f = JustFlags(0); // bits 4-5 = 0 → use user
        assert!(should_full_justify(f, 1));
    }

    #[test]
    fn should_full_justify_user_off() {
        let f = JustFlags(0);
        assert!(!should_full_justify(f, 0));
    }

    #[test]
    fn should_full_justify_user_auto_recommend() {
        // user=-1, recommend bits 6-7: 0x40 = yes, 其他 = no
        let f_yes = JustFlags(0x40); // bits 6-7 = 0x40
        assert!(should_full_justify(f_yes, -1));

        let f_no = JustFlags(0); // bits 6-7 = 0
        assert!(!should_full_justify(f_no, -1));
    }

    // --- fully_justify_with_gaps ---

    fn make_gray_with_value(w: u32, h: u32, val: u8) -> Bitmap {
        let mut bmp = Bitmap::new(w, h, 300.0, PixelFormat::Gray8).unwrap();
        bmp.fill_byte(val);
        bmp
    }

    #[test]
    fn no_gaps_left_align() {
        // 单 word，width=10，jbmp=20，left align → piece 落 [0..10)
        let src = make_gray_with_value(10, 4, 100);
        let dst = fully_justify_with_gaps(&src, &[], 20, JustifyMode::Left, None).unwrap();
        assert_eq!(dst.width, 20);
        // [0..10) 是 100，[10..20) 是 255
        let row0 = dst.row(0).unwrap();
        for x in 0..10 {
            assert_eq!(row0[x], 100, "left piece at col {} should be 100", x);
        }
        for x in 10..20 {
            assert_eq!(row0[x], 255, "right white at col {} should be 255", x);
        }
    }

    #[test]
    fn no_gaps_center_align() {
        let src = make_gray_with_value(10, 4, 100);
        let dst = fully_justify_with_gaps(&src, &[], 20, JustifyMode::Center, None).unwrap();
        // destx0 = (20-10)/2 = 5
        let row0 = dst.row(0).unwrap();
        for x in 0..5 {
            assert_eq!(row0[x], 255);
        }
        for x in 5..15 {
            assert_eq!(row0[x], 100);
        }
        for x in 15..20 {
            assert_eq!(row0[x], 255);
        }
    }

    #[test]
    fn no_gaps_right_align() {
        let src = make_gray_with_value(10, 4, 100);
        let dst = fully_justify_with_gaps(&src, &[], 20, JustifyMode::Right, None).unwrap();
        // destx0 = 20-10 = 10
        let row0 = dst.row(0).unwrap();
        for x in 0..10 {
            assert_eq!(row0[x], 255);
        }
        for x in 10..20 {
            assert_eq!(row0[x], 100);
        }
    }

    #[test]
    fn with_gap_spreads_pieces() {
        // src 宽 8，gap_pos=[3]（即 word 0 占 [0..4]，word 1 占 [5..8]，gap 在 col 3-4 之间）
        // wait, gap_pos[i] = c2+1, 所以 gap_pos=[3] 表示 word 0 c2=2, piece 0 = [0..4) (dx = gap_pos[0]+1 = 4)
        // piece 1 = [gap_pos[0]+1 .. src_w) = [4..8) (dx = src_w-(gap_pos[0]+1) = 4)
        // ng=1, newwidth = min(8*5/4, 16) = 10
        // destx0 (Left) = 0
        // piece 0: sx0=0, dx0=0, copy 4 bytes from src[0..4] → dst[0..4]
        // piece 1: sx0=4, dx0=destx0+4+(10-8)*1/1=0+4+2=6, copy 4 bytes from src[4..8] → dst[6..10]
        // 所以 dst[0..4]=100, dst[4..6]=255, dst[6..10]=100, dst[10..16]=255
        let mut src = make_gray_with_value(8, 4, 100);
        // 模拟 word 1 用不同颜色便于检验
        for y in 0..4 {
            let row = src.row_mut(y).unwrap();
            for x in 4..8 {
                row[x] = 50;
            }
        }
        let dst = fully_justify_with_gaps(&src, &[3], 16, JustifyMode::Left, None).unwrap();
        let row0 = dst.row(0).unwrap();
        // piece 0
        for x in 0..4 {
            assert_eq!(row0[x], 100, "piece0 col {}", x);
        }
        // gap (white)
        for x in 4..6 {
            assert_eq!(row0[x], 255, "gap col {}", x);
        }
        // piece 1
        for x in 6..10 {
            assert_eq!(row0[x], 50, "piece1 col {}", x);
        }
        // trailing white
        for x in 10..16 {
            assert_eq!(row0[x], 255, "trailing col {}", x);
        }
    }

    #[test]
    fn fully_justify_preserves_format_and_dpi() {
        let src = Bitmap::new(10, 4, 200.0, PixelFormat::Rgb8).unwrap();
        let dst = fully_justify_with_gaps(&src, &[], 20, JustifyMode::Left, None).unwrap();
        assert_eq!(dst.format, PixelFormat::Rgb8);
        assert!((dst.dpi - 200.0).abs() < 1e-6);
        assert_eq!(dst.height, 4);
        assert_eq!(dst.width, 20);
    }

    #[test]
    fn fully_justify_rgb_white_fill() {
        let src = Bitmap::new(4, 2, 300.0, PixelFormat::Rgb8).unwrap();
        // src 全 0（黑）
        let dst = fully_justify_with_gaps(&src, &[], 10, JustifyMode::Left, None).unwrap();
        // 检查右半部分（src 外）全白
        let row0 = dst.row(0).unwrap();
        // RGB 3 bytes per pixel，src 占 0..4（cols） = 0..12（bytes）
        for x in 12..30 {
            assert_eq!(row0[x], 255, "rgb white col-byte {}", x);
        }
    }

    // --- wrectmaps_add_gap ---

    fn make_wrmap_at(x: f64, w: f64) -> WRectMap {
        let mut m = WRectMap::new();
        m.coords[1].0 = x;
        m.coords[2].0 = w;
        m
    }

    #[test]
    fn add_gap_empty_wrmaps_no_op() {
        let mut maps = WRectMaps::new();
        wrectmaps_add_gap(&mut maps, 10, 5);
        assert_eq!(maps.items.len(), 0);
    }

    #[test]
    fn add_gap_before_wrmap_no_change() {
        let mut maps = WRectMaps::new();
        maps.add(make_wrmap_at(20.0, 10.0));
        // gap 在 x0=5, dx=3，wrmap 起点 20 在 gap 后 → 应右移
        wrectmaps_add_gap(&mut maps, 5, 3);
        assert!(
            (maps.items[0].coords[1].0 - 23.0).abs() < 1e-9,
            "should shift right by 3"
        );
    }

    #[test]
    fn add_gap_after_wrmap_no_change() {
        let mut maps = WRectMaps::new();
        maps.add(make_wrmap_at(5.0, 10.0)); // wrmap 占 [5..15]
        wrectmaps_add_gap(&mut maps, 20, 3); // gap 在 20，wrmap 已结束 → 不变
        assert!((maps.items[0].coords[1].0 - 5.0).abs() < 1e-9);
        assert!((maps.items[0].coords[2].0 - 10.0).abs() < 1e-9);
    }

    #[test]
    fn add_gap_splits_wrmap() {
        // wrmap 占 [5..15], gap 在 x0=10, dx=3
        // C 算法：len1 = x0-left = 5, len2 = width-len1 = 5
        //   原 wrmap 留前半：coords[1]=5, coords[2]=5
        //   新 wrmap2：coords[1] = original.coords[1] + len1 + dx = 5+5+3 = 13, coords[2]=5
        //   coords[0] (源 x): original 100 → 新 = 100 + len1 = 105
        let mut maps = WRectMaps::new();
        let mut m = make_wrmap_at(5.0, 10.0);
        m.coords[0].0 = 100.0; // 源坐标
        maps.add(m);
        wrectmaps_add_gap(&mut maps, 10, 3);
        assert_eq!(maps.items.len(), 2);
        // 排序后第一段在前 (x=5)，第二段在后 (x=13)
        let first = &maps.items[0];
        let second = &maps.items[1];
        assert!((first.coords[1].0 - 5.0).abs() < 1e-9);
        assert!((first.coords[2].0 - 5.0).abs() < 1e-9);
        assert!(
            (second.coords[1].0 - 13.0).abs() < 1e-9,
            "second piece coords[1].x should be 13, got {}",
            second.coords[1].0
        );
        assert!((second.coords[2].0 - 5.0).abs() < 1e-9);
        // 源坐标应同步分裂
        assert!((second.coords[0].0 - 105.0).abs() < 1e-9);
    }

    // --- end-to-end smoke ---

    #[test]
    fn fully_justify_three_pieces_spread() {
        // 三 word，src=12，gaps=[3, 7]
        // pieces:
        //   p0: sx=0, dx=4 (gap_pos[0]+1=4)
        //   p1: sx=4, dx=4 (gap_pos[1]-gap_pos[0]=4)
        //   p2: sx=8, dx=4 (src_w - (gap_pos[1]+1) = 12-8 = 4)
        // ng=2, newwidth=min(12*5/4, 24)=15
        // destx0 (Left) = 0
        // dx0 piece 0 = 0
        // dx0 piece 1 = 0+4+(15-12)*1/2 = 4+1 = 5
        // dx0 piece 2 = 0+8+(15-12)*2/2 = 8+3 = 11
        let src = make_gray_with_value(12, 2, 100);
        let dst = fully_justify_with_gaps(&src, &[3, 7], 24, JustifyMode::Left, None).unwrap();
        let row0 = dst.row(0).unwrap();
        // 验证 piece 0 落 [0..4)，piece 1 落 [5..9)，piece 2 落 [11..15)
        for x in 0..4 {
            assert_eq!(row0[x], 100, "p0 col {}", x);
        }
        assert_eq!(row0[4], 255);
        for x in 5..9 {
            assert_eq!(row0[x], 100, "p1 col {}", x);
        }
        for x in 9..11 {
            assert_eq!(row0[x], 255, "gap col {}", x);
        }
        for x in 11..15 {
            assert_eq!(row0[x], 100, "p2 col {}", x);
        }
        for x in 15..24 {
            assert_eq!(row0[x], 255, "tail col {}", x);
        }
    }

    #[test]
    fn fully_justify_jbmp_narrower_than_src_clipped() {
        // jbmp=8 < src=10：应不崩，piece spread 但 clip 到 dst 边缘
        let src = make_gray_with_value(10, 1, 100);
        // 无 gap → newwidth=src.width=10，但 jbmp=8 < 10
        // destx0 (Left) = 0
        // piece 0: sx=0, dx=10, dst[0..10)，但 dst 只到 col 7 → 末尾 effective_copy=8
        let dst = fully_justify_with_gaps(&src, &[], 8, JustifyMode::Left, None).unwrap();
        let row0 = dst.row(0).unwrap();
        for x in 0..8 {
            assert_eq!(row0[x], 100);
        }
    }

    #[test]
    fn flags_into_i32_roundtrip() {
        let f = JustFlags(0x88);
        let raw: i32 = f.into();
        let f2: JustFlags = raw.into();
        assert_eq!(f, f2);
    }
}
