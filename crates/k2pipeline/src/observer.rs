//! `observer` —— Step 7.4 落地 ADR-013：进度回调 + 取消令牌。
//!
//! 该模块为 [`crate::ConvertJob`] 提供两个独立而互补的能力：
//!
//! - [`ProgressObserver`] trait：pipeline 各阶段 emit [`ProgressEvent`]，
//!   实现方（CLI 的 `indicatif`、未来的 GUI / Web）按需可视化。
//! - [`CancellationToken`]：包 `Arc<AtomicBool>` 提供 `is_cancelled()` /
//!   `cancel()` / `shared()` API；ConvertJob 在每页前检查，触发即抛
//!   [`crate::ConvertError::Cancelled`]。
//!
//! 两者**默认无副作用**：未注入 observer 时使用 [`NopObserver`]，未注入
//! token 时使用空 [`CancellationToken::default`]。
//!
//! # 设计参考
//!
//! - 来源：`docs/adr/ADR-013-progress-cancel-hooks.md`
//! - C 等价物：`k2master.c::masterinfo_publish` 内置 `verbose` 打印，但无
//!   回调接口。Rust 版抽出 trait 以便复用到非 CLI 场景。
//!
//! # 撤回条件（与 ADR-013 一致）
//!
//! 若 benchmark 显示 `Arc<dyn ProgressObserver>` 动态分发占用 > 1% CPU，
//! 改为泛型 `ConvertJob<O: ProgressObserver>` 静态分发；目前 events
//! 节奏为 per-page（远小于 1 ms 级），动态分发可接受。

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// pipeline 阶段事件。所有变体均可 `Clone` 以便 observer 持久化。
///
/// **不要 panic**：observer 实现侧不应假设事件顺序严格；ConvertJob 内部
/// 实现尽力按 `JobStart → PageStart → (OcrPage)? → PageDone* → PdfWrite* → JobDone`
/// 顺序 emit，但 `Warn` 可在任意阶段穿插。
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// 整体开始 —— `total_pages` 来自 [`k2render::DocumentRenderer::page_count`]
    /// （Step 4.1 落地）。在第一次 mutool 渲染前 emit。
    JobStart { total_pages: usize },
    /// 单页开始处理 —— 在 `MutoolRenderer::render_page` 之前 emit。
    PageStart { source_page: usize },
    /// 单页处理完成 —— 在该源页 add_bitmap + drain_full_pages 完成后 emit；
    /// `dst_pages` 是本次 source page 触发输出的 PaginatorPage 数量
    /// （0 表示该页内容尚未填满 dst canvas，等下次 flush）。
    PageDone {
        source_page: usize,
        dst_pages: usize,
    },
    /// OCR 进度（Step 9.x 落地后才会真正 emit）。
    OcrPage {
        source_page: usize,
        words_found: usize,
    },
    /// 输出 PDF 写入 —— 每 pop_page → writer.add_page 之后 emit；
    /// `dst_pages_written` 是已写入的累计 dst 页数，`total_dst_pages` 是
    /// 当前已知的目标页数（可能随处理增长）。
    PdfWrite {
        dst_pages_written: usize,
        total_dst_pages: usize,
    },
    /// 整体完成 —— `writer.finish()` 成功后 emit；`elapsed_ms` 用
    /// `Instant::elapsed()` 测量。
    JobDone {
        total_input_pages: usize,
        total_output_pages: usize,
        elapsed_ms: u64,
    },
    /// 警告（不阻断流程） —— 例如某页渲染失败但允许继续。
    Warn { message: String },
}

/// 进度观察者 trait。
///
/// 实现侧约束：
///
/// - `Send + Sync`：必须能 `Arc<dyn ProgressObserver>` 跨线程共享
/// - `on_event` 不可 panic，不可阻塞超过单 frame（约 16 ms），否则
///   ConvertJob 主循环吞吐受影响
/// - 实现侧负责自己的内部同步（例如 `Mutex<ProgressBar>`）
pub trait ProgressObserver: Send + Sync {
    /// 接收单个事件。实现可读取 `event` 的引用，但不应持有引用超出函数返回。
    fn on_event(&self, event: &ProgressEvent);
}

/// 空 observer —— 默认使用，零开销。
#[derive(Debug, Default)]
pub struct NopObserver;

impl ProgressObserver for NopObserver {
    #[inline]
    fn on_event(&self, _event: &ProgressEvent) {
        // intentionally empty
    }
}

/// 取消令牌：协作式取消（cooperative cancellation）。
///
/// 内部包 `Arc<AtomicBool>`，可在多个所有者（主 ConvertJob、ctrl-c handler、
/// GUI 取消按钮 callback）之间共享。**`Default` 即可创建未取消状态。**
///
/// # 语义
///
/// - `cancel()` 永久翻转标志（无法撤销），后续所有 `is_cancelled()` 返 `true`
/// - 检查点位于：每个 source page 渲染前、每次 writer.add_page 前、
///   writer.finish 前。详见 `ConvertJob::check_cancel` 内联调用点
/// - 触发取消后，ConvertJob 立刻 `Err(ConvertError::Cancelled)` 退出；
///   `tempfile::TempDir` / `LopdfWriter` 等 RAII 资源由 Drop 清理
///
/// # 使用示例
///
/// ```no_run
/// use k2pipeline::{CancellationToken, ConvertJob, ConvertJobConfig};
/// use std::sync::Arc;
///
/// let token = CancellationToken::new();
/// let token_for_handler = token.clone();
/// // 假设的 Ctrl-C 处理（实际通过 ctrlc crate 注入）
/// std::thread::spawn(move || {
///     token_for_handler.cancel();
/// });
///
/// let job = ConvertJob::new("a.pdf", "b.pdf", ConvertJobConfig::default())
///     .with_cancel(token);
/// let _ = job.run();
/// ```
#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: Arc<AtomicBool>,
}

impl CancellationToken {
    /// 构造未取消的令牌。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 从已有 `Arc<AtomicBool>` 构造，便于复用外部已存在的标志。
    #[must_use]
    pub fn from_atomic(flag: Arc<AtomicBool>) -> Self {
        Self { inner: flag }
    }

    /// 当前是否已取消。
    #[inline]
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::Relaxed)
    }

    /// 翻转取消标志（不可撤销）。
    pub fn cancel(&self) {
        self.inner.store(true, Ordering::Relaxed);
    }

    /// 返回内部 `Arc<AtomicBool>` 的克隆，便于注入到 `ctrlc` 等需要原始
    /// `Arc<AtomicBool>` API 的回调中（例如 ADR-013 的示例代码）。
    #[must_use]
    pub fn shared(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.inner)
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// 调试 / 测试用 observer：把所有事件记到 `Mutex<Vec>`。
///
/// 仅在 `cfg(any(test, debug_assertions))` 之外也保留，便于下游 crate 集成测试
/// （集成测试不能依赖 `#[cfg(test)]` 私有项）。生产代码不应使用，因为
/// `Mutex` 在高频事件下有锁竞争。
#[derive(Default)]
pub struct RecordingObserver {
    events: Mutex<Vec<ProgressEvent>>,
}

impl RecordingObserver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 取出当前已记录的事件副本。
    ///
    /// 返回 `Vec<ProgressEvent>` 而非 `&[..]`，避免锁悬挂。
    #[must_use]
    pub fn snapshot(&self) -> Vec<ProgressEvent> {
        match self.events.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// 记录数量。
    #[must_use]
    pub fn len(&self) -> usize {
        match self.events.lock() {
            Ok(g) => g.len(),
            Err(p) => p.into_inner().len(),
        }
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 清空事件列表。
    pub fn clear(&self) {
        if let Ok(mut g) = self.events.lock() {
            g.clear();
        }
    }
}

impl ProgressObserver for RecordingObserver {
    fn on_event(&self, event: &ProgressEvent) {
        if let Ok(mut g) = self.events.lock() {
            g.push(event.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn nop_observer_silently_consumes_events() {
        let obs = NopObserver;
        // 多次调用不应 panic 也不应做任何事
        obs.on_event(&ProgressEvent::JobStart { total_pages: 10 });
        obs.on_event(&ProgressEvent::JobDone {
            total_input_pages: 10,
            total_output_pages: 5,
            elapsed_ms: 0,
        });
    }

    #[test]
    fn cancellation_token_default_is_not_cancelled() {
        let t = CancellationToken::default();
        assert!(!t.is_cancelled());
    }

    #[test]
    fn cancellation_token_cancel_flips_flag() {
        let t = CancellationToken::new();
        assert!(!t.is_cancelled());
        t.cancel();
        assert!(t.is_cancelled());
    }

    #[test]
    fn cancellation_token_cancel_is_idempotent() {
        let t = CancellationToken::new();
        t.cancel();
        t.cancel();
        t.cancel();
        assert!(t.is_cancelled());
    }

    #[test]
    fn cancellation_token_clone_shares_state() {
        let a = CancellationToken::new();
        let b = a.clone();
        assert!(!a.is_cancelled());
        assert!(!b.is_cancelled());
        b.cancel();
        assert!(a.is_cancelled());
        assert!(b.is_cancelled());
    }

    #[test]
    fn cancellation_token_from_atomic_reuses_flag() {
        let flag = Arc::new(AtomicBool::new(true));
        let t = CancellationToken::from_atomic(Arc::clone(&flag));
        assert!(t.is_cancelled());
        // 通过 token 取消应反映到原 flag
        let t2 = CancellationToken::from_atomic(Arc::clone(&flag));
        assert!(t2.is_cancelled());
    }

    #[test]
    fn cancellation_token_shared_returns_same_arc() {
        let t = CancellationToken::new();
        let a1 = t.shared();
        let a2 = t.shared();
        // 修改其一应同步到另一
        a1.store(true, Ordering::Relaxed);
        assert!(a2.load(Ordering::Relaxed));
        assert!(t.is_cancelled());
    }

    #[test]
    fn cancellation_token_debug_shows_state() {
        let t = CancellationToken::new();
        let s = format!("{:?}", t);
        assert!(s.contains("cancelled"));
        assert!(s.contains("false"));
        t.cancel();
        let s = format!("{:?}", t);
        assert!(s.contains("true"));
    }

    #[test]
    fn recording_observer_captures_events_in_order() {
        let obs = RecordingObserver::new();
        obs.on_event(&ProgressEvent::JobStart { total_pages: 3 });
        obs.on_event(&ProgressEvent::PageStart { source_page: 0 });
        obs.on_event(&ProgressEvent::PageDone {
            source_page: 0,
            dst_pages: 1,
        });
        let snap = obs.snapshot();
        assert_eq!(snap.len(), 3);
        assert!(matches!(
            &snap[0],
            ProgressEvent::JobStart { total_pages: 3 }
        ));
        assert!(matches!(
            &snap[1],
            ProgressEvent::PageStart { source_page: 0 }
        ));
        assert!(matches!(
            &snap[2],
            ProgressEvent::PageDone {
                source_page: 0,
                dst_pages: 1
            }
        ));
    }

    #[test]
    fn recording_observer_len_and_is_empty() {
        let obs = RecordingObserver::new();
        assert!(obs.is_empty());
        assert_eq!(obs.len(), 0);
        obs.on_event(&ProgressEvent::Warn {
            message: "hi".into(),
        });
        assert!(!obs.is_empty());
        assert_eq!(obs.len(), 1);
    }

    #[test]
    fn recording_observer_clear_empties_log() {
        let obs = RecordingObserver::new();
        obs.on_event(&ProgressEvent::PageStart { source_page: 0 });
        obs.on_event(&ProgressEvent::PageStart { source_page: 1 });
        assert_eq!(obs.len(), 2);
        obs.clear();
        assert_eq!(obs.len(), 0);
    }

    #[test]
    fn recording_observer_via_trait_object() {
        let obs: Arc<dyn ProgressObserver> = Arc::new(RecordingObserver::new());
        obs.on_event(&ProgressEvent::JobDone {
            total_input_pages: 10,
            total_output_pages: 8,
            elapsed_ms: 1234,
        });
        // 通过 trait object 调用应正常工作；可不取回 inner（仅验证 dispatch ok）
        // 实际取回需要 downcast，这里只验证 dispatch
    }

    #[test]
    fn progress_event_clone_roundtrip() {
        let e1 = ProgressEvent::Warn {
            message: "test warning".into(),
        };
        let e2 = e1.clone();
        if let ProgressEvent::Warn { message } = &e2 {
            assert_eq!(message, "test warning");
        } else {
            panic!("clone broke variant");
        }
    }

    #[test]
    fn cancellation_token_thread_safety() {
        let t = CancellationToken::new();
        let t2 = t.clone();
        let h = std::thread::spawn(move || {
            t2.cancel();
        });
        h.join().unwrap();
        assert!(t.is_cancelled());
    }
}
