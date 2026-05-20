//! `native_box` - Native PDF crop boxes（feature gated）桶。
//!
//! 见 [`crate::master`] 模块文档与 `docs/masterinfo-design.md` §2 第 8 行。
//!
//! # C 字段对应
//!
//! 来源：`k2pdfoptlib/k2pdfopt.h:674-742`（MASTERINFO struct 的 native PDF 字段）
//!
//! | C 字段 | Rust 字段 | C 行号 |
//! |--------|-----------|--------|
//! | `pageinfo` (WPDFPAGEINFO) | [`NativeBoxAccumulator::boxes`] | 693 |
//!
//! # Feature gating
//!
//! Native PDF 输出（保留原 PDF text/vector，仅重排 crop boxes）是 M8+ 高级功能，
//! 当前 (M3) 仅落地数据结构占位，算法实现要等 M8+。

/// 单个 native PDF crop box（在源 PDF 坐标系）。
///
/// 对应 C 版 `WPDFPAGEINFO.boxes[]` 的单元素（`willuslib/wpdfoutline.c` 的相关数据结构）。
#[derive(Debug, Clone)]
pub struct CropBox {
    /// 左下角 x 坐标（PDF 单位 = 1/72 inch）。
    pub x: f64,
    /// 左下角 y 坐标（PDF 单位 = 1/72 inch）。
    pub y: f64,
    /// 宽度（PDF 单位）。
    pub w: f64,
    /// 高度（PDF 单位）。
    pub h: f64,
    /// 源 PDF 页号（0-based）。
    pub src_page: i32,
    /// 输出 PDF 页号（0-based）。`-1` = 尚未映射。
    pub dst_page: i32,
}

/// Native PDF crop boxes 累积器（feature gated，M8+）。
///
/// 算法部分（apply_box / map_to_output）在 M8+ 落地。
#[derive(Debug, Clone, PartialEq)]
pub struct NativeBoxAccumulator {
    /// 累积的 crop boxes。对应 C `pageinfo.boxes[]`。
    pub boxes: Vec<CropBox>,
}

impl NativeBoxAccumulator {
    /// 构造默认空 NativeBoxAccumulator。
    #[must_use]
    pub fn new() -> Self {
        Self { boxes: Vec::new() }
    }

    /// 添加一个 crop box。
    ///
    /// **未实现**（Step 5.6 占位）。落地于 M8+（native PDF 输出阶段）。
    pub fn add_box(&mut self, cbox: CropBox) {
        let _ = cbox;
        unimplemented!("add_box (wpdfoutline.c) — M8+ (native PDF output)")
    }
}

impl Default for NativeBoxAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

// CropBox 不实现 Eq（含 f64）；用 PartialEq 的 bit-level 比较避免 NaN 异常。
impl PartialEq for CropBox {
    fn eq(&self, other: &Self) -> bool {
        self.x.to_bits() == other.x.to_bits()
            && self.y.to_bits() == other.y.to_bits()
            && self.w.to_bits() == other.w.to_bits()
            && self.h.to_bits() == other.h.to_bits()
            && self.src_page == other.src_page
            && self.dst_page == other.dst_page
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let n = NativeBoxAccumulator::new();
        assert!(n.boxes.is_empty());
    }

    #[test]
    fn default_eq_new() {
        let a = NativeBoxAccumulator::default();
        let b = NativeBoxAccumulator::new();
        assert_eq!(a, b);
    }

    #[test]
    fn cropbox_construct() {
        let c = CropBox {
            x: 100.0,
            y: 200.0,
            w: 300.0,
            h: 400.0,
            src_page: 0,
            dst_page: 1,
        };
        assert!((c.x - 100.0).abs() < f64::EPSILON);
        assert!((c.y - 200.0).abs() < f64::EPSILON);
        assert!((c.w - 300.0).abs() < f64::EPSILON);
        assert!((c.h - 400.0).abs() < f64::EPSILON);
        assert_eq!(c.src_page, 0);
        assert_eq!(c.dst_page, 1);
    }

    #[test]
    fn cropbox_partial_eq_bit_level() {
        let a = CropBox {
            x: 1.0,
            y: 2.0,
            w: 3.0,
            h: 4.0,
            src_page: 0,
            dst_page: 0,
        };
        let b = CropBox {
            x: 1.0,
            y: 2.0,
            w: 3.0,
            h: 4.0,
            src_page: 0,
            dst_page: 0,
        };
        assert_eq!(a, b);
    }

    #[test]
    fn cropbox_src_page_differs() {
        let a = CropBox {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            src_page: 0,
            dst_page: 0,
        };
        let b = CropBox {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            src_page: 1,
            dst_page: 0,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn boxes_writable() {
        let mut n = NativeBoxAccumulator::new();
        n.boxes.push(CropBox {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
            src_page: 0,
            dst_page: -1,
        });
        assert_eq!(n.boxes.len(), 1);
    }

    #[test]
    #[should_panic(expected = "add_box")]
    fn add_box_unimplemented_panics() {
        let mut n = NativeBoxAccumulator::new();
        n.add_box(CropBox {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            src_page: 0,
            dst_page: 0,
        });
    }
}
