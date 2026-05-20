//! Step 7.2 集成测试 — Bitmap PDF Writer 端到端验证。
//!
//! 覆盖矩阵：
//! - PDF 结构合法性（用 `lopdf` 重新加载）
//! - 多页 / 单页 / 不同 PixelFormat
//! - JPEG passthrough vs Flate 兜底
//! - OCR layer Page.Contents 数组转换
//! - 嵌套 outline tree（First/Last/Parent/Next/Prev/Count）
//! - 边界 / 错误路径
//!
//! 不依赖外部二进制（mutool/Sumatra）—— 完全靠 lopdf 自检 + 文件大小启发式。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use k2pdfout::{LopdfWriter, PdfWriter};
use k2settings::OcrVisibility;
use k2types::{Bitmap, OcrWord, OutlineEntry, OutputPage, PixelFormat};
use lopdf::{Document as LDoc, Object};
use tempfile::tempdir;

fn make_gray_bitmap(w: u32, h: u32, val: u8) -> Bitmap {
    let mut b = Bitmap::new(w, h, 150.0, PixelFormat::Gray8).unwrap();
    b.fill_byte(val);
    b
}

fn make_rgb_bitmap(w: u32, h: u32, r: u8, g: u8, b: u8) -> Bitmap {
    let mut bmp = Bitmap::new(w, h, 150.0, PixelFormat::Rgb8).unwrap();
    bmp.fill_rgb(r, g, b);
    bmp
}

// ===================== 端到端：PDF 结构合法性 =====================

#[test]
fn integration_single_page_round_trip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("single.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(300, 200, 220);
        let page = OutputPage::from_bitmap(0, bmp, 150.0);
        w.add_page(&page).unwrap();
        Box::new(w).finish().unwrap();
    }
    // 重新加载并验证
    let doc = LDoc::load(&path).expect("load output PDF");
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 1, "应当只有 1 页");
}

#[test]
fn integration_three_pages_count_matches() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("three.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        for i in 0..3 {
            let bmp = make_gray_bitmap(100, 100, (i as u8) * 50 + 50);
            w.add_page(&OutputPage::from_bitmap(i, bmp, 100.0)).unwrap();
        }
        Box::new(w).finish().unwrap();
    }
    let doc = LDoc::load(&path).expect("load output PDF");
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 3, "应当有 3 页");
}

#[test]
fn integration_mediabox_dimensions_match_dpi() {
    // 200x100 像素 @ 100 DPI → MediaBox 应为 [0, 0, 144, 72]
    let dir = tempdir().unwrap();
    let path = dir.path().join("mediabox.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(200, 100, 200);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        Box::new(w).finish().unwrap();
    }

    let doc = LDoc::load(&path).expect("load");
    let pages = doc.get_pages();
    let (_, first_page_id) = pages.into_iter().next().expect("at least one page");
    let page_obj = doc.get_object(first_page_id).expect("page object");
    let page_dict = match page_obj {
        Object::Dictionary(d) => d,
        _ => panic!("page is not dict"),
    };
    let mbox = page_dict.get(b"MediaBox").expect("MediaBox");
    let arr = match mbox {
        Object::Array(a) => a,
        _ => panic!("MediaBox not array"),
    };
    assert_eq!(arr.len(), 4);
    // 第 3 个元素 = width_pt = 144；第 4 个 = height_pt = 72
    let width = object_to_f32(&arr[2]);
    let height = object_to_f32(&arr[3]);
    assert!(
        (width - 144.0).abs() < 0.5,
        "width_pt 应当接近 144，实际 {width}"
    );
    assert!(
        (height - 72.0).abs() < 0.5,
        "height_pt 应当接近 72，实际 {height}"
    );
}

fn object_to_f32(o: &Object) -> f32 {
    match o {
        Object::Integer(i) => *i as f32,
        Object::Real(r) => *r,
        _ => f32::NAN,
    }
}

// ===================== JPEG 与 Flate 编码路径 =====================

#[test]
fn integration_jpeg_path_uses_dctdecode() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("jpeg.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_rgb_bitmap(120, 80, 255, 128, 0);
        let mut page = OutputPage::from_bitmap(0, bmp, 100.0);
        page.jpeg_quality = 85;
        w.add_page(&page).unwrap();
        Box::new(w).finish().unwrap();
    }
    let raw = std::fs::read(&path).unwrap();
    // PDF 中应能找到 /DCTDecode 关键字
    assert!(
        raw.windows(b"/DCTDecode".len()).any(|w| w == b"/DCTDecode"),
        "JPEG 模式应当用 /DCTDecode filter"
    );
}

#[test]
fn integration_flate_path_uses_flatedecode() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("flate.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(100, 100, 200);
        let mut page = OutputPage::from_bitmap(0, bmp, 100.0);
        page.jpeg_quality = -1;
        w.add_page(&page).unwrap();
        Box::new(w).finish().unwrap();
    }
    let raw = std::fs::read(&path).unwrap();
    assert!(
        raw.windows(b"/FlateDecode".len())
            .any(|w| w == b"/FlateDecode"),
        "Flate 模式应当用 /FlateDecode filter"
    );
}

#[test]
fn integration_jpeg_and_flate_both_produce_valid_pdfs() {
    // 不做 "JPEG 一定比 Flate 小" 的硬性假设（取决于图像熵）。
    // 仅校验两条路径都能产生可加载的 PDF。性能 baseline 推迟到 Step 10.3。
    let dir = tempdir().unwrap();
    let path_j = dir.path().join("nat_jpeg.pdf");
    let path_f = dir.path().join("nat_flate.pdf");

    let bmp = make_rgb_bitmap(120, 90, 200, 150, 100);

    {
        let mut w = LopdfWriter::new(&path_j).unwrap();
        let mut page = OutputPage::from_bitmap(0, bmp.clone(), 100.0);
        page.jpeg_quality = 75;
        w.add_page(&page).unwrap();
        Box::new(w).finish().unwrap();
    }
    {
        let mut w = LopdfWriter::new(&path_f).unwrap();
        let mut page = OutputPage::from_bitmap(0, bmp, 100.0);
        page.jpeg_quality = -1;
        w.add_page(&page).unwrap();
        Box::new(w).finish().unwrap();
    }

    let doc_j = LDoc::load(&path_j).expect("load jpeg PDF");
    let doc_f = LDoc::load(&path_f).expect("load flate PDF");
    assert_eq!(doc_j.get_pages().len(), 1);
    assert_eq!(doc_f.get_pages().len(), 1);
}

// ===================== Pixel formats =====================

#[test]
fn integration_gray_rgb_rgba_all_produce_valid_pdf() {
    for (idx, fmt) in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8]
        .iter()
        .enumerate()
    {
        let dir = tempdir().unwrap();
        let path = dir.path().join(format!("fmt_{idx}.pdf"));
        let mut bmp = Bitmap::new(60, 40, 100.0, *fmt).unwrap();
        bmp.fill_rgb(100, 150, 200);
        let page = OutputPage::from_bitmap(0, bmp, 100.0);
        {
            let mut w = LopdfWriter::new(&path).unwrap();
            w.add_page(&page).unwrap();
            Box::new(w).finish().unwrap();
        }
        // 加载验证
        let doc = LDoc::load(&path).expect("load");
        assert_eq!(doc.get_pages().len(), 1, "fmt {fmt:?} 应当 1 页");
    }
}

// ===================== OCR layer =====================

#[test]
fn integration_ocr_layer_adds_contents_array() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ocr.pdf");
    {
        // Step 11.11 P1-2：显式 SHOW_OCR_TEXT 让 OCR layer 实际写入（默认 SHOW_SOURCE 短路）
        let mut w = LopdfWriter::new(&path)
            .unwrap()
            .with_ocr_visibility(OcrVisibility::from_bits(
                OcrVisibility::SHOW_SOURCE.bits() | OcrVisibility::SHOW_OCR_TEXT.bits(),
            ));
        let bmp = make_gray_bitmap(300, 200, 255);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        let words = vec![
            OcrWord::new("hello", 30.0, 80.0, 50.0, 14.0),
            OcrWord::new("world", 90.0, 80.0, 50.0, 14.0),
        ];
        w.add_ocr_layer(&words).unwrap();
        Box::new(w).finish().unwrap();
    }

    let doc = LDoc::load(&path).expect("load");
    let pages = doc.get_pages();
    let (_, page_id) = pages.into_iter().next().unwrap();
    let page_obj = doc.get_object(page_id).unwrap();
    let dict = match page_obj {
        Object::Dictionary(d) => d,
        _ => panic!("not dict"),
    };
    let contents = dict.get(b"Contents").expect("Contents");
    match contents {
        Object::Array(arr) => assert_eq!(arr.len(), 2, "应当有 2 个 content stream (bitmap + OCR)"),
        _ => panic!("Contents 应当是数组（bitmap + OCR），实际：{contents:?}"),
    }
}

#[test]
fn integration_multiple_ocr_calls_accumulate() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ocr_multi.pdf");
    {
        // Step 11.11 P1-2：显式 SHOW_OCR_TEXT（默认 SHOW_SOURCE 不写文字层）
        let mut w = LopdfWriter::new(&path)
            .unwrap()
            .with_ocr_visibility(OcrVisibility::from_bits(
                OcrVisibility::SHOW_SOURCE.bits() | OcrVisibility::SHOW_OCR_TEXT.bits(),
            ));
        let bmp = make_gray_bitmap(300, 200, 255);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        w.add_ocr_layer(&[OcrWord::new("a", 10.0, 50.0, 20.0, 12.0)])
            .unwrap();
        w.add_ocr_layer(&[OcrWord::new("b", 40.0, 50.0, 20.0, 12.0)])
            .unwrap();
        w.add_ocr_layer(&[OcrWord::new("c", 70.0, 50.0, 20.0, 12.0)])
            .unwrap();
        Box::new(w).finish().unwrap();
    }
    let doc = LDoc::load(&path).expect("load");
    let pages = doc.get_pages();
    let (_, page_id) = pages.into_iter().next().unwrap();
    let page_obj = doc.get_object(page_id).unwrap();
    let dict = match page_obj {
        Object::Dictionary(d) => d,
        _ => panic!("not dict"),
    };
    let contents = dict.get(b"Contents").unwrap();
    match contents {
        Object::Array(arr) => assert_eq!(arr.len(), 4, "1 bitmap + 3 OCR streams = 4"),
        _ => panic!("Contents should be array"),
    }
}

#[test]
fn integration_ocr_layer_for_correct_page_only() {
    // 多页 PDF：OCR 应该只附在 last add_page 的那一页
    let dir = tempdir().unwrap();
    let path = dir.path().join("ocr_page2.pdf");
    {
        // Step 11.11 P1-2：显式 SHOW_OCR_TEXT（默认 SHOW_SOURCE 不写文字层）
        let mut w = LopdfWriter::new(&path)
            .unwrap()
            .with_ocr_visibility(OcrVisibility::from_bits(
                OcrVisibility::SHOW_SOURCE.bits() | OcrVisibility::SHOW_OCR_TEXT.bits(),
            ));
        // page 0
        let bmp1 = make_gray_bitmap(100, 100, 255);
        w.add_page(&OutputPage::from_bitmap(0, bmp1, 100.0))
            .unwrap();
        // page 1
        let bmp2 = make_gray_bitmap(100, 100, 220);
        w.add_page(&OutputPage::from_bitmap(1, bmp2, 100.0))
            .unwrap();
        // OCR 应当属于 page 1（最后添加）
        w.add_ocr_layer(&[OcrWord::new("p2", 10.0, 50.0, 30.0, 12.0)])
            .unwrap();
        Box::new(w).finish().unwrap();
    }

    let doc = LDoc::load(&path).expect("load");
    let pages: Vec<_> = doc.get_pages().into_iter().collect();
    assert_eq!(pages.len(), 2);

    // page 0 Contents 应当是单 ref
    let p0 = pages[0].1;
    let p0_obj = doc.get_object(p0).unwrap();
    let p0_dict = match p0_obj {
        Object::Dictionary(d) => d,
        _ => panic!(),
    };
    match p0_dict.get(b"Contents").unwrap() {
        Object::Reference(_) => { /* OK - 单 ref */ }
        Object::Array(arr) => panic!("page 0 不该有 OCR layer, 实际 array len={}", arr.len()),
        _ => panic!("unexpected page 0 Contents type"),
    }

    // page 1 Contents 应当是 array of 2
    let p1 = pages[1].1;
    let p1_obj = doc.get_object(p1).unwrap();
    let p1_dict = match p1_obj {
        Object::Dictionary(d) => d,
        _ => panic!(),
    };
    match p1_dict.get(b"Contents").unwrap() {
        Object::Array(arr) => assert_eq!(arr.len(), 2, "page 1 应有 bitmap+OCR=2"),
        _ => panic!("page 1 Contents should be array"),
    }
}

// ===================== Outline 嵌套树 =====================

#[test]
fn integration_outline_root_object_exists() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outline_root.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(50, 50, 200);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        w.add_outline(OutlineEntry::top_level("Chapter A", 0))
            .unwrap();
        Box::new(w).finish().unwrap();
    }
    let doc = LDoc::load(&path).expect("load");
    let catalog = doc.catalog().expect("catalog");
    let outlines_ref = catalog.get(b"Outlines");
    assert!(outlines_ref.is_ok(), "Catalog 应当有 /Outlines");
}

#[test]
fn integration_outline_nested_tree_structure() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outline_nested.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        for i in 0..4 {
            let bmp = make_gray_bitmap(50, 50, 100 + (i as u8) * 30);
            w.add_page(&OutputPage::from_bitmap(i, bmp, 100.0)).unwrap();
        }
        // Part 1 (idx 0)
        //   Chap 1 (idx 1, parent 0)
        //   Chap 2 (idx 2, parent 0)
        // Part 2 (idx 3)
        w.add_outline(OutlineEntry::top_level("Part 1", 0)).unwrap();
        w.add_outline(OutlineEntry::child("Chap 1", 1, 0)).unwrap();
        w.add_outline(OutlineEntry::child("Chap 2", 2, 0)).unwrap();
        w.add_outline(OutlineEntry::top_level("Part 2", 3)).unwrap();
        Box::new(w).finish().unwrap();
    }

    let doc = LDoc::load(&path).expect("load");
    let catalog = doc.catalog().expect("catalog");
    let outlines_id = match catalog.get(b"Outlines").unwrap() {
        Object::Reference(id) => *id,
        _ => panic!("Outlines should be reference"),
    };
    let outlines_obj = doc.get_object(outlines_id).unwrap();
    let outlines_dict = match outlines_obj {
        Object::Dictionary(d) => d,
        _ => panic!("Outlines not dict"),
    };
    // 顶层 Count 应当是 2 (Part 1 + Part 2)
    let count = match outlines_dict.get(b"Count").unwrap() {
        Object::Integer(c) => *c,
        _ => panic!("Count not integer"),
    };
    assert_eq!(count, 2, "Outlines 顶层 Count 应当是 2");
    // First 与 Last 都存在
    assert!(outlines_dict.get(b"First").is_ok(), "Outlines /First");
    assert!(outlines_dict.get(b"Last").is_ok(), "Outlines /Last");
}

#[test]
fn integration_outline_with_unmapped_dst_page() {
    // dst_page = -1 应不写 /Dest 但 entry 依然要被记录
    let dir = tempdir().unwrap();
    let path = dir.path().join("outline_unmapped.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(50, 50, 200);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        let mut entry = OutlineEntry::top_level("Unmapped", -1);
        entry.dst_page = -1;
        w.add_outline(entry).unwrap();
        Box::new(w).finish().unwrap();
    }
    // 加载不该失败
    let doc = LDoc::load(&path).expect("load");
    assert!(doc.catalog().is_ok());
}

// ===================== UTF-8 / 边界 =====================

#[test]
fn integration_outline_with_utf8_title() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("utf8.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(50, 50, 200);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        w.add_outline(OutlineEntry::top_level("中文章节", 0))
            .unwrap();
        Box::new(w).finish().unwrap();
    }
    // 加载不该失败（UTF-8 字节按 literal 写入）
    let doc = LDoc::load(&path).expect("load");
    assert!(doc.catalog().is_ok());
}

#[test]
fn integration_ocr_with_utf8_text() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ocr_utf8.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(100, 100, 255);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        w.add_ocr_layer(&[OcrWord::new("中文", 10.0, 50.0, 30.0, 14.0)])
            .unwrap();
        Box::new(w).finish().unwrap();
    }
    let doc = LDoc::load(&path).expect("load");
    assert_eq!(doc.get_pages().len(), 1);
}

#[test]
fn integration_large_page_count() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("many.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        for i in 0..20u32 {
            let bmp = make_gray_bitmap(40, 40, ((i * 12) % 255) as u8);
            w.add_page(&OutputPage::from_bitmap(i, bmp, 100.0)).unwrap();
        }
        Box::new(w).finish().unwrap();
    }
    let doc = LDoc::load(&path).expect("load");
    assert_eq!(doc.get_pages().len(), 20);
}

#[test]
fn integration_pdf_header_is_valid() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("header.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(50, 50, 200);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        Box::new(w).finish().unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF-"), "PDF header 必须以 %PDF- 开头");
    // EOF marker
    let tail = &bytes[bytes.len().saturating_sub(16)..];
    assert!(
        tail.windows(b"%%EOF".len()).any(|w| w == b"%%EOF"),
        "PDF 末尾应当含 %%EOF marker"
    );
}

// ===================== 错误路径 =====================

#[test]
fn integration_finish_no_pages_yields_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("noop.pdf");
    let w = LopdfWriter::new(&path).unwrap();
    let result = Box::new(w).finish();
    assert!(result.is_err(), "无页时 finish 应当返回错误");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("zero pages"),
        "错误信息应当提示 zero pages: {err_msg}"
    );
}

#[test]
fn integration_ocr_before_page_yields_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ocr_early.pdf");
    let mut w = LopdfWriter::new(&path).unwrap();
    let err = w
        .add_ocr_layer(&[OcrWord::new("oops", 0.0, 0.0, 10.0, 10.0)])
        .unwrap_err();
    assert!(format!("{err}").contains("add_ocr_layer"));
}

#[test]
fn integration_halfsize_unsupported() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("hsize.pdf");
    let mut w = LopdfWriter::new(&path).unwrap();
    let bmp = make_gray_bitmap(10, 10, 0);
    let mut page = OutputPage::from_bitmap(0, bmp, 72.0);
    page.halfsize = 3;
    let err = w.add_page(&page).unwrap_err();
    assert!(format!("{err}").contains("halfsize=3"));
}

// ===================== Boxed trait object =====================

#[test]
fn integration_dyn_pdf_writer_usable() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("dyn.pdf");
    let mut w: Box<dyn PdfWriter> = Box::new(LopdfWriter::new(&path).unwrap());
    let bmp = make_gray_bitmap(50, 50, 100);
    w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
    w.add_outline(OutlineEntry::top_level("X", 0)).unwrap();
    w.add_ocr_layer(&[OcrWord::new("y", 5.0, 25.0, 10.0, 8.0)])
        .unwrap();
    w.finish().unwrap();
    assert!(path.exists());
}

// ===================== fixture round-trip smoke =====================

#[test]
fn integration_synthetic_workflow_smoke() {
    // 模拟"渲染 1 页 → reflow → 输出"的端到端形态
    let dir = tempdir().unwrap();
    let path = dir.path().join("workflow.pdf");
    {
        let mut w = LopdfWriter::new(&path).unwrap();
        // 3 页，含 outline + OCR
        for i in 0..3u32 {
            let bmp = if i == 1 {
                make_rgb_bitmap(200, 280, 220, 220, 220)
            } else {
                make_gray_bitmap(200, 280, 240)
            };
            let mut page = OutputPage::from_bitmap(i, bmp, 150.0);
            page.srcpageno = i as i32;
            w.add_page(&page).unwrap();
            w.add_ocr_layer(&[OcrWord::new(format!("page-{i}"), 30.0, 100.0, 60.0, 12.0)])
                .unwrap();
        }
        // 1 个 top-level + 2 child = 3 outline entries
        w.add_outline(OutlineEntry::top_level("Document", 0))
            .unwrap();
        w.add_outline(OutlineEntry::child("Chapter 1", 0, 0))
            .unwrap();
        w.add_outline(OutlineEntry::child("Chapter 2", 2, 0))
            .unwrap();
        Box::new(w).finish().unwrap();
    }
    // 加载验证
    let doc = LDoc::load(&path).expect("load workflow PDF");
    assert_eq!(doc.get_pages().len(), 3);
    assert!(doc.catalog().is_ok());
    assert!(doc.catalog().unwrap().get(b"Outlines").is_ok());
}
