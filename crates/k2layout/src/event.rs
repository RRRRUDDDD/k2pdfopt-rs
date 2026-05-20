//! `event` - Event-driven 备选方案（fallback only，**当前不使用**）。
//!
//! 来源：Spike C（Step 0.4），`spikes/masterinfo-statemachine/src/event.rs`
//!
//! # 决策状态
//!
//! ADR-016（Approved 2026-05-07）：**直接 `&mut self` 调用方式可行**（Rust 分离借用），
//! 因此 event-driven 暂不需要。本文件保留作为以下场景的 fallback 设计参考：
//!
//! 1. M6 wrap/reflow 阶段如果出现无法用分步借用解决的交叉可变借用
//! 2. 未来引入多线程 pipeline 需要跨线程传递事件时
//!
//! 任一触发则切换到 [`EventConsumer`] 模式，记 ADR-018+ 追溯。
//!
//! 详见 `docs/masterinfo-design.md` §6 与 `docs/adr/ADR-016-masterinfo-decomposition.md`。

use crate::master::OcrWord;

/// Layout 引擎产出的事件（fallback 模式专用）。
///
/// **当前不使用** —— 见模块顶部决策状态。
#[derive(Debug)]
#[allow(
    dead_code,
    clippy::large_enum_variant,
    reason = "fallback only — variants reserved for future event-driven mode (see ADR-016)"
)]
pub enum LayoutEvent {
    /// 一块 bitmap 准备好，可以添加到 master canvas。
    BitmapReady {
        /// bitmap 宽度（pixel）
        width: u32,
        /// bitmap 高度（pixel）
        height: u32,
        /// bitmap 像素数据
        pixels: Vec<u8>,
        /// bitmap 的 DPI
        dpi: f64,
        /// 段落对齐（[`crate::master::Justification`] 编码）
        justification: i32,
        /// 白色阈值（区分背景 / 内容）
        whitethresh: i32,
    },
    /// 强制分页。
    ForcePageBreak {
        /// 分页类型（0 = 普通；其他见 C 版 K2PAGEBREAKMARK_TYPE_*）
        mark_type: i32,
    },
    /// Wrap 缓冲区需要 flush。
    WrapFlush,
    /// OCR 识别结果就绪。
    OcrResult {
        /// 一批 OCR words
        words: Vec<OcrWord>,
    },
    /// 源文件处理完毕（流结束）。
    DocumentEnd,
}

/// Event 消费者 trait（fallback 模式专用）。
///
/// **当前不使用**。如果未来切换到 event-driven 模式，[`crate::master::ConvertContext`]
/// 实现此 trait，由 publisher 单线程顺序消费 [`LayoutEvent`] 流。
pub trait EventConsumer {
    /// 消费一个事件。
    fn consume(&mut self, event: LayoutEvent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_construct_compiles() {
        // 仅证明 enum 可构造（fallback 设计验证）
        let _e1 = LayoutEvent::BitmapReady {
            width: 800,
            height: 50,
            pixels: vec![255; 800 * 50],
            dpi: 300.0,
            justification: 0,
            whitethresh: 200,
        };
        let _e2 = LayoutEvent::ForcePageBreak { mark_type: 0 };
        let _e3 = LayoutEvent::WrapFlush;
        let _e4 = LayoutEvent::OcrResult { words: Vec::new() };
        let _e5 = LayoutEvent::DocumentEnd;
    }

    struct NoopConsumer {
        count: u32,
    }

    impl EventConsumer for NoopConsumer {
        fn consume(&mut self, _event: LayoutEvent) {
            self.count += 1;
        }
    }

    #[test]
    fn event_consumer_trait_implementable() {
        let mut c = NoopConsumer { count: 0 };
        c.consume(LayoutEvent::DocumentEnd);
        c.consume(LayoutEvent::WrapFlush);
        assert_eq!(c.count, 2);
    }
}
