//! `spacing_state` - 行间距与 lastrow 度量桶。
//!
//! 见 [`crate::master`] 模块文档与 `docs/masterinfo-design.md` §2 第 3 行。
//!
//! # C 字段对应
//!
//! 来源：`k2pdfoptlib/k2pdfopt.h:674-742`（MASTERINFO struct 的 lastrow / spacing 字段）
//!
//! | C 字段 | Rust 字段 | C 行号 |
//! |--------|-----------|--------|
//! | `lastrow.lcheight` | [`SpacingState::last_row_lcheight`] | 716 |
//! | `lastrow.capheight` | [`SpacingState::last_row_capheight`] | 716 |
//! | `lastrow.h5050` | [`SpacingState::last_row_h5050`] | 716 |
//! | `lastrow.rowheight` | [`SpacingState::last_row_rowheight`] | 716 |
//! | `lastrow.gap` | [`SpacingState::last_row_gap`] | 716 |
//! | `lastrow.gapblank` | [`SpacingState::last_row_gapblank`] | 716 |
//! | `lastrow.type` | [`SpacingState::last_row_type`] | 716 |
//! | `nocr` | [`SpacingState::nocr`] | 721 |
//! | `gapblank` | [`SpacingState::gapblank`] | 722 |
//! | `mandatory_region_gap` | [`SpacingState::mandatory_region_gap`] | 723 |
//! | `page_region_gap_in` | [`SpacingState::page_region_gap_in`] | 731 |

use crate::master::RegionType;

/// 行间距状态桶：跟踪 lastrow 的尺寸 / gap，用于计算下一行的行距。
///
/// 算法部分（update_after_add、calculate gap from text-row stats）在 Step 7.1+（M5）落地。
#[derive(Debug, Clone, PartialEq)]
pub struct SpacingState {
    /// lastrow 的 lowercase letter height（小写字母高度）。
    /// 对应 C `lastrow.lcheight`。`-1` = 未设置（初始化哨兵）。
    pub last_row_lcheight: i32,
    /// lastrow 的 capital letter height（大写字母高度）。
    /// 对应 C `lastrow.capheight`。`-1` = 未设置。
    pub last_row_capheight: i32,
    /// lastrow 的 50% 高度位置（h5050，用于 baseline 推断）。
    /// 对应 C `lastrow.h5050`。`-1` = 未设置。
    pub last_row_h5050: i32,
    /// lastrow 的总行高（pixels）。
    /// 对应 C `lastrow.rowheight`。`-1` = 未设置。
    pub last_row_rowheight: i32,
    /// lastrow 的 gap（基线到下一行顶部的距离，pixels）。
    /// 对应 C `lastrow.gap`。
    pub last_row_gap: i32,
    /// lastrow 的 gapblank（基线到下一非空白行顶部的距离）。
    /// 对应 C `lastrow.gapblank`。
    pub last_row_gapblank: i32,
    /// lastrow 的类型（Text / Figure / Blank）。
    /// 对应 C `lastrow.type` 字段（REGION_TYPE_* 枚举）。
    pub last_row_type: RegionType,
    /// Scaling value used on lastrow（lastrow 上的 OCR / scale 比例）。
    /// 对应 C `nocr`（k2pdfopt.h:721）。
    pub nocr: i32,
    /// Master bitmap 底部当前白色 gap 像素数。
    /// 对应 C `gapblank`（k2pdfopt.h:722）。
    pub gapblank: i32,
    /// 是否强制使用 `page_region_gap_in`。
    /// 对应 C `mandatory_region_gap`（k2pdfopt.h:723）。
    /// 0 = 跟随自然 gap；1 = 强制使用 `page_region_gap_in`。
    pub mandatory_region_gap: i32,
    /// 跨页 region 间距（inch）。
    /// 对应 C `page_region_gap_in`（k2pdfopt.h:731）。`-1.0` = 未设置。
    pub page_region_gap_in: f64,
}

impl SpacingState {
    /// 构造默认空 SpacingState（所有 lastrow 度量初始为 `-1` 哨兵）。
    ///
    /// 对应 C 版 `masterinfo_init` 中 lastrow 全 0/-1 清零的语义。
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_row_lcheight: -1,
            last_row_capheight: -1,
            last_row_h5050: -1,
            last_row_rowheight: -1,
            last_row_gap: 0,
            last_row_gapblank: 0,
            last_row_type: RegionType::Undetermined,
            nocr: 0,
            gapblank: 0,
            mandatory_region_gap: 0,
            page_region_gap_in: -1.0,
        }
    }

    /// add_bitmap 第 8 步：根据新加入的 bitmap 高度 + dpi 更新 lastrow 度量。
    ///
    /// **Step 7.3（M5）简化实现**：当前只更新 `last_row_rowheight`（用入参 height）
    /// 和 `nocr=1`（与 M5 直通模式一致——bitmap 已是输出 DPI 不需要再 scale）。
    /// 完整的 textrow-derived lastrow 字段（lcheight/capheight/h5050/gap/gapblank/
    /// type）由 M6/M7 wrap_state + row detection 串联后落地（推迟到 Step 8.x）。
    ///
    /// 对应 C 版 `k2master.c:640-642`：`masterinfo->lastrow = region.textrows.
    /// textrow[region.textrows.n-1]; masterinfo->nocr = nocr;`。
    pub fn update_after_add(&mut self, height: u32, dpi: f64) {
        let _ = dpi; // Step 7.3 简化模式：直通 dst_dpi，不参与计算
        self.last_row_rowheight = height as i32;
        self.last_row_type = RegionType::Text;
        self.nocr = 1;
    }
}

impl Default for SpacingState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_initializes_lastrow_sentinels() {
        let s = SpacingState::new();
        assert_eq!(s.last_row_lcheight, -1);
        assert_eq!(s.last_row_capheight, -1);
        assert_eq!(s.last_row_h5050, -1);
        assert_eq!(s.last_row_rowheight, -1);
        assert_eq!(s.last_row_gap, 0);
        assert_eq!(s.last_row_gapblank, 0);
        assert_eq!(s.last_row_type, RegionType::Undetermined);
        assert_eq!(s.nocr, 0);
        assert_eq!(s.gapblank, 0);
        assert_eq!(s.mandatory_region_gap, 0);
        assert!((s.page_region_gap_in - (-1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn default_eq_new() {
        let a = SpacingState::default();
        let b = SpacingState::new();
        assert_eq!(a, b);
    }

    #[test]
    fn fields_writable() {
        let mut s = SpacingState::new();
        s.last_row_rowheight = 30;
        s.last_row_type = RegionType::Text;
        s.nocr = 5;
        s.mandatory_region_gap = 1;
        s.page_region_gap_in = 0.25;
        assert_eq!(s.last_row_rowheight, 30);
        assert_eq!(s.last_row_type, RegionType::Text);
        assert_eq!(s.nocr, 5);
        assert_eq!(s.mandatory_region_gap, 1);
        assert!((s.page_region_gap_in - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn update_after_add_sets_rowheight_and_type() {
        let mut s = SpacingState::new();
        s.update_after_add(30, 300.0);
        assert_eq!(s.last_row_rowheight, 30);
        assert_eq!(s.last_row_type, RegionType::Text);
        assert_eq!(s.nocr, 1);
    }
}
