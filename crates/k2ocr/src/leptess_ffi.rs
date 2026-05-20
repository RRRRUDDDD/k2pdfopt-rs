//! `k2ocr::leptess_ffi` —— ADR-017 备选实现占位。
//!
//! 此模块仅在启用 Cargo feature `leptess` 时编译。**当前为 stub**：
//! [`LeptessEngine::new`] 一律返回 [`OcrError::FeatureDisabled`]，
//! 真正引入 `leptess = 0.14` crate 与 vcpkg/leptonica 链接配置
//! 推迟到 M7 末 vs CLI benchmark 阶段（详见 ADR-017 撤回触发条件）。
//!
//! # 为什么先放占位而不直接删
//!
//! - `OcrEngine` trait 设计完整暴露 `leptess` 路径的能力（probe / list_langs / recognize），
//!   后续真实实现可以平滑替换而不破坏调用方
//! - feature gate 在 `Cargo.toml` 已写入，CI 矩阵跑 `--features leptess` 时编译保证不挂
//! - 撤回条件触发（leptess 活跃维护回归 / CLI 子进程开销在新场景下无法接受）时，
//!   只需在本文件实装，调用方零改动
//!
//! # 撤回 stub 时要做的事
//!
//! 1. `Cargo.toml` 把 feature 表达式从 `leptess = []` 改为
//!    `leptess = ["dep:leptess"]`，并在 `[dependencies]` 加 `leptess = { version = "0.14", optional = true }`
//! 2. 在本文件实装 `LeptessEngine` 的真实 FFI 调用（参考 `willuslib/ocrtess.c::ocrtess_ocrwords_from_bmp8` 流程）
//! 3. 在 `tests/leptess_smoke.rs` 加 cfg-gated 集成测试，跑同一份 fixture 校验与 CLI 输出一致性
//! 4. 更新 ADR-017 状态：`Approved → Superseded by ADR-018`（如有）

use crate::types::{OcrEngineInfo, OcrError, OcrPageInput};
use crate::OcrEngine;
use k2types::OcrWord;

/// leptess FFI 占位。真实实现推迟到 M7 末。
///
/// 所有方法返回 [`OcrError::FeatureDisabled`]；构造也走同一错误。
pub struct LeptessEngine {
    _private: (),
}

impl LeptessEngine {
    /// 永远返回 [`OcrError::FeatureDisabled`]（feature stub）。
    pub fn new() -> Result<Self, OcrError> {
        Err(OcrError::FeatureDisabled(
            "leptess FFI 推迟到 M7 末 (ADR-017 撤回触发条件)",
        ))
    }
}

impl OcrEngine for LeptessEngine {
    fn engine_name(&self) -> &'static str {
        "leptess-ffi"
    }

    fn probe(&self) -> Result<OcrEngineInfo, OcrError> {
        Err(OcrError::FeatureDisabled(
            "leptess FFI 推迟到 M7 末 (ADR-017)",
        ))
    }

    fn list_langs(&self) -> Result<Vec<String>, OcrError> {
        Err(OcrError::FeatureDisabled(
            "leptess FFI 推迟到 M7 末 (ADR-017)",
        ))
    }

    fn recognize(&self, _input: &OcrPageInput<'_>) -> Result<Vec<OcrWord>, OcrError> {
        Err(OcrError::FeatureDisabled(
            "leptess FFI 推迟到 M7 末 (ADR-017)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_returns_feature_disabled() {
        let r = LeptessEngine::new();
        assert!(matches!(r, Err(OcrError::FeatureDisabled(_))));
    }
}
