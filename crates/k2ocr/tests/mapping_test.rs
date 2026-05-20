//! 集成测试：`k2ocr::mapping` —— OCR word 坐标映射 1:1 复刻 C 版几何变换。
//!
//! 验证项：
//! - C `ocrwords_offset` 等价（平移）
//! - C `ocrwords_scale` 等价（浮点缩放）
//! - C `ocrwords_int_scale` 等价（整数除法缩放）
//! - C `ocrwords_concatenate` 等价（拼接）
//! - C `ocrwords_sort_by_position` 等价（7% 行重叠容差排序）
//! - 端到端流水线场景（k2master.c:707/744/763/1582 完整调用顺序）
//!
//! 单元测试在 `src/mapping.rs` 内嵌；本文件做集成层 smoke + 完整场景验证。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use k2ocr::mapping;
use k2types::OcrWord;

fn w(text: &str, x: f64, y: f64, ww: f64, h: f64) -> OcrWord {
    OcrWord::new(text, x, y, ww, h)
}

// ---- 集成场景 1：单 region OCR 倍率渲染 → 缩回 → offset 到 master ----

#[test]
fn integration_region_int_scale_then_offset() {
    // 模拟：region 用 nocr=2 倍率渲染，OCR 拿到 word 坐标在 2x bitmap 系；
    // 然后 int_scale(2) 缩回原图坐标系；再 offset 平移到 master canvas 坐标。
    let mut words = vec![
        w("Hello", 20.0, 40.0, 60.0, 24.0),
        w("World", 100.0, 40.0, 60.0, 24.0),
    ];

    // 1. ocrwords_int_scale(words, 2)
    mapping::int_scale(&mut words, 2);
    // c=20, w=60 → c2=(20+60-1)/2=39, new_c=10, new_w=30
    assert!((words[0].x - 10.0).abs() < 1e-9);
    assert!((words[0].w - 30.0).abs() < 1e-9);
    // r=40, h=24 → r2=(40+24-1)/2=31, new_r=20, new_h=12
    assert!((words[0].y - 20.0).abs() < 1e-9);
    assert!((words[0].h - 12.0).abs() < 1e-9);

    // 2. ocrwords_offset(words, dw=100, masterinfo.rows+gap_start=200)
    mapping::offset(&mut words, 100.0, 200.0);
    assert!((words[0].x - 110.0).abs() < 1e-9);
    assert!((words[0].y - 220.0).abs() < 1e-9);
    assert!((words[1].x - 150.0).abs() < 1e-9);
    assert!((words[1].y - 220.0).abs() < 1e-9);
}

// ---- 集成场景 2：多 region 拼接到 master + 全局排序 ----

#[test]
fn integration_concatenate_multiple_regions_and_sort() {
    let mut master: Vec<OcrWord> = Vec::new();

    // region 1：第 1 行（master y_bottom ~110）
    let region1 = vec![
        w("first", 50.0, 100.0, 30.0, 10.0),
        w("region1", 100.0, 100.0, 30.0, 10.0),
    ];
    mapping::concatenate(&mut master, region1);

    // region 2：第 2 行（master y_bottom ~210）
    let region2 = vec![
        w("second", 50.0, 200.0, 30.0, 10.0),
        w("region2", 100.0, 200.0, 30.0, 10.0),
    ];
    mapping::concatenate(&mut master, region2);

    assert_eq!(master.len(), 4);

    // 排序：行内按 x 升序 + 跨行按 y 升序
    mapping::sort_by_position(&mut master);
    // 期望顺序：first(50,100) → region1(100,100) → second(50,200) → region2(100,200)
    assert_eq!(master[0].text, "first");
    assert_eq!(master[1].text, "region1");
    assert_eq!(master[2].text, "second");
    assert_eq!(master[3].text, "region2");
}

// ---- 集成场景 3：scale 与 int_scale 在精确整数下结果一致 ----

#[test]
fn scale_and_int_scale_match_for_clean_division() {
    // 选 (c=10, w=40, r=20, h=80) + srat=2：
    //   scale: c2=(10+40-1)*2=98, c=20, w=98-20+1=79; r2=(20+80-1)*2=198, r=40, h=198-40+1=159
    //   int_scale n=... 没有整数等价 with srat=2 因为浮点乘和整数除不同
    // 改成除法测试：scale(0.5) vs int_scale(2)：
    //   scale(0.5): c2=(10+40-1)*0.5=24.5, c=5, w=20; r2=(20+80-1)*0.5=49.5, r=10, h=40
    //   int_scale(2): c2=(10+40-1)/2=24, c=5, w=20; r2=(20+80-1)/2=49, r=10, h=40
    // 整数 c/r 部分一致，w/h 部分 scale 多了 0.5，int_scale 截断。
    let mut a = vec![w("a", 10.0, 20.0, 40.0, 80.0)];
    let mut b = a.clone();
    mapping::scale(&mut a, 0.5);
    mapping::int_scale(&mut b, 2);
    // x/y 完全一致
    assert!((a[0].x - b[0].x).abs() < 1e-9);
    assert!((a[0].y - b[0].y).abs() < 1e-9);
    // w/h 差 0.5（C 整数 truncates）
    assert!((a[0].w - (b[0].w + 0.5)).abs() < 1e-9);
    assert!((a[0].h - (b[0].h + 0.5)).abs() < 1e-9);
}

// ---- 集成场景 4：sort 行重叠容差 ----

#[test]
fn sort_handles_tilted_text_with_small_overlap() {
    // 模拟扫描页中文本行轻微歪斜：相邻 word y_bottom 相差 1-2px，应视为同一行
    let mut words = vec![
        w("c", 200.0, 102.0, 50.0, 20.0), // y_bot=122
        w("a", 10.0, 100.0, 50.0, 20.0),  // y_bot=120
        w("b", 100.0, 101.0, 50.0, 20.0), // y_bot=121
    ];
    mapping::sort_by_position(&mut words);
    // h=20，相邻 y_bot 差 1-2px，重叠 ≥ 90% > 7%，应视为同行按 x 排
    assert_eq!(words[0].text, "a");
    assert_eq!(words[1].text, "b");
    assert_eq!(words[2].text, "c");
}

// ---- 集成场景 5：offset 后再 sort（覆盖 k2master.c:1576+1582 flow）----

#[test]
fn offset_then_sort_simulates_page_publish() {
    // C `k2master.c:1576 ocrwords_offset(ocrwords, w1, 0)` —— 页内水平居中
    // `k2master.c:1582 ocrwords_sort_by_position` —— 按位置排序
    let mut page_words = vec![
        w("right", 200.0, 100.0, 50.0, 10.0),
        w("left", 10.0, 100.0, 50.0, 10.0),
    ];
    mapping::offset(&mut page_words, 50.0, 0.0); // 水平居中 offset
    mapping::sort_by_position(&mut page_words);
    assert_eq!(page_words[0].text, "left"); // x=60
    assert_eq!(page_words[1].text, "right"); // x=250
    assert!((page_words[0].x - 60.0).abs() < 1e-9);
}

// ---- 集成场景 6：concat 多组 + int_scale 验证字段独立 ----

#[test]
fn int_scale_after_concatenate_acts_on_all() {
    let mut dst = vec![w("a", 100.0, 200.0, 40.0, 80.0)];
    let src = vec![w("b", 200.0, 400.0, 40.0, 80.0)];
    mapping::concatenate(&mut dst, src);
    mapping::int_scale(&mut dst, 2);
    // a: c=50, w=(100+40-1)/2 - 50 + 1 = 69-50+1=20 → 20
    assert!((dst[0].x - 50.0).abs() < 1e-9);
    assert!((dst[0].w - 20.0).abs() < 1e-9);
    // b: c=100, w=(200+40-1)/2 - 100 + 1 = 119-100+1=20 → 20
    assert!((dst[1].x - 100.0).abs() < 1e-9);
    assert!((dst[1].w - 20.0).abs() < 1e-9);
}

// ---- 集成场景 7：OcrWord.y_bottom 用于 PDF baseline 换算（Step 9.2 校正）----

#[test]
fn y_bottom_is_correct_pdf_baseline_anchor() {
    // OcrWord.y 是顶部；PDF baseline 应该用 y_bottom = y + h
    let w1 = w("hello", 100.0, 50.0, 80.0, 14.0);
    assert!((w1.y_bottom() - 64.0).abs() < 1e-9);
    // 如果误用 y（顶部），baseline 会偏上 14px
    let dpi = 72.0_f64;
    let page_h_pt = 100.0_f64;
    let baseline_correct = page_h_pt - w1.y_bottom() * 72.0 / dpi; // = 100 - 64 = 36
    let baseline_wrong = page_h_pt - w1.y * 72.0 / dpi; // = 100 - 50 = 50
    assert!((baseline_correct - 36.0).abs() < 1e-9);
    assert!((baseline_wrong - 50.0).abs() < 1e-9);
    assert!((baseline_wrong - baseline_correct - 14.0).abs() < 1e-9);
}

// ---- 集成场景 8：所有变换组合（k2master.c:707-763 完整流水）----

#[test]
fn full_pipeline_int_scale_offset_concatenate_sort() {
    // 完整模拟 C k2master.c:705-774 中 OCR 部分：
    // 1. int_scale(nocr) 缩回原图
    // 2. offset(dw, masterinfo.rows + gap_start) 进入 master 全局坐标
    // 3. concatenate 到 master 桶
    // 4. （flush 时）sort_by_position 准备输出

    // Region 1：被识别 OCR words（2x 倍率渲染坐标系）
    let mut region1 = vec![
        w("hello", 40.0, 80.0, 80.0, 24.0),
        w("world", 200.0, 80.0, 80.0, 24.0),
    ];
    // Region 2：另一 region
    let mut region2 = vec![w("k2pdfopt", 40.0, 80.0, 200.0, 24.0)];

    // Step 1 & 2 for region1: int_scale(2) + offset(dw1=50, master_rows1=100)
    mapping::int_scale(&mut region1, 2);
    mapping::offset(&mut region1, 50.0, 100.0);

    // Step 1 & 2 for region2: int_scale(2) + offset(dw2=20, master_rows2=200)
    mapping::int_scale(&mut region2, 2);
    mapping::offset(&mut region2, 20.0, 200.0);

    // Step 3：concatenate 到 master OcrStaging
    let mut master: Vec<OcrWord> = Vec::new();
    mapping::concatenate(&mut master, region1);
    mapping::concatenate(&mut master, region2);
    assert_eq!(master.len(), 3);

    // Step 4：sort_by_position
    mapping::sort_by_position(&mut master);
    // hello: int_scale(2) → x=20,y=40,w=40,h=12 → offset(50,100) → x=70,y=140,y_bot=152
    // world: int_scale(2) → x=100,y=40,w=40,h=12 → offset(50,100) → x=150,y=140,y_bot=152
    // k2pdfopt: int_scale(2) → x=20,y=40,w=100,h=12 → offset(20,200) → x=40,y=240,y_bot=252
    // 排序：y_bot 升序，同行 x 升序
    //   hello (70, 152) < world (150, 152) < k2pdfopt (40, 252)
    assert_eq!(master[0].text, "hello");
    assert_eq!(master[1].text, "world");
    assert_eq!(master[2].text, "k2pdfopt");
}
