//! `k2io` - v2.1 新增。统一 mutool 进程调用、临时目录管理、Windows 路径编码（`dunce`）、UTF-8/UNC 处理。
//!
//! 来源 C 文件（部分）：`willuslib/wfile.c`、`willuslib/wsys.c`。
//!
//! 设计依据：
//! - ADR-011（跨平台路径：dunce 规范化 + k2io crate 收口）
//! - ADR-015（mutool stdout PAM 管道为默认渲染路径）
//!
//! 详见 `rust-rewrite-plan.md` v2.1 §5.1（v2.1 修订）/ §5.2。
//! 本 crate 在 M0 阶段仅占位；mutool 进程封装在 Step 4.1（M2）落地。

#![forbid(unsafe_code)]

/// M0 占位：保证 workspace `cargo build` 可通过。
pub fn _placeholder() {}
