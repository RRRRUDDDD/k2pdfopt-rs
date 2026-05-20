//! `k2core` —— 基础位图与几何：`Bitmap` 算法 + `Rect` + `Histogram` + I/O。
//!
//! ## 角色定位
//!
//! - **结构层 [`k2types`]**：定义 `Bitmap` / `PixelFormat` / `BitmapPage` 等纯数据,
//!   不引入算法依赖（叶子 crate）。Step 4.1 决策（Open Question 4.1.A）。
//! - **算法层 `k2core`（本 crate）**：通过 free function（不带 `&self` 的工具函数）
//!   或 `BitmapExt` trait 给 [`k2types::Bitmap`] 加算法。Step 5.x 起承载预处理
//!   (preprocess / deskew / autocrop) 全部实现。
//!
//! ## Step 5.1 交付（结构层）
//!
//! - [`rect::Rect`]：inclusive 语义矩形（对应 C `c1/r1/c2/r2`）
//! - [`histogram::Histogram`]：`Vec<u64>` 直方图 + 横/纵向投影 + 暗像素计数
//! - [`bitmap`]：PNG / PAM / PPM 读写（用 `image` crate 与手写 PAM/PPM 解析）
//!
//! ## Step 5.3 交付（图像预处理）
//!
//! - [`preprocess`]：[`preprocess::apply_contrast`] / [`preprocess::apply_gamma`] /
//!   [`preprocess::apply_sharpen`] 三个滤镜。1:1 复刻 C `willuslib/bmp.c`
//!   `bmp_contrast_adjust` / `bmp_gamma_correct` / `bmp_sharpen` / `bmp_apply_filter`。
//!
//! ## Step 5.4 交付（去歪斜）
//!
//! - [`deskew`]：[`deskew::auto_straighten_angle`] / [`deskew::auto_straighten`] /
//!   [`deskew::rotate_fast`]。1:1 复刻 C `willuslib/bmp.c`
//!   `bmp_autostraighten` / `bmp_row_by_row_stdev`（私有）/ `bmp_rotate_fast`。
//!
//! ## Step 5.5 交付（自动裁剪）
//!
//! - [`autocrop`]：[`autocrop::auto_crop`] / [`autocrop::apply_auto_crop`]
//!   + 数据结构 [`autocrop::AutoCropMargins`] / [`autocrop::AutoCropResult`]。
//!     1:1 复刻 C `k2pdfoptlib/k2bmp.c` `bmp_autocrop2` / `k2bmp_apply_autocrop`
//!     + 私有 `bmp_autocrop2_ex` / `bmp_autocrop_refine` + `xsmooth` (含 C bug)。
//!
//! ## Step 8.4 交付（直角旋转与像素反转）
//!
//! - [`rotate`]：[`rotate::invert`] / [`rotate::rotate_right_angle`]。
//!   1:1 复刻 C `willuslib/bmp.c::bmp_invert` (3223-3244) +
//!   `bmp_rotate_right_angle` (1569-1589) + `bmp_rotate_90` (1592-1624) +
//!   `bmp_rotate_270` (1627-1659)。专为 figure rotate / negative invert 主路径，
//!   不引入 [`deskew::rotate_fast`] 的 bilinear 插值误差。
//!
//! 来源：`rust-rewrite-plan.md` v2.1 §5.2 / §8.2；C 源 `willuslib/bmp.c` (5015 行)
//! 与 `k2pdfoptlib/k2bmp.c` (autocrop 主要在此)。

#![forbid(unsafe_code)]

pub mod autocrop;
pub mod bitmap;
pub mod deskew;
pub mod histogram;
pub mod preprocess;
pub mod rect;
pub mod rotate;

pub use autocrop::{apply_auto_crop, auto_crop, AutoCropMargins, AutoCropResult};
pub use bitmap::{
    read_pam, read_pam_file, read_png, write_pam, write_pam_file, write_png, write_ppm,
    write_ppm_file, BitmapIoError,
};
pub use deskew::{auto_straighten, auto_straighten_angle, rotate_fast};
pub use histogram::{
    horizontal_dark_count, horizontal_projection, vertical_dark_count, vertical_projection,
    Histogram,
};
pub use preprocess::{
    apply_contrast, apply_filter_3x3, apply_gamma, apply_sharpen, build_contrast_lut,
    build_gamma_lut,
};
pub use rect::Rect;
pub use rotate::{invert, rotate_right_angle};

// Re-export of k2types primitives for downstream crates (`k2layout` 等) 不必同时
// 引 k2types 与 k2core。
pub use k2types::{Bitmap, BitmapError, BitmapPage, PixelFormat};
