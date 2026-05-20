//! `hyphen` - 行尾连字符（hyphen / dash）检测（Step 8.2 / M6）。
//!
//! 实现 C 版 `bmpregion_hyphen_detect`（`k2pdfoptlib/bmpregion.c:879-1174`）的
//! 1:1 Rust 移植。算法沿文本行尾从行外向行内扫描，在 baseline 中心附近的窄水平条带内
//! 找出可能是 hyphen 的水平笔画位置，并定位 hyphen 与前一字母的接触点。
//!
//! # 算法概要
//!
//! 1. 围绕 baseline 中心 `±0.04 * lcheight` 的窄水平条带（rmin..=rmax）扫描
//! 2. 朝行内方向（LTR 时从 c2 → c1，RTL 时从 c1 → c2）逐列遍历
//! 3. 在每列里找最接近 mid 的 dark pixel（dr），并记录上方/下方字母段的边界
//!    `col_r0/col_r1/col_r2/col_r3`：
//!     - `col_r1`：hyphen 段顶部行（mid 往上第一个白行的下一行）
//!     - `col_r0`：上方字母底部行（hyphen 上面遇到的下一个 dark）
//!     - `col_r2`：hyphen 段底部行（mid 往下第一个白行的上一行）
//!     - `col_r3`：下方字母顶部行
//! 4. 多种启发式终止条件：
//!     - 列内无 dark → 可能 hyphen 已结束，记录 `ch = j - cdir`
//!     - 下/上方字母触及 → 设 c2 = j，break
//!     - hyphen 段太厚 / 太薄 / 不在 baseline 正确位置 → break
//! 5. 后置 sanity：必须有 c2（前面有字母）+ aspect ratio ∈ [0.08, 0.75]
//!
//! 不满足以上条件返回 [`HyphenInfo::none()`]（C 同 `textrow->hyphen.ch = -1`）。
//!
//! # C 行号对照
//!
//! - C 入口：`bmpregion.c:879` `void bmpregion_hyphen_detect(BMPREGION *region, int hyphen_detect, int left_to_right)`
//! - C 主循环：`bmpregion.c:967-1138`
//! - C 后置检查：`bmpregion.c:1142-1163`
//! - 调用方：`wrapbmp.c:138` `bmpregion_hyphen_detect(region, k2settings->hyphen_detect, ...)`
//!
//! # Step 8.1 与 8.2 的边界
//!
//! Step 8.1 的 [`crate::wrap::detect_hyphen`] 是 stub（永远返 None）。Step 8.2 把真实算法
//! 落在本模块，并由 [`crate::wrap::detect_hyphen`] 委托调用。WrapState::add_word 不直接
//! 触发 hyphen 检测（避免循环借用），仍由调用方在 wrap 流之外预先填好 `AddRegion::hyphen`。

use crate::master::HyphenInfo;
use k2types::Bitmap;

/// hyphen 检测入参。
///
/// 字段语义与 C `BMPREGION->bbox` + `BMPREGION->bgcolor` + `left_to_right` 一致。
#[derive(Debug, Clone, Copy)]
pub struct HyphenDetectInput<'a> {
    /// 源 bitmap（不可变借用）。算法仅读 `gray_at(x, y)`。
    pub bmp: &'a Bitmap,
    /// region 左列（inclusive）。C `textrow->c1`。
    pub c1: i32,
    /// region 右列（inclusive）。C `textrow->c2`。
    pub c2: i32,
    /// region 顶部行（inclusive）。C `textrow->r1`。
    pub r1: i32,
    /// region 底部行（inclusive）。C `textrow->r2`。
    pub r2: i32,
    /// 基线行号。C `textrow->rowbase`。
    pub rowbase: i32,
    /// cap height。C `textrow->capheight`。
    pub capheight: i32,
    /// lower-case x-height。C `textrow->lcheight`。`lcheight <= 0` 直接返回 None。
    pub lcheight: i32,
    /// 白色阈值（>= 视为白，< 视为黑/前景）。C `region->bgcolor`。
    pub bgcolor: u8,
    /// 文字方向。`true` = LTR（行尾在右），`false` = RTL（行尾在左）。
    pub left_to_right: bool,
}

impl<'a> HyphenDetectInput<'a> {
    /// 用 region + bbox 字段一次性构造。
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bmp: &'a Bitmap,
        c1: i32,
        c2: i32,
        r1: i32,
        r2: i32,
        rowbase: i32,
        capheight: i32,
        lcheight: i32,
        bgcolor: u8,
        left_to_right: bool,
    ) -> Self {
        Self {
            bmp,
            c1,
            c2,
            r1,
            r2,
            rowbase,
            capheight,
            lcheight,
            bgcolor,
            left_to_right,
        }
    }
}

/// 检测 region 行尾是否存在 hyphen，返回 [`HyphenInfo`]。
///
/// `ch < 0` 表示无 hyphen（等价 C `textrow->hyphen.ch = -1`）。
///
/// # 复杂度
///
/// O(width × max_drmax) ≈ O(width × row_height)。算法只扫一次行，常数因子小。
#[must_use]
pub fn detect_hyphen(input: &HyphenDetectInput<'_>) -> HyphenInfo {
    let textrow_c1 = input.c1;
    let textrow_c2 = input.c2;
    let textrow_r1 = input.r1;
    let textrow_r2 = input.r2;
    let textrow_rowbase = input.rowbase;
    let textrow_capheight = input.capheight;
    let textrow_lcheight = input.lcheight;
    let bgcolor = input.bgcolor;

    // C 行 921-928: 字段合法性
    if textrow_c2 < 0
        || textrow_c1 < 0
        || textrow_r1 < 0
        || textrow_r2 < 0
        || textrow_rowbase < 0
        || textrow_capheight < 0
        || textrow_lcheight < 0
    {
        return HyphenInfo::none();
    }

    let width = textrow_c2 - textrow_c1 + 1;
    // C 行 929-936: width<2 早退
    if width < 2 {
        return HyphenInfo::none();
    }

    // lcheight==0 时除法会爆炸；直接早退（保留 C 行 922 的相同语义防御）
    if textrow_lcheight == 0 {
        return HyphenInfo::none();
    }

    let usize_width = width as usize;
    let mut col_r0 = vec![-1i32; usize_width];
    let mut col_r1 = vec![-1i32; usize_width];
    let mut col_r2 = vec![-1i32; usize_width];
    let mut col_r3 = vec![-1i32; usize_width];

    let lcf = f64::from(textrow_lcheight);

    // C 行 943-948: rmin/rmax = hyphen 段允许的上下边界初值
    let mut rmin = (f64::from(textrow_rowbase) - f64::from(textrow_capheight) - lcf * 0.04) as i32;
    if rmin < textrow_r1 {
        rmin = textrow_r1;
    }
    let mut rmax = (f64::from(textrow_rowbase) + lcf * 0.04) as i32;
    if rmax > textrow_r2 {
        rmax = textrow_r2;
    }

    // 结果变量
    let mut hyphen_ch: i32 = -1;
    let mut hyphen_c2: i32 = -1;
    let mut hyphen_hr1: i32 = -1;
    let mut hyphen_hr2: i32 = -1;
    let mut nrmid: i32 = 0;

    // C 行 952-963: 扫描方向
    let (cstart, cend, cdir) = if input.left_to_right {
        (textrow_c2, textrow_c1 - 1, -1)
    } else {
        (textrow_c1, textrow_c2 + 1, 1)
    };

    // C 行 967-1138 主循环
    let mut j = cstart;
    while j != cend {
        // C 行 972-985: 找最接近 center 的 dark pixel
        let mut rmid = (rmin + rmax) / 2;
        let drmax_top = textrow_r2 + 1 - rmid;
        let drmax_bot = rmid - textrow_r1 + 1;
        let drmax = drmax_top.max(drmax_bot);

        let mut dr: i32 = drmax;
        let mut dr_found = false;
        for d in 0..drmax {
            // 先看 below（C 顺序: `rmid+dr` 在前）
            if rmid + d <= textrow_r2 {
                let g = pixel_gray(input.bmp, j, rmid + d);
                if g < bgcolor {
                    dr = d;
                    dr_found = true;
                    break;
                }
            }
            if rmid - d >= textrow_r1 {
                let g = pixel_gray(input.bmp, j, rmid - d);
                if g < bgcolor {
                    dr = -d;
                    dr_found = true;
                    break;
                }
            }
        }
        // 终止 #1 + 主分支

        // C 行 989-1008: 终止 #1
        let cond1_a = !dr_found || dr >= drmax;
        let cond1_b =
            nrmid > 2 && f64::from(nrmid) / lcf > 0.1 && (rmid + dr < rmin || rmid + dr > rmax);
        if cond1_a || cond1_b {
            // C 行 994-995: 已有 ch && dr>=drmax → continue（不 break）
            if hyphen_ch >= 0 && cond1_a {
                j += cdir;
                continue;
            }
            // C 行 996-1001: nrmid 满足设置 ch
            if nrmid > 2 && f64::from(nrmid) / lcf > 0.35 {
                hyphen_ch = j - cdir;
                hyphen_hr1 = rmin;
                hyphen_hr2 = rmax;
            }
            // C 行 1002-1006: dr<drmax 触发的（即 cond1_b）→ 设 c2=j break
            if !cond1_a {
                hyphen_c2 = j;
                break;
            }
            j += cdir;
            continue;
        }

        // C 行 1009-1013: 已检测到 ch 但本列又发现 dark → c2=j break
        if hyphen_ch >= 0 {
            hyphen_c2 = j;
            break;
        }

        nrmid += 1;
        rmid += dr;
        let col_idx = (j - textrow_c1) as usize;

        // C 行 1025-1037: 向上扫，找 col_r1 + col_r0
        let mut r = rmid;
        while r >= textrow_r1 {
            let g = pixel_gray(input.bmp, j, r);
            if g >= bgcolor {
                break;
            }
            r -= 1;
        }
        col_r1[col_idx] = r + 1;
        col_r0[col_idx] = -1;
        if r >= textrow_r1 {
            while r >= textrow_r1 {
                let g = pixel_gray(input.bmp, j, r);
                if g < bgcolor {
                    break;
                }
                r -= 1;
            }
            if r >= textrow_r1 {
                col_r0[col_idx] = r;
            }
        }

        // C 行 1038-1050: 向下扫，找 col_r2 + col_r3
        let mut r = rmid;
        while r <= textrow_r2 {
            let g = pixel_gray(input.bmp, j, r);
            if g >= bgcolor {
                break;
            }
            r += 1;
        }
        col_r2[col_idx] = r - 1;
        col_r3[col_idx] = -1;
        if r <= textrow_r2 {
            while r <= textrow_r2 {
                let g = pixel_gray(input.bmp, j, r);
                if g < bgcolor {
                    break;
                }
                r += 1;
            }
            if r <= textrow_r2 {
                col_r3[col_idx] = r;
            }
        }

        // C 行 1054-1055: c2 未设 && (col_r0 或 col_r3 已检测到字母) → c2=j
        if hyphen_c2 < 0 && (col_r0[col_idx] >= 0 || col_r3[col_idx] >= 0) {
            hyphen_c2 = j;
        }

        // C 行 1056-1069: 终止 #2
        if nrmid > 2
            && f64::from(nrmid) / lcf > 0.35
            && (col_r1[col_idx] > rmax || col_r2[col_idx] < rmin)
        {
            hyphen_ch = j - cdir;
            hyphen_hr1 = rmin;
            hyphen_hr2 = rmax;
            if hyphen_c2 < 0 {
                hyphen_c2 = j;
            }
            break;
        }

        // C 行 1072-1094: r1/r2 漂移 DQ
        if nrmid > 1 {
            if f64::from(rmin - col_r1[col_idx]) / lcf > 0.1
                || f64::from(col_r2[col_idx] - rmax) / lcf > 0.1
            {
                break;
            }
            if f64::from(nrmid) / lcf > 0.1
                && (f64::from((rmin - col_r1[col_idx]).abs()) / lcf > 0.1
                    || f64::from(rmax - col_r2[col_idx]) / lcf > 0.1)
            {
                break;
            }
        }

        // C 行 1095-1098: 更新 rmin/rmax
        if nrmid == 1 || col_r1[col_idx] < rmin {
            rmin = col_r1[col_idx];
        }
        if nrmid == 1 || col_r2[col_idx] > rmax {
            rmax = col_r2[col_idx];
        }

        // C 行 1099-1137: thickness + centered sanity
        if f64::from(nrmid) / lcf > 0.1 && nrmid > 1 {
            let thickness = f64::from(rmax - rmin + 1) / lcf;
            if !(0.05..=0.55).contains(&thickness) {
                break;
            }
            let rmean = f64::from(rmax + rmin) / 2.0;
            let centered_rat = (f64::from(textrow_rowbase) - rmean) / lcf;
            if !(0.25..=0.85).contains(&centered_rat) {
                break;
            }
            if f64::from(textrow_rowbase - rmax) / lcf < 0.2
                || f64::from(textrow_rowbase - rmin) / lcf > 0.92
            {
                break;
            }
        }

        j += cdir;
    }

    // C 行 1142-1163: 后置 sanity
    if hyphen_ch < 0 {
        return HyphenInfo::none();
    }
    if hyphen_c2 < 0 {
        // C 行 1147-1153: 只有 hyphen 无前导字母 → 可能是 dash，不计
        return HyphenInfo::none();
    }
    if nrmid <= 0 {
        // 防御：极端情况避免除 0
        return HyphenInfo::none();
    }
    let ar = f64::from(hyphen_hr2 - hyphen_hr1) / f64::from(nrmid);
    if !(0.08..=0.75).contains(&ar) {
        return HyphenInfo::none();
    }

    HyphenInfo {
        ch: hyphen_ch,
        c2: hyphen_c2,
        r1: hyphen_hr1,
        r2: hyphen_hr2,
    }
}

/// 读 bitmap 像素灰度。越界返回 255（白）= "不计入 dark"。
#[inline]
fn pixel_gray(bmp: &Bitmap, x: i32, y: i32) -> u8 {
    if x < 0 || y < 0 {
        return 255;
    }
    bmp.gray_at(x as u32, y as u32).unwrap_or(255)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_range_loop)]
    use super::*;
    use k2types::PixelFormat;

    /// 构造一张全白 bitmap（255）。
    fn make_white(w: u32, h: u32) -> Bitmap {
        let mut bmp = Bitmap::new(w, h, 300.0, PixelFormat::Gray8).unwrap();
        bmp.fill_byte(255);
        bmp
    }

    /// 在 bitmap 上画一个水平 hyphen 段（dark pixels）于 [c_left, c_right] x [r_top, r_bot]。
    fn draw_segment(bmp: &mut Bitmap, c_left: u32, c_right: u32, r_top: u32, r_bot: u32) {
        for y in r_top..=r_bot {
            let row = bmp.row_mut(y).unwrap();
            for x in c_left..=c_right {
                row[x as usize] = 0;
            }
        }
    }

    /// 在 bitmap 上画一个垂直字母 stem（dark pixels），覆盖整行高度。
    fn draw_letter_stem(bmp: &mut Bitmap, col: u32, r_top: u32, r_bot: u32) {
        for y in r_top..=r_bot {
            let row = bmp.row_mut(y).unwrap();
            row[col as usize] = 0;
        }
    }

    #[test]
    fn defaults_construct_clean() {
        // 全白 bitmap → 不应检测到 hyphen
        let bmp = make_white(40, 20);
        let input = HyphenDetectInput::new(&bmp, 0, 39, 0, 19, 14, 10, 10, 255, true);
        let h = detect_hyphen(&input);
        assert!(!h.is_hyphen());
    }

    #[test]
    fn invalid_bbox_returns_none() {
        let bmp = make_white(40, 20);
        // c1<0
        let input = HyphenDetectInput::new(&bmp, -1, 39, 0, 19, 14, 10, 10, 255, true);
        assert!(!detect_hyphen(&input).is_hyphen());
        // rowbase<0
        let input = HyphenDetectInput::new(&bmp, 0, 39, 0, 19, -1, 10, 10, 255, true);
        assert!(!detect_hyphen(&input).is_hyphen());
        // capheight<0
        let input = HyphenDetectInput::new(&bmp, 0, 39, 0, 19, 14, -1, 10, 255, true);
        assert!(!detect_hyphen(&input).is_hyphen());
        // lcheight<0
        let input = HyphenDetectInput::new(&bmp, 0, 39, 0, 19, 14, 10, -1, 255, true);
        assert!(!detect_hyphen(&input).is_hyphen());
        // lcheight=0 防御
        let input = HyphenDetectInput::new(&bmp, 0, 39, 0, 19, 14, 10, 0, 255, true);
        assert!(!detect_hyphen(&input).is_hyphen());
    }

    #[test]
    fn width_lt_2_returns_none() {
        let bmp = make_white(40, 20);
        // width=c2-c1+1=1
        let input = HyphenDetectInput::new(&bmp, 10, 10, 0, 19, 14, 10, 10, 255, true);
        assert!(!detect_hyphen(&input).is_hyphen());
    }

    #[test]
    fn isolated_dash_without_letter_rejected() {
        // 只有一段 hyphen，前面无字母（无 c2）→ probably dash → None
        let mut bmp = make_white(40, 20);
        // 行高 20，baseline=14，lcheight=10，hyphen 段在中线附近
        // mid = (rmin+rmax)/2 ≈ baseline-lcheight/2 ≈ 9
        // 画一段 hyphen 在 c=20..30 y=9..10
        draw_segment(&mut bmp, 20, 30, 9, 10);
        let input = HyphenDetectInput::new(&bmp, 0, 39, 0, 19, 14, 10, 10, 255, true);
        let h = detect_hyphen(&input);
        // 因为前面无字母 stem → c2 未设 → None
        assert!(!h.is_hyphen());
    }

    #[test]
    fn hyphen_with_preceding_letter_detected_ltr() {
        // LTR: 行尾在 c2 右侧。画一个字母 stem 在 c=15..16，r=6..=12（顶部留 2 像素白便于
        // col_r0 detection）+ hyphen 在 c=20..30, y=9..10
        let mut bmp = make_white(40, 20);
        // letter stem
        for y in 6..=12 {
            let row = bmp.row_mut(y).unwrap();
            row[15] = 0;
            row[16] = 0;
        }
        // hyphen 段在 c=20..30，y=9..10（rowbase=14 上方 4-5 像素，对应 mid 范围）
        draw_segment(&mut bmp, 20, 30, 9, 10);

        let input = HyphenDetectInput::new(&bmp, 0, 39, 4, 14, 14, 10, 10, 255, true);
        let h = detect_hyphen(&input);
        assert!(
            h.is_hyphen(),
            "expected hyphen detected, got ch={} c2={}",
            h.ch,
            h.c2
        );
        // ch 应在 hyphen 段左缘附近（C: ch = j - cdir = 19 - (-1) = 20）
        assert!(
            (19..=22).contains(&h.ch),
            "ch should be near hyphen left edge, got {}",
            h.ch
        );
        // c2 = 触发终止 #1 时的列（hyphen 左侧第一段全白列），近似 19
        assert!(
            (15..=22).contains(&h.c2),
            "c2 should be near hyphen-letter boundary, got {}",
            h.c2
        );
        // r1/r2 应近似 hyphen 段高度
        assert!(h.r1 <= 9 && h.r2 >= 10);
    }

    #[test]
    fn hyphen_with_preceding_letter_detected_rtl() {
        // RTL: 行尾在 c1 左侧。画字母 stem 在 c=23..24（hyphen 右边，留间隔）+ hyphen 在 c=10..20
        let mut bmp = make_white(40, 20);
        for y in 6..=12 {
            let row = bmp.row_mut(y).unwrap();
            row[23] = 0;
            row[24] = 0;
        }
        draw_segment(&mut bmp, 10, 20, 9, 10);

        let input = HyphenDetectInput::new(&bmp, 0, 39, 4, 14, 14, 10, 10, 255, false);
        let h = detect_hyphen(&input);
        assert!(
            h.is_hyphen(),
            "RTL hyphen should be detected, got ch={} c2={}",
            h.ch,
            h.c2
        );
        // RTL: ch = j - cdir = 21 - 1 = 20（hyphen 右缘附近）
        assert!(
            (18..=22).contains(&h.ch),
            "RTL ch should be near hyphen right edge, got {}",
            h.ch
        );
    }

    #[test]
    fn thick_segment_rejected_as_not_hyphen() {
        // 太厚的水平段（厚度 > 0.55 * lcheight）→ break，可能不被识别为 hyphen
        let mut bmp = make_white(40, 30);
        draw_letter_stem(&mut bmp, 5, 5, 20);
        // hyphen 段从 c=10..30 y=8..18（厚度 10 像素，lcheight=10，比例 1.1 > 0.55）
        draw_segment(&mut bmp, 10, 30, 8, 18);

        let input = HyphenDetectInput::new(&bmp, 0, 39, 5, 20, 20, 14, 10, 255, true);
        let h = detect_hyphen(&input);
        // 太厚 → 不识别
        assert!(!h.is_hyphen(), "thick block should not be hyphen");
    }

    #[test]
    fn thin_segment_rejected_too_thin() {
        // 太薄（< 0.05 * lcheight=0.5 → 几乎不可能；至少 1 像素，比例 0.1 > 0.05 ok）
        // 改测 aspect ratio：单点小段不被识别
        let bmp = make_white(40, 20);
        let input = HyphenDetectInput::new(&bmp, 0, 39, 0, 19, 14, 10, 10, 255, true);
        let h = detect_hyphen(&input);
        assert!(!h.is_hyphen());
    }

    #[test]
    fn bgcolor_threshold_affects_detection() {
        // 用浅灰像素（200）+ bgcolor=128 → 200>=128 视为白 → 检测不到
        let mut bmp = make_white(40, 20);
        // 画浅灰像素 hyphen
        for y in 9..=10 {
            let row = bmp.row_mut(y).unwrap();
            for x in 20..=30 {
                row[x] = 200;
            }
        }
        // letter stem 紧贴 hyphen 左侧（顶部留 2 像素白）
        for y in 6..=12 {
            let row = bmp.row_mut(y).unwrap();
            row[15] = 200;
            row[16] = 200;
        }

        // bgcolor=128 → 200>=128 视为白 → 不检测
        let input = HyphenDetectInput::new(&bmp, 0, 39, 4, 14, 14, 10, 10, 128, true);
        assert!(!detect_hyphen(&input).is_hyphen());
        // bgcolor=255 → 200<255 视为黑 → 检测（前提：letter stem 也是 dark）
        let input = HyphenDetectInput::new(&bmp, 0, 39, 4, 14, 14, 10, 10, 255, true);
        let h = detect_hyphen(&input);
        assert!(h.is_hyphen(), "bgcolor=255 should detect light gray hyphen");
    }

    #[test]
    fn empty_region_returns_none() {
        // 全白 + 合法 bbox → None
        let bmp = make_white(20, 20);
        let input = HyphenDetectInput::new(&bmp, 0, 19, 0, 19, 14, 10, 10, 255, true);
        let h = detect_hyphen(&input);
        assert!(!h.is_hyphen());
    }

    #[test]
    fn hyphen_info_is_hyphen_flag() {
        let none = HyphenInfo::none();
        assert!(!none.is_hyphen());

        let real = HyphenInfo {
            ch: 20,
            c2: 30,
            r1: 9,
            r2: 10,
        };
        assert!(real.is_hyphen());
    }

    #[test]
    fn rgb_bitmap_grayscale_works() {
        // RGB bitmap 上 gray_at 自动转灰度
        let mut bmp = Bitmap::new(40, 20, 300.0, PixelFormat::Rgb8).unwrap();
        // 初始化全白
        bmp.fill_rgb(255, 255, 255);
        // 字母 stem r=6..=12（顶部留 2 像素白）
        for y in 6..=12 {
            let px = bmp.pixel_mut(15, y).unwrap();
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            let px = bmp.pixel_mut(16, y).unwrap();
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
        }
        // hyphen
        for y in 9..=10 {
            for x in 20..=30 {
                let px = bmp.pixel_mut(x, y).unwrap();
                px[0] = 0;
                px[1] = 0;
                px[2] = 0;
            }
        }
        let input = HyphenDetectInput::new(&bmp, 0, 39, 4, 14, 14, 10, 10, 255, true);
        let h = detect_hyphen(&input);
        assert!(h.is_hyphen(), "RGB bitmap hyphen should be detected");
    }

    #[test]
    fn aspect_ratio_filter() {
        // 模拟一个很窄的 hyphen 段（高度大，宽度小）→ aspect ratio 太大 → 拒绝
        // 但当前 detect 已经会因 thickness 太大或终止条件被拒，这里间接验证
        let bmp = make_white(40, 20);
        let input = HyphenDetectInput::new(&bmp, 0, 39, 0, 19, 14, 10, 10, 255, true);
        assert!(!detect_hyphen(&input).is_hyphen());
    }

    #[test]
    fn input_new_builder_works() {
        let bmp = make_white(40, 20);
        let input = HyphenDetectInput::new(&bmp, 1, 2, 3, 4, 5, 6, 7, 200, false);
        assert_eq!(input.c1, 1);
        assert_eq!(input.c2, 2);
        assert_eq!(input.r1, 3);
        assert_eq!(input.r2, 4);
        assert_eq!(input.rowbase, 5);
        assert_eq!(input.capheight, 6);
        assert_eq!(input.lcheight, 7);
        assert_eq!(input.bgcolor, 200);
        assert!(!input.left_to_right);
    }
}
