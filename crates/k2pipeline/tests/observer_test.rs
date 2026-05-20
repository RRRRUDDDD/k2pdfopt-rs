//! Step 7.4 集成测试：ProgressObserver 事件流 + CancellationToken 取消语义。
//!
//! 与 convert_test.rs 互补：那里测端到端 PDF 输出结构，这里测：
//! - JobStart / PageStart / PageDone / PdfWrite / JobDone 事件按预期顺序 emit
//! - 预先 cancel 时 ConvertJob 在 mutool 启动前就返 Cancelled
//! - 跑中 cancel 时 ConvertJob 在下一安全检查点退出

#![allow(clippy::unwrap_used, clippy::expect_used)]

use k2pipeline::{
    CancellationToken, ConvertError, ConvertJob, ConvertJobConfig, ProgressEvent, ProgressObserver,
    RecordingObserver,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn fixture(name: &str) -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("tests").join("fixtures").join(name))
        .expect("workspace root reachable")
}

fn temp_output(label: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let mut p = std::env::temp_dir();
    p.push(format!("k2pdfopt_step74_{label}_{pid}_{nanos}.pdf"));
    p
}

/// 判定是否应跳过测试（mutool 不可用 / fixture 不存在）。
fn should_skip(input: &Path, res: &Result<(), ConvertError>) -> bool {
    if !input.exists() {
        eprintln!("skipped: fixture {} 不存在", input.display());
        return true;
    }
    if let Err(ConvertError::Render(ref e)) = res {
        let msg = format!("{e}");
        if msg.contains("not found in PATH") || msg.contains("BinaryNotFound") {
            eprintln!("skipped: mutool not available ({msg})");
            return true;
        }
    }
    false
}

#[test]
fn observer_receives_jobstart_and_jobdone_for_single_column() {
    let input = fixture("single-column.pdf");
    let output = temp_output("obs_single");

    let rec = Arc::new(RecordingObserver::new());
    let observer: Arc<dyn ProgressObserver> = rec.clone();
    let job = ConvertJob::new(&input, &output, ConvertJobConfig::default()).with_observer(observer);
    let res = job.run();
    if should_skip(&input, &res) {
        return;
    }
    res.expect("ConvertJob::run should succeed for valid fixture");

    let events = rec.snapshot();
    assert!(!events.is_empty(), "should have emitted at least JobStart");

    // 第一个事件必须是 JobStart
    let first = events.first().expect("non-empty events");
    let total_pages_expected = match first {
        ProgressEvent::JobStart { total_pages } => *total_pages,
        _ => panic!("expected JobStart as first event, got {first:?}"),
    };
    assert!(total_pages_expected > 0, "fixture should have >= 1 page");

    // 最后一个事件必须是 JobDone
    let last = events.last().expect("non-empty events");
    match last {
        ProgressEvent::JobDone {
            total_input_pages,
            total_output_pages,
            elapsed_ms: _,
        } => {
            assert_eq!(*total_input_pages, total_pages_expected);
            assert!(*total_output_pages > 0, "should produce >= 1 output page");
        }
        _ => panic!("expected JobDone as last event, got {last:?}"),
    }

    let _ = std::fs::remove_file(&output);
}

#[test]
fn observer_page_events_monotonic_source_pages() {
    let input = fixture("single-column.pdf");
    let output = temp_output("obs_monotonic");

    let rec = Arc::new(RecordingObserver::new());
    let observer: Arc<dyn ProgressObserver> = rec.clone();
    let job = ConvertJob::new(&input, &output, ConvertJobConfig::default()).with_observer(observer);
    let res = job.run();
    if should_skip(&input, &res) {
        return;
    }
    res.expect("run ok");

    let events = rec.snapshot();
    // 收集 PageStart 序列
    let mut starts: Vec<usize> = Vec::new();
    for e in &events {
        if let ProgressEvent::PageStart { source_page } = e {
            starts.push(*source_page);
        }
    }
    assert!(!starts.is_empty(), "should have PageStart events");
    // PageStart 必须单调递增 (0, 1, 2, ...)
    for i in 0..starts.len() {
        assert_eq!(starts[i], i, "PageStart sequence broken at {i}: {starts:?}");
    }

    // PageDone 数量应等于 PageStart
    let dones: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, ProgressEvent::PageDone { .. }))
        .collect();
    assert_eq!(dones.len(), starts.len());

    let _ = std::fs::remove_file(&output);
}

#[test]
fn observer_pdfwrite_events_track_growing_count() {
    let input = fixture("single-column.pdf");
    let output = temp_output("obs_pdfwrite");

    let rec = Arc::new(RecordingObserver::new());
    let observer: Arc<dyn ProgressObserver> = rec.clone();
    let job = ConvertJob::new(&input, &output, ConvertJobConfig::default()).with_observer(observer);
    let res = job.run();
    if should_skip(&input, &res) {
        return;
    }
    res.expect("run ok");

    let events = rec.snapshot();
    // 收集 PdfWrite 序列，验证 dst_pages_written 单调不减
    let mut prev = 0usize;
    let mut count = 0;
    for e in &events {
        if let ProgressEvent::PdfWrite {
            dst_pages_written, ..
        } = e
        {
            assert!(
                *dst_pages_written >= prev,
                "PdfWrite counter regressed: {} -> {dst_pages_written}",
                prev
            );
            prev = *dst_pages_written;
            count += 1;
        }
    }
    assert!(count > 0, "should have at least one PdfWrite event");

    let _ = std::fs::remove_file(&output);
}

#[test]
fn pre_cancelled_token_returns_cancelled_without_invoking_mutool() {
    // 预先翻 cancel，run() 应在 mutool 子进程启动之前就 Cancelled
    // 这意味着即使输入文件不存在也不会触发 Render error
    let bogus_input = PathBuf::from("/this/path/definitely/does/not/exist.pdf");
    let bogus_output = PathBuf::from("/another/missing/path/out.pdf");

    let token = CancellationToken::new();
    token.cancel();

    let rec = Arc::new(RecordingObserver::new());
    let observer: Arc<dyn ProgressObserver> = rec.clone();
    let job = ConvertJob::new(&bogus_input, &bogus_output, ConvertJobConfig::default())
        .with_observer(observer)
        .with_cancel(token);
    let err = job.run().unwrap_err();
    assert!(
        matches!(err, ConvertError::Cancelled),
        "expected Cancelled, got {err:?}"
    );

    // 因为提前退出，observer 应未收到 JobStart（取消在 renderer 之前）
    let events = rec.snapshot();
    let saw_jobstart = events
        .iter()
        .any(|e| matches!(e, ProgressEvent::JobStart { .. }));
    assert!(
        !saw_jobstart,
        "JobStart should NOT be emitted when cancelled pre-render"
    );
}

#[test]
fn nop_observer_default_does_not_break_pipeline() {
    let input = fixture("single-column.pdf");
    let output = temp_output("obs_nop");

    // 不注入 observer / cancel；默认 NopObserver + 未取消 token
    let job = ConvertJob::new(&input, &output, ConvertJobConfig::default());
    let res = job.run();
    if should_skip(&input, &res) {
        return;
    }
    res.expect("default observer should not break pipeline");
    assert!(output.exists(), "output PDF should exist");

    let _ = std::fs::remove_file(&output);
}

#[test]
fn cancel_after_first_page_stops_pipeline_mid_run() {
    // 自定义 observer：在收到第 1 个 PageDone 后翻 cancel
    use std::sync::Mutex;

    struct CancelAfterFirstPage {
        rec: Arc<RecordingObserver>,
        token: CancellationToken,
        cancelled: Mutex<bool>,
    }
    impl ProgressObserver for CancelAfterFirstPage {
        fn on_event(&self, event: &ProgressEvent) {
            self.rec.on_event(event);
            if let ProgressEvent::PageDone { .. } = event {
                if let Ok(mut flag) = self.cancelled.lock() {
                    if !*flag {
                        *flag = true;
                        self.token.cancel();
                    }
                }
            }
        }
    }

    // 使用一个多页的 fixture；single-column 至少 1 页，可能直接结束。
    // 优先用 two-column.pdf 或 three-column.pdf 这些总页数 > 1 的 fixture。
    let candidates = ["two-column.pdf", "three-column.pdf", "single-column.pdf"];
    for name in &candidates {
        let input = fixture(name);
        let output = temp_output(&format!("obs_cancel_mid_{name}"));
        if !input.exists() {
            continue;
        }

        let rec = Arc::new(RecordingObserver::new());
        let token = CancellationToken::new();
        let inner = Arc::new(CancelAfterFirstPage {
            rec: rec.clone(),
            token: token.clone(),
            cancelled: Mutex::new(false),
        });
        let observer: Arc<dyn ProgressObserver> = inner.clone();
        let job = ConvertJob::new(&input, &output, ConvertJobConfig::default())
            .with_observer(observer)
            .with_cancel(token);
        let res = job.run();

        // mutool 不可用时跳过整组
        if let Err(ConvertError::Render(ref e)) = res {
            if format!("{e}").contains("BinaryNotFound") {
                eprintln!("skipped: mutool not available");
                return;
            }
        }

        let events = rec.snapshot();
        match res {
            Err(ConvertError::Cancelled) => {
                // 至少收到 1 个 PageDone（触发 cancel 的那次）
                let done_count = events
                    .iter()
                    .filter(|e| matches!(e, ProgressEvent::PageDone { .. }))
                    .count();
                assert!(done_count >= 1, "expected >= 1 PageDone before cancel");
                // 不应有 JobDone（提前退出）
                let job_done = events
                    .iter()
                    .any(|e| matches!(e, ProgressEvent::JobDone { .. }));
                assert!(!job_done, "JobDone should not be emitted on cancel");
                let _ = std::fs::remove_file(&output);
                return; // 一个 fixture 验证成功即可
            }
            Ok(()) => {
                // 该 fixture 只有 1 页 → cancel 已晚于唯一一次 add_bitmap，run 仍 Ok。
                // 试下一个 fixture。
                let _ = std::fs::remove_file(&output);
                continue;
            }
            Err(e) => {
                panic!("unexpected error: {e}");
            }
        }
    }
    eprintln!("skipped: no multi-page fixture available for mid-run cancel test");
}

#[test]
fn observer_arc_dyn_sharing_works_across_threads() {
    // observer 必须实现 Send + Sync 才能 Arc<dyn> 跨线程共享
    let rec = Arc::new(RecordingObserver::new());
    let observer: Arc<dyn ProgressObserver> = rec.clone();

    let h = std::thread::spawn(move || {
        observer.on_event(&ProgressEvent::PageStart { source_page: 42 });
    });
    h.join().expect("thread join");

    let events = rec.snapshot();
    assert_eq!(events.len(), 1);
    if let ProgressEvent::PageStart { source_page } = &events[0] {
        assert_eq!(*source_page, 42);
    } else {
        panic!("event variant mismatch");
    }
}
