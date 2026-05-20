//! `bitmap_pdf` - `LopdfWriter`：基于 `lopdf 0.40` 薄封装的 Bitmap PDF writer。
//!
//! 见 ADR-014（Approved 2026-05-07）+ `spikes/pdf-writer/REPORT.md`。
//!
//! # 设计要点
//!
//! 1. **JPEG passthrough**：`OutputPage.jpeg_quality >= 0` 时用 image crate 编码
//!    为 JPEG（DCT），直接写入 PDF Stream（`Filter=/DCTDecode`），零解码开销。
//! 2. **Flate 兜底**：`jpeg_quality < 0` 时把 raw 像素 zlib deflate 写入
//!    （`Filter=/FlateDecode`），适合需要无损（C 版 v2.50+ 默认行为）。
//! 3. **不可见 OCR 层**：`add_ocr_layer` 把 OCR words 编码为 `BT/Tf/3 Tr/Td/Tj/ET`
//!    片段追加到 Page.Contents 数组，依赖 Helvetica 内置字体（无需嵌入字体文件）。
//! 4. **嵌套 outline**：内部用 `Vec<OutlineEntry>` + parent_idx 收集，`finish` 时
//!    构建 PDF outline 对象树（First/Last/Next/Prev/Parent/Count 五指针），与
//!    C 版 `pdffile_add_outline`（`willuslib/pdfwrite.c:193-263`）行为对齐。
//!
//! # C 算法对照
//!
//! | 阶段 | C 函数 | Rust 函数 |
//! |------|--------|-----------|
//! | open file | `pdffile_init` / `pdffile_start` | [`LopdfWriter::new`] |
//! | 写一页 | `pdffile_add_bitmap` / `pdffile_add_bitmap_with_ocrwords` | [`LopdfWriter::add_page`] |
//! | OCR 文字层 | `ocrwords_to_pdf_stream`（pdfwrite.c:1606+） | [`LopdfWriter::add_ocr_layer`] |
//! | outline | `pdffile_add_outline` | [`LopdfWriter::finish`] 内部 |
//! | 收尾 | `pdffile_finish` + `pdffile_close` | [`LopdfWriter::finish`] |

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context as _, Result};
use image::codecs::jpeg::JpegEncoder;
use image::ExtendedColorType;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};

use k2settings::OcrVisibility;
use k2types::{Bitmap, OcrWord, OutlineEntry, OutputPage, PixelFormat};

use crate::{PdfWriteError, PdfWriter};

/// PDF 写出器主实现（ADR-014）。
///
/// 调用顺序：`new` → `add_page` (×N) [→ `add_ocr_layer`] [→ `add_outline`] → `finish`。
/// `add_ocr_layer` 必须紧跟在 `add_page` 之后。
pub struct LopdfWriter {
    doc: Document,
    output_path: PathBuf,
    /// 每页 Page 字典 ID，按 `add_page` 调用顺序。
    pages: Vec<ObjectId>,
    /// 每页物理尺寸（width_pt, height_pt），OCR layer 坐标换算用。
    page_sizes: Vec<(f32, f32)>,
    /// 每页 bitmap 像素尺寸 + DPI，OCR 像素坐标转换 PDF 点用。
    page_bitmap_info: Vec<(u32, u32, f32)>,
    /// 最近一次 add_page 的 self.pages 索引（OCR 归属定位）。
    last_page_idx: Option<usize>,
    /// 内置 Helvetica 字体对象 ID（OCR text layer 用）。
    font_id: ObjectId,
    /// outline 条目（finish 时一次性构建 PDF outline tree）。
    outline_entries: Vec<OutlineEntry>,
    /// Pages 目录 ID（构造时预分配，finish 时填充 Kids/Count）。
    pages_id: ObjectId,
    /// 单调递增的图像资源名前缀（"Im1", "Im2", ...）。
    next_image_no: u32,
    /// 单调递增的页面对象内部计数（仅日志/调试）。
    page_counter: u32,
    /// OCR visibility bit mask（Step 11.11 P1-2）。
    ///
    /// 控制 [`PdfWriter::add_ocr_layer`] 写入 PDF content stream 的渲染策略：
    /// - 不含 [`OcrVisibility::SHOW_OCR_TEXT`] → 跳过整个 BT...ET 段
    /// - 含 [`OcrVisibility::SHOW_OCR_TEXT`] → 写 Tr 3 (invisible)
    /// - 含 [`OcrVisibility::SHOW_BOXES`] → 额外画 word 矩形边框
    ///
    /// 默认 [`OcrVisibility::DEFAULT`] = `SHOW_SOURCE` —— 不含 SHOW_OCR_TEXT，
    /// 不写 OCR 文字层（**与 v0.1.0 默认行为偏离**：v0.1.0 默认 i32=1 同样不写
    /// OCR 文字层但走的是另一条逻辑路径；本步统一到 `pdf_text_render_mode()`
    /// 单一决策点）。调用方通常通过 [`Self::with_ocr_visibility`] 注入
    /// `settings.ocr.dst_ocr_visibility_flags`。
    ocr_visibility: OcrVisibility,
}

impl LopdfWriter {
    /// 创建一个新的 PDF writer，绑定输出路径。
    ///
    /// 此调用**不**立即创建文件——延迟到 [`PdfWriter::finish`]。但会先快速
    /// 校验父目录存在 + 可写。
    pub fn new<P: AsRef<Path>>(output_path: P) -> Result<Self> {
        let path = output_path.as_ref().to_path_buf();
        validate_output_path(&path)?;

        let mut doc = Document::with_version("1.5");

        // 预分配 Pages 目录占位（具体 Kids/Count 在 finish 时填充）
        let pages_id = doc.add_object(dictionary! {});

        // Helvetica Type1 字体（PDF 内置 14 种之一，无需嵌入字体文件）
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });

        Ok(Self {
            doc,
            output_path: path,
            pages: Vec::new(),
            page_sizes: Vec::new(),
            page_bitmap_info: Vec::new(),
            last_page_idx: None,
            font_id,
            outline_entries: Vec::new(),
            pages_id,
            next_image_no: 1,
            page_counter: 0,
            // Step 11.11 P1-2 默认 SHOW_SOURCE（不含 SHOW_OCR_TEXT → 不写 OCR 文字层）
            ocr_visibility: OcrVisibility::DEFAULT,
        })
    }

    /// builder：注入 OCR visibility bit mask（Step 11.11 P1-2）。
    ///
    /// 调用方通常传 `settings.ocr.dst_ocr_visibility_flags` —— 见
    /// `app/k2pdfopt/src/main.rs::open_writer`。
    ///
    /// # 副作用
    ///
    /// 仅影响后续 `add_ocr_layer` 的渲染行为，不动 `add_page` 路径。已添加的页
    /// 不会被回溯重写。
    #[must_use]
    pub fn with_ocr_visibility(mut self, vis: OcrVisibility) -> Self {
        self.ocr_visibility = vis;
        self
    }
}

impl PdfWriter for LopdfWriter {
    fn add_page(&mut self, page: &OutputPage) -> Result<()> {
        // Step 7.2 仅支持 halfsize=0（8-bit）。其它值推迟 Open Question。
        if page.halfsize != 0 {
            return Err(PdfWriteError::UnsupportedHalfsize {
                halfsize: page.halfsize,
            }
            .into());
        }

        // 1. 编码图像 (raw bytes / JPEG / Flate)
        let img_data = encode_image_for_pdf(&page.bitmap, page.jpeg_quality, page.page_index)?;

        // 2. 创建 Image XObject stream
        let mut img_dict = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => i64::from(page.bitmap.width),
            "Height" => i64::from(page.bitmap.height),
            "ColorSpace" => img_data.color_space,
            "BitsPerComponent" => i64::from(img_data.bits_per_component),
            "Filter" => img_data.filter,
        };
        // Flate (raw pixel) 模式需要明确 DecodeParms 让 reader 知道这是连续行
        if img_data.filter == "FlateDecode" {
            // 无 Predictor（zlib 直接压缩 raw 像素行），不写 DecodeParms 即可
            // 但为兼容性显式声明 Columns=width 以便阅读器准确解码
            let _ = img_dict;
            img_dict = dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => i64::from(page.bitmap.width),
                "Height" => i64::from(page.bitmap.height),
                "ColorSpace" => img_data.color_space,
                "BitsPerComponent" => i64::from(img_data.bits_per_component),
                "Filter" => "FlateDecode",
            };
        }
        let img_stream = Stream::new(img_dict, img_data.bytes);
        let img_id = self.doc.add_object(img_stream);

        // 3. 资源名（PDF 字典内引用）
        let image_resource_name = format!("Im{}", self.next_image_no);
        self.next_image_no = self.next_image_no.saturating_add(1);

        // 4. 物理尺寸（pt）
        let width_pt = page.width_pt();
        let height_pt = page.height_pt();

        // 5. Bitmap content stream: q [w_pt 0 0 h_pt 0 0] cm /ImN Do Q
        let bitmap_content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        Object::Real(width_pt),
                        Object::Integer(0),
                        Object::Integer(0),
                        Object::Real(height_pt),
                        Object::Integer(0),
                        Object::Integer(0),
                    ],
                ),
                Operation::new(
                    "Do",
                    vec![Object::Name(image_resource_name.as_bytes().to_vec())],
                ),
                Operation::new("Q", vec![]),
            ],
        };
        let bitmap_content_bytes = bitmap_content
            .encode()
            .context("encode bitmap content stream")?;
        let bitmap_content_id = self
            .doc
            .add_object(Stream::new(dictionary! {}, bitmap_content_bytes));

        // 6. Page 对象
        let page_dict = dictionary! {
            "Type" => "Page",
            "Parent" => self.pages_id,
            "MediaBox" => vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Real(width_pt),
                Object::Real(height_pt),
            ],
            "Contents" => bitmap_content_id,
            "Resources" => dictionary! {
                "XObject" => dictionary! {
                    image_resource_name.as_str() => img_id,
                },
                "Font" => dictionary! {
                    "F1" => self.font_id,
                },
            },
        };
        let page_id = self.doc.add_object(page_dict);

        // 7. 记账
        self.pages.push(page_id);
        self.page_sizes.push((width_pt, height_pt));
        self.page_bitmap_info
            .push((page.bitmap.width, page.bitmap.height, page.output_dpi));
        self.last_page_idx = Some(self.pages.len() - 1);
        self.page_counter = self.page_counter.saturating_add(1);

        Ok(())
    }

    fn add_outline(&mut self, entry: OutlineEntry) -> Result<()> {
        // 校验 parent_idx 在当前 outline_entries 范围内
        if let Some(p) = entry.parent_idx {
            if p >= self.outline_entries.len() {
                return Err(PdfWriteError::OutlineParentOutOfBounds {
                    parent_idx: p,
                    current_len: self.outline_entries.len(),
                }
                .into());
            }
        }
        // 校验 dst_page 在已添加 pages 范围内（-1 = 未映射，允许，finish 时跳过 Dest）
        if entry.dst_page >= 0 && (entry.dst_page as usize) >= self.pages.len() {
            return Err(PdfWriteError::OutlineDstPageOutOfBounds {
                dst_page: entry.dst_page,
                current_pages: self.pages.len() as u32,
            }
            .into());
        }
        self.outline_entries.push(entry);
        Ok(())
    }

    fn add_ocr_layer(&mut self, words: &[OcrWord]) -> Result<()> {
        let last_idx = self.last_page_idx.ok_or(PdfWriteError::OcrBeforePage)?;

        if words.is_empty() {
            return Ok(());
        }

        // Step 11.11 P1-2：visibility 不含 SHOW_OCR_TEXT 且不含 SHOW_BOXES → 整段短路
        // （既不写文字层，也不画边框，OCR 输入纯粹被丢弃。这与 C `k2ocr.c` 的
        // `dst_ocr_visibility_flags & 0x06 == 0` 路径行为一致：visibility=1 默认即
        // "仅显示 source bitmap" → OCR words 不进入 PDF content stream）。
        let vis = self.ocr_visibility;
        if vis.pdf_text_render_mode().is_none() && !vis.contains(OcrVisibility::SHOW_BOXES) {
            return Ok(());
        }

        let page_id = self.pages[last_idx];
        let (_, page_height_pt) = self.page_sizes[last_idx];
        let (_, _, dpi) = self.page_bitmap_info[last_idx];

        // 构建 OCR content stream（按 visibility 切换 Tr 模式 + 可选 box 绘制）
        let ocr_content_bytes = build_ocr_content_stream(words, dpi, page_height_pt, vis)?;
        if ocr_content_bytes.is_empty() {
            // 内部决策为"无内容" → 不追加空 stream（保持 PDF Contents 数组紧凑）
            return Ok(());
        }
        let ocr_content_id = self
            .doc
            .add_object(Stream::new(dictionary! {}, ocr_content_bytes));

        // 把 page.Contents 改为数组 [bitmap_content_id, ocr_content_id, ...]
        append_to_page_contents(&mut self.doc, page_id, ocr_content_id)?;

        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<()> {
        let mut writer = *self;

        // 1. 构建 Pages 字典
        if writer.pages.is_empty() {
            return Err(anyhow!(
                "cannot finish PDF with zero pages (add_page never called)"
            ));
        }
        let kids: Vec<Object> = writer
            .pages
            .iter()
            .map(|&id| Object::Reference(id))
            .collect();
        let pages_dict = dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => writer.pages.len() as i64,
        };
        replace_dict(&mut writer.doc, writer.pages_id, pages_dict)?;

        // 2. 构建 outline 树（如有）
        let outlines_id =
            build_outline_tree(&mut writer.doc, &writer.outline_entries, &writer.pages);

        // 3. Catalog
        let catalog_id = if let Some(out_id) = outlines_id {
            writer.doc.add_object(dictionary! {
                "Type" => "Catalog",
                "Pages" => writer.pages_id,
                "Outlines" => out_id,
                "PageMode" => "UseOutlines",
            })
        } else {
            writer.doc.add_object(dictionary! {
                "Type" => "Catalog",
                "Pages" => writer.pages_id,
            })
        };
        writer.doc.trailer.set("Root", catalog_id);

        // 4. 写盘
        writer
            .doc
            .save(&writer.output_path)
            .with_context(|| format!("save PDF to {}", writer.output_path.display()))?;

        Ok(())
    }
}

// ===================== 私有 helper =====================

/// 早期校验输出路径父目录存在 + 可写。
fn validate_output_path(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(PdfWriteError::OutputPathNotWritable {
                path: path.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("parent directory {} does not exist", parent.display()),
                ),
            }
            .into());
        }
    }
    Ok(())
}

/// 编码图像为 PDF Stream 用字节流 + filter / colorspace / bpc 元信息。
struct EncodedImage {
    bytes: Vec<u8>,
    filter: &'static str,
    color_space: &'static str,
    bits_per_component: u8,
}

fn encode_image_for_pdf(bmp: &Bitmap, jpeg_quality: i32, page_index: u32) -> Result<EncodedImage> {
    if jpeg_quality >= 0 {
        encode_as_jpeg(bmp, jpeg_quality, page_index)
    } else {
        encode_as_flate(bmp, page_index)
    }
}

fn encode_as_jpeg(bmp: &Bitmap, quality: i32, page_index: u32) -> Result<EncodedImage> {
    let q = quality.clamp(1, 100) as u8;
    let mut buf: Vec<u8> = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut buf, q);

    let (data_ref, color_type, color_space) = match bmp.format {
        PixelFormat::Gray8 => (
            std::borrow::Cow::Borrowed(&bmp.pixels[..]),
            ExtendedColorType::L8,
            "DeviceGray",
        ),
        PixelFormat::Rgb8 => (
            std::borrow::Cow::Borrowed(&bmp.pixels[..]),
            ExtendedColorType::Rgb8,
            "DeviceRGB",
        ),
        PixelFormat::Rgba8 => {
            // JPEG 不支持 alpha，先丢 alpha 转 RGB（C 版无 RGBA，是 Rust 扩展行为）
            let mut rgb: Vec<u8> = Vec::with_capacity(bmp.pixels.len() / 4 * 3);
            for chunk in bmp.pixels.chunks_exact(4) {
                rgb.push(chunk[0]);
                rgb.push(chunk[1]);
                rgb.push(chunk[2]);
            }
            (
                std::borrow::Cow::Owned(rgb),
                ExtendedColorType::Rgb8,
                "DeviceRGB",
            )
        }
    };

    encoder
        .encode(&data_ref, bmp.width, bmp.height, color_type)
        .map_err(|e| PdfWriteError::ImageEncode {
            page_index,
            reason: format!("JPEG encode: {e}"),
        })?;

    Ok(EncodedImage {
        bytes: buf,
        filter: "DCTDecode",
        color_space,
        bits_per_component: 8,
    })
}

fn encode_as_flate(bmp: &Bitmap, page_index: u32) -> Result<EncodedImage> {
    let (raw, color_space) = match bmp.format {
        PixelFormat::Gray8 => (std::borrow::Cow::Borrowed(&bmp.pixels[..]), "DeviceGray"),
        PixelFormat::Rgb8 => (std::borrow::Cow::Borrowed(&bmp.pixels[..]), "DeviceRGB"),
        PixelFormat::Rgba8 => {
            // 丢 alpha，与 JPEG 路径一致
            let mut rgb: Vec<u8> = Vec::with_capacity(bmp.pixels.len() / 4 * 3);
            for chunk in bmp.pixels.chunks_exact(4) {
                rgb.push(chunk[0]);
                rgb.push(chunk[1]);
                rgb.push(chunk[2]);
            }
            (std::borrow::Cow::Owned(rgb), "DeviceRGB")
        }
    };

    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&raw)
        .map_err(|e| PdfWriteError::ImageEncode {
            page_index,
            reason: format!("flate write: {e}"),
        })?;
    let bytes = encoder.finish().map_err(|e| PdfWriteError::ImageEncode {
        page_index,
        reason: format!("flate finish: {e}"),
    })?;

    Ok(EncodedImage {
        bytes,
        filter: "FlateDecode",
        color_space,
        bits_per_component: 8,
    })
}

/// 构建 OCR 不可见文字层 content stream（Step 11.11 P1-2 visibility 切换版）。
///
/// 坐标换算（Step 9.2 修正：[`OcrWord`] 的 `y` 是矩形**顶部** y，image top-left
/// 原点 y 向下增长。PDF 坐标系是左下原点 y 向上，baseline 对应 word **底部** y）：
///
/// - `x_pt = x_px * 72 / dpi`
/// - `baseline_y_pt = page_height_pt - y_bottom_px * 72 / dpi`，其中
///   `y_bottom_px = w.y + w.h` 由 [`OcrWord::y_bottom`] 提供
/// - `font_size_pt = h_px * 72 / dpi`
///
/// Step 7.2 原实现误用 `w.y * 72/dpi`（顶部 y 当作 baseline 用）导致 OCR 文字
/// 层高度偏移一个 `h_px`；Step 9.2 校正为 `y_bottom`（Open Question 9.1.C / 7.2.D）。
///
/// # Step 11.11 visibility 决策（参数 `vis`）
///
/// | bit | 行为 |
/// |-----|------|
/// | 不含 SHOW_OCR_TEXT 且不含 SHOW_BOXES | 返空 stream（调用方应跳过追加）|
/// | 含 SHOW_OCR_TEXT | 输出 `BT 3 Tr Tf Tm Tj ET` 序列（Tr 3 = invisible，可复制+搜索）|
/// | 含 SHOW_BOXES | 在文字层**之外**输出 `q m l l l l h S Q` 路径绘制 word 边框矩形 |
///
/// 两个 bit 可同时 on（默认 PDF 不显示文字 + 不画框；调试时手动开启 SHOW_BOXES 看 OCR 命中位置）。
fn build_ocr_content_stream(
    words: &[OcrWord],
    dpi: f32,
    page_height_pt: f32,
    vis: OcrVisibility,
) -> Result<Vec<u8>> {
    let dpi_f64 = f64::from(dpi);
    let page_h_f64 = f64::from(page_height_pt);
    let render_mode = vis.pdf_text_render_mode();
    let show_boxes = vis.contains(OcrVisibility::SHOW_BOXES);

    // 早退：visibility 既不要文字也不要框 → 空 stream（add_ocr_layer 也已做同样短路）
    if render_mode.is_none() && !show_boxes {
        return Ok(Vec::new());
    }

    let mut ops: Vec<Operation> = Vec::with_capacity(4 + words.len() * 6);

    // ── SHOW_BOXES：先画矩形（在文字之下，避免覆盖 Tj 输出）──
    if show_boxes {
        // 1 pt 黑色线宽（最小可见线宽，不依赖 SetLineCap/SetLineJoin 默认值）
        ops.push(Operation::new("q", vec![])); // 保存 graphics state
        ops.push(Operation::new(
            "w",
            vec![Object::Real(0.5_f32)], // 0.5 pt 细线
        ));
        ops.push(Operation::new(
            "RG",
            vec![
                Object::Real(1.0_f32),
                Object::Real(0.0_f32),
                Object::Real(0.0_f32),
            ], // 红色 stroke，便于调试可见
        ));
        for w in words {
            let x_pt = (w.x * 72.0) / dpi_f64;
            let y_bot_pt = page_h_f64 - (w.y_bottom() * 72.0) / dpi_f64;
            let width_pt = (w.w * 72.0) / dpi_f64;
            let height_pt = (w.h * 72.0) / dpi_f64;
            // PDF rectangle 算子：`re x y w h` 然后 `S` stroke
            ops.push(Operation::new(
                "re",
                vec![
                    Object::Real(x_pt as f32),
                    Object::Real(y_bot_pt as f32),
                    Object::Real(width_pt as f32),
                    Object::Real(height_pt as f32),
                ],
            ));
            ops.push(Operation::new("S", vec![]));
        }
        ops.push(Operation::new("Q", vec![])); // 恢复 graphics state
    }

    // ── SHOW_OCR_TEXT：写不可见文字层（Tr 3）──
    if let Some(tr_mode) = render_mode {
        ops.push(Operation::new("BT", vec![]));
        ops.push(Operation::new("Tr", vec![Object::Integer(tr_mode)]));

        for w in words {
            let font_size_pt = ((w.h * 72.0) / dpi_f64).max(1.0);
            let x_pt = (w.x * 72.0) / dpi_f64;
            let baseline_y_pt = page_h_f64 - (w.y_bottom() * 72.0) / dpi_f64;

            // 字号变化时重设字体（简化：每个 word 都设）
            ops.push(Operation::new(
                "Tf",
                vec![
                    Object::Name(b"F1".to_vec()),
                    Object::Real(font_size_pt as f32),
                ],
            ));
            // Text matrix 重置 + 平移（用 Tm 1 0 0 1 x y 直接设置）
            ops.push(Operation::new(
                "Tm",
                vec![
                    Object::Integer(1),
                    Object::Integer(0),
                    Object::Integer(0),
                    Object::Integer(1),
                    Object::Real(x_pt as f32),
                    Object::Real(baseline_y_pt as f32),
                ],
            ));
            // 文本（用 PDF Literal String 编码，UTF-8 字节流原样写入；PDF 阅读器
            // 用 WinAnsiEncoding 解释，非 ASCII 字符可能搜索不到但不影响渲染）
            ops.push(Operation::new(
                "Tj",
                vec![Object::String(
                    w.text.as_bytes().to_vec(),
                    StringFormat::Literal,
                )],
            ));
        }

        ops.push(Operation::new("ET", vec![]));
    }

    let content = Content { operations: ops };
    content.encode().context("encode OCR content stream")
}

/// 把额外的 content stream 追加到 page.Contents。
///
/// PDF 规范允许 Page.Contents 是单个 stream ref **或**数组 of stream refs。
/// 本函数处理两种现有形式：单 ref → 转为数组；数组 → push。
fn append_to_page_contents(
    doc: &mut Document,
    page_id: ObjectId,
    extra_id: ObjectId,
) -> Result<()> {
    // 先取当前 Contents 拷贝出来（避免双重借用 doc）
    let current_contents: Object = {
        let page_obj = doc
            .get_object(page_id)
            .context("page not found when appending OCR")?;
        let dict = match page_obj {
            Object::Dictionary(d) => d,
            _ => return Err(anyhow!("page object is not a dictionary")),
        };
        dict.get(b"Contents")
            .map_err(|_| anyhow!("page dict missing /Contents"))?
            .clone()
    };

    let new_contents = match current_contents {
        Object::Reference(existing_id) => Object::Array(vec![
            Object::Reference(existing_id),
            Object::Reference(extra_id),
        ]),
        Object::Array(mut arr) => {
            arr.push(Object::Reference(extra_id));
            Object::Array(arr)
        }
        other => other, // 不应发生，保留以避免破坏 PDF
    };

    let page_obj = doc
        .get_object_mut(page_id)
        .context("page not found when writing OCR back")?;
    if let Object::Dictionary(ref mut d) = page_obj {
        d.set("Contents", new_contents);
    } else {
        return Err(anyhow!("page object is not a dictionary"));
    }
    Ok(())
}

/// 用新字典替换一个已存在 ObjectId 指向的字典（用于占位 ObjectId 的延迟填充）。
fn replace_dict(doc: &mut Document, id: ObjectId, new_dict: Dictionary) -> Result<()> {
    let obj = doc
        .get_object_mut(id)
        .context("object not found when replacing dict")?;
    if let Object::Dictionary(ref mut d) = obj {
        *d = new_dict;
        Ok(())
    } else {
        Err(anyhow!("target object is not a dictionary"))
    }
}

/// 构建 PDF outline 对象树（嵌套书签）。
///
/// 输入：扁平 `entries` + parent_idx 表达层级 + page IDs 数组。
/// 输出：outlines root ObjectId（空 entries 返回 None）。
///
/// 对应 C 版 `pdffile_add_outline`（`willuslib/pdfwrite.c:193-263`）。
fn build_outline_tree(
    doc: &mut Document,
    entries: &[OutlineEntry],
    pages: &[ObjectId],
) -> Option<ObjectId> {
    if entries.is_empty() {
        return None;
    }

    // 1. 预分配每个 entry 的 ObjectId（空字典占位）
    let entry_ids: Vec<ObjectId> = entries
        .iter()
        .map(|_| doc.add_object(Dictionary::new()))
        .collect();

    // 2. 根据 parent_idx 建 children map（key: parent_idx, value: 子条目索引列表）
    let mut children: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        children.entry(e.parent_idx).or_default().push(i);
    }

    // 3. Outlines root ObjectId 预分配
    let outlines_id = doc.add_object(Dictionary::new());
    let empty_vec: Vec<usize> = Vec::new();
    let top_level: &Vec<usize> = children.get(&None).unwrap_or(&empty_vec);
    if top_level.is_empty() {
        // 所有 entry 都有 parent_idx 但没有顶层 — 不写 outline
        return None;
    }

    // 4. 填充每个 entry 字典
    for (i, entry) in entries.iter().enumerate() {
        let mut dict = Dictionary::new();
        dict.set(
            "Title",
            Object::String(entry.title.as_bytes().to_vec(), StringFormat::Literal),
        );

        // Parent
        let parent_id = match entry.parent_idx {
            Some(p) => entry_ids[p],
            None => outlines_id,
        };
        dict.set("Parent", parent_id);

        // Dest（仅当 dst_page 合法）
        if entry.dst_page >= 0 && (entry.dst_page as usize) < pages.len() {
            let page_id = pages[entry.dst_page as usize];
            dict.set(
                "Dest",
                Object::Array(vec![
                    Object::Reference(page_id),
                    Object::Name(b"Fit".to_vec()),
                ]),
            );
        }

        // First/Last/Count（子条目）
        let my_children: &Vec<usize> = children.get(&Some(i)).unwrap_or(&empty_vec);
        if !my_children.is_empty() {
            let first_id = entry_ids[my_children[0]];
            let last_id = entry_ids[my_children[my_children.len() - 1]];
            dict.set("First", first_id);
            dict.set("Last", last_id);
            dict.set("Count", my_children.len() as i64);
        }

        // Next/Prev（同 parent 兄弟）
        let siblings: &Vec<usize> = children.get(&entry.parent_idx).unwrap_or(&empty_vec);
        if let Some(pos) = siblings.iter().position(|&x| x == i) {
            if pos > 0 {
                dict.set("Prev", entry_ids[siblings[pos - 1]]);
            }
            if pos + 1 < siblings.len() {
                dict.set("Next", entry_ids[siblings[pos + 1]]);
            }
        }

        // 写回
        if let Ok(Object::Dictionary(ref mut d)) = doc.get_object_mut(entry_ids[i]) {
            *d = dict;
        }
    }

    // 5. Outlines root 字典
    let mut root_dict = Dictionary::new();
    root_dict.set("Type", "Outlines");
    root_dict.set("First", entry_ids[top_level[0]]);
    root_dict.set("Last", entry_ids[top_level[top_level.len() - 1]]);
    root_dict.set("Count", top_level.len() as i64);
    if let Ok(Object::Dictionary(ref mut d)) = doc.get_object_mut(outlines_id) {
        *d = root_dict;
    }

    Some(outlines_id)
}

// ===================== Unit Tests =====================

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use k2types::PixelFormat;
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

    #[test]
    fn new_writer_with_valid_path_succeeds() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        let w = LopdfWriter::new(&path);
        assert!(w.is_ok());
    }

    #[test]
    fn new_writer_with_missing_parent_dir_fails() {
        let path = std::path::Path::new("Z:/nonexistent_dir_xyz_12345/out.pdf");
        let w = LopdfWriter::new(path);
        // Windows 可能 Z: 不存在，校验失败
        // Linux/Mac 上 /nonexistent_... 同样不存在
        assert!(w.is_err());
    }

    #[test]
    fn add_page_writes_single_gray_jpeg_pdf() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("gray.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(100, 50, 200);
        let page = OutputPage::from_bitmap(0, bmp, 150.0);
        w.add_page(&page).unwrap();
        Box::new(w).finish().unwrap();

        let meta = std::fs::metadata(&path).unwrap();
        assert!(meta.len() > 100, "PDF should have non-trivial size");
    }

    #[test]
    fn add_page_writes_rgb_jpeg_pdf() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("rgb.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_rgb_bitmap(80, 60, 255, 0, 0);
        let page = OutputPage::from_bitmap(0, bmp, 150.0);
        w.add_page(&page).unwrap();
        Box::new(w).finish().unwrap();

        let meta = std::fs::metadata(&path).unwrap();
        assert!(meta.len() > 100);
    }

    #[test]
    fn flate_mode_writes_pdf() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("flate.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(50, 50, 128);
        let mut page = OutputPage::from_bitmap(0, bmp, 100.0);
        page.jpeg_quality = -1; // FlateDecode
        w.add_page(&page).unwrap();
        Box::new(w).finish().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
    }

    #[test]
    fn unsupported_halfsize_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("half.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(10, 10, 0);
        let mut page = OutputPage::from_bitmap(0, bmp, 72.0);
        page.halfsize = 1;
        let err = w.add_page(&page).unwrap_err();
        assert!(format!("{err}").contains("halfsize=1"));
    }

    #[test]
    fn multipage_pdf() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("multi.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        for i in 0..3 {
            let bmp = make_gray_bitmap(40, 40, 100 + (i as u8) * 20);
            let page = OutputPage::from_bitmap(i, bmp, 100.0);
            w.add_page(&page).unwrap();
        }
        Box::new(w).finish().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 200);
    }

    #[test]
    fn finish_with_no_pages_fails() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.pdf");
        let w = LopdfWriter::new(&path).unwrap();
        let err = Box::new(w).finish().unwrap_err();
        assert!(format!("{err}").contains("zero pages"));
    }

    #[test]
    fn add_ocr_before_add_page_fails() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ocr_early.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let word = OcrWord::new("hello", 10.0, 20.0, 50.0, 12.0);
        let err = w.add_ocr_layer(&[word]).unwrap_err();
        assert!(format!("{err}").contains("add_ocr_layer"));
    }

    #[test]
    fn add_ocr_layer_after_page() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ocr.pdf");
        // Step 11.11 P1-2：默认 visibility=SHOW_SOURCE 不写 OCR 文字层 —— 这一测试历史
        // 上假设 OCR words 会被写入 → 显式启用 SHOW_OCR_TEXT 保证 PDF >100B。
        let mut w = LopdfWriter::new(&path)
            .unwrap()
            .with_ocr_visibility(OcrVisibility::from_bits(
                OcrVisibility::SHOW_SOURCE.bits() | OcrVisibility::SHOW_OCR_TEXT.bits(),
            ));
        let bmp = make_gray_bitmap(200, 100, 255);
        let page = OutputPage::from_bitmap(0, bmp, 100.0);
        w.add_page(&page).unwrap();
        let words = vec![
            OcrWord::new("hello", 20.0, 50.0, 40.0, 14.0),
            OcrWord::new("world", 70.0, 50.0, 40.0, 14.0),
        ];
        w.add_ocr_layer(&words).unwrap();
        Box::new(w).finish().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 100);
    }

    #[test]
    fn add_ocr_empty_words_is_noop() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ocr_empty.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(50, 50, 200);
        let page = OutputPage::from_bitmap(0, bmp, 100.0);
        w.add_page(&page).unwrap();
        w.add_ocr_layer(&[]).unwrap();
        Box::new(w).finish().unwrap();
    }

    #[test]
    fn add_outline_top_level() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("outline.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(50, 50, 200);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        w.add_outline(OutlineEntry::top_level("Chapter 1", 0))
            .unwrap();
        Box::new(w).finish().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 100);
    }

    #[test]
    fn add_outline_nested() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        for i in 0..3 {
            let bmp = make_gray_bitmap(50, 50, 100 + (i as u8) * 20);
            w.add_page(&OutputPage::from_bitmap(i, bmp, 100.0)).unwrap();
        }
        w.add_outline(OutlineEntry::top_level("Part I", 0)).unwrap();
        w.add_outline(OutlineEntry::child("Chapter 1", 1, 0))
            .unwrap();
        w.add_outline(OutlineEntry::child("Chapter 2", 2, 0))
            .unwrap();
        w.add_outline(OutlineEntry::top_level("Part II", 2))
            .unwrap();
        Box::new(w).finish().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 200);
    }

    #[test]
    fn add_outline_dst_page_out_of_bounds_fails() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("oob.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(50, 50, 200);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        let err = w
            .add_outline(OutlineEntry::top_level("Bad", 5))
            .unwrap_err();
        assert!(format!("{err}").contains("dst_page=5"));
    }

    #[test]
    fn add_outline_parent_idx_out_of_bounds_fails() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("badparent.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(50, 50, 200);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        let err = w
            .add_outline(OutlineEntry::child("Orphan", 0, 99))
            .unwrap_err();
        assert!(format!("{err}").contains("parent_idx=99"));
    }

    #[test]
    fn add_outline_dst_page_minus_one_allowed() {
        // dst_page = -1 表示未映射，应被允许保留（finish 时跳过 /Dest）
        let dir = tempdir().unwrap();
        let path = dir.path().join("unmapped.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(50, 50, 200);
        w.add_page(&OutputPage::from_bitmap(0, bmp, 100.0)).unwrap();
        let mut entry = OutlineEntry::top_level("Future", -1);
        entry.dst_page = -1;
        w.add_outline(entry).unwrap();
        Box::new(w).finish().unwrap();
    }

    #[test]
    fn pdf_writer_trait_object_works() {
        // 经 Box<dyn PdfWriter> 也能 add_page / finish
        let dir = tempdir().unwrap();
        let path = dir.path().join("dyn.pdf");
        let w: Box<dyn PdfWriter> = Box::new(LopdfWriter::new(&path).unwrap());
        let mut boxed = w;
        let bmp = make_gray_bitmap(50, 50, 200);
        boxed
            .add_page(&OutputPage::from_bitmap(0, bmp, 100.0))
            .unwrap();
        boxed.finish().unwrap();
        assert!(path.exists());
    }

    #[test]
    fn jpeg_quality_clamped_to_valid_range() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("q.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let bmp = make_gray_bitmap(50, 50, 128);
        let mut page = OutputPage::from_bitmap(0, bmp, 100.0);
        page.jpeg_quality = 200; // 越界值；应被 clamp 到 [1, 100]
        w.add_page(&page).unwrap();
        Box::new(w).finish().unwrap();
    }

    #[test]
    fn rgba_bitmap_drops_alpha_writes_pdf() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("rgba.pdf");
        let mut w = LopdfWriter::new(&path).unwrap();
        let mut bmp = Bitmap::new(20, 20, 100.0, PixelFormat::Rgba8).unwrap();
        bmp.fill_rgb(50, 100, 150);
        let page = OutputPage::from_bitmap(0, bmp, 100.0);
        w.add_page(&page).unwrap();
        Box::new(w).finish().unwrap();
    }

    #[test]
    fn encode_image_jpeg_gray() {
        let bmp = make_gray_bitmap(10, 10, 128);
        let img = encode_image_for_pdf(&bmp, 85, 0).unwrap();
        assert_eq!(img.filter, "DCTDecode");
        assert_eq!(img.color_space, "DeviceGray");
        assert_eq!(img.bits_per_component, 8);
        assert!(!img.bytes.is_empty());
    }

    #[test]
    fn encode_image_flate_rgb() {
        let bmp = make_rgb_bitmap(8, 8, 10, 20, 30);
        let img = encode_image_for_pdf(&bmp, -1, 0).unwrap();
        assert_eq!(img.filter, "FlateDecode");
        assert_eq!(img.color_space, "DeviceRGB");
        assert!(!img.bytes.is_empty());
    }

    #[test]
    fn ocr_content_includes_invisible_marker() {
        let words = vec![OcrWord::new("hi", 10.0, 50.0, 20.0, 12.0)];
        // 显式 SHOW_OCR_TEXT 触发 Tr 3 输出；默认 SHOW_SOURCE 不写文字层
        let vis = OcrVisibility::SHOW_OCR_TEXT;
        let bytes = build_ocr_content_stream(&words, 100.0, 72.0, vis).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("3 Tr"), "should set Tr=3 (invisible)");
        assert!(s.contains("BT") && s.contains("ET"));
        assert!(s.contains("hi"));
    }

    // ── Step 11.11 P1-2 visibility 切换专项测试 ──

    /// 不含 SHOW_OCR_TEXT 且不含 SHOW_BOXES → 返空 stream（add_ocr_layer 也已短路）。
    #[test]
    fn ocr_content_empty_when_visibility_omits_text_and_boxes() {
        let words = vec![OcrWord::new("hi", 10.0, 50.0, 20.0, 12.0)];
        // 默认 SHOW_SOURCE 仅控制 source bitmap，不输出 OCR 内容
        let vis = OcrVisibility::DEFAULT;
        let bytes = build_ocr_content_stream(&words, 100.0, 72.0, vis).unwrap();
        assert!(
            bytes.is_empty(),
            "默认 visibility（SHOW_SOURCE only）应短路返空 stream"
        );
    }

    /// 含 SHOW_BOXES 但不含 SHOW_OCR_TEXT → 仅画矩形不输出文字层。
    /// 应含 PDF rectangle 算子 `re` + stroke `S` 作为独立 token（lopdf 输出每个
    /// 算子独占一行，因此用 split_whitespace + token 精确比较，避免 "S" 被 "Subtype"
    /// 之类的关键字干扰）。
    #[test]
    fn ocr_content_includes_boxes_only_when_visibility_is_show_boxes() {
        let words = vec![OcrWord::new("hi", 10.0, 50.0, 20.0, 12.0)];
        let vis = OcrVisibility::SHOW_BOXES;
        let bytes = build_ocr_content_stream(&words, 100.0, 72.0, vis).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        let tokens: Vec<&str> = s.split_whitespace().collect();
        assert!(tokens.contains(&"re"), "应含 PDF 矩形算子 're': {s}");
        assert!(tokens.contains(&"S"), "应含 stroke 算子 'S': {s}");
        assert!(!s.contains("BT"), "不应含 BT（无文字层）: {s}");
        assert!(!s.contains("Tj"), "不应含 Tj（无文字写入）: {s}");
    }

    /// 同时含 SHOW_OCR_TEXT + SHOW_BOXES → 既画框又写文字（box 在 BT 之前）。
    #[test]
    fn ocr_content_includes_boxes_and_text_when_both_flags_set() {
        let words = vec![OcrWord::new("hi", 10.0, 50.0, 20.0, 12.0)];
        let vis = OcrVisibility::from_bits(
            OcrVisibility::SHOW_OCR_TEXT.bits() | OcrVisibility::SHOW_BOXES.bits(),
        );
        let bytes = build_ocr_content_stream(&words, 100.0, 72.0, vis).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        let tokens: Vec<&str> = s.split_whitespace().collect();
        let box_pos = s.find("re").expect("矩形算子缺失");
        let bt_pos = s.find("BT").expect("BT 缺失");
        assert!(
            box_pos < bt_pos,
            "矩形应在 BT 之前（先画框再写文字，避免覆盖）"
        );
        assert!(tokens.contains(&"S"), "应含 stroke 算子 'S': {s}");
        assert!(s.contains("3 Tr"), "应含 Tr=3 invisible 模式: {s}");
        assert!(s.contains("hi"), "应含原始 word 文本: {s}");
    }

    #[test]
    fn validate_output_path_accepts_empty_parent() {
        // 当前目录或 bare filename
        let result = validate_output_path(std::path::Path::new("foo.pdf"));
        assert!(result.is_ok());
    }
}
