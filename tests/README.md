# Regression tests

Step 5.7 (M4.5) 端到端回归集成测试。

## 实际位置

由于 Cargo workspace 不自动运行 workspace 根 `tests/` 下的 Rust 文件（必须挂到某个 crate 的 `tests/` 子目录），本步骤把执行计划要求的
`k2pdfopt-rs/tests/regression.rs` 实际落在：

- **`tools/compare_pages/tests/regression.rs`** — 6+ 个端到端集成测试
  覆盖 PNG self-compare（SSIM=1.0 sanity）、fixture 完整性、mutool 可达性检测、PDF
  端到端 self-compare。

## 跑法

```powershell
# 全工作区集成测试（含本回归套件）
cargo test --workspace -j1

# 仅跑回归套件
cargo test -p compare-pages --test regression

# 批量跑 12 fixture 的 SSIM 对照报告
cargo run --release --bin run_regression -- --all
```

报告输出在 `tests/golden/_regression/<timestamp>/` 下，含：
- `summary.json` — 跨 fixture 汇总
- `<fixture>.json` / `<fixture>.html` — 单 fixture 详情
- `index.html` — 链接到所有单页报告

## 当前阶段约束

- **M5 (Step 7.x) 之前**：Rust 端尚未产出 PDF。`run_regression --mode self`（默认）
  把 `c-output.pdf` 与自身比对，验证 SSIM 工具链 sanity；`--mode rust-vs-c` 要求
  fixture 旁有 `rs-output.pdf`，否则报错（stub）。
- **M5 起**：把 pipeline 输出的 PDF 写到 `tests/golden/<fixture>/rs-output.pdf`，
  即可切到 `--mode rust-vs-c` 做真实回归对照。
