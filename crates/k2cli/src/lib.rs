//! `k2cli` - CLI 参数解析、环境变量合并、help text、settings <-> args 双向序列化入口
//!
//! 来源 C 文件：`k2pdfopt.c`、`k2parsecmd.c`（1628 行）、`k2usage.c`（1341 行）
//!
//! 详见 `rust-rewrite-plan.md` v2.1 §5.2 与附录 A。
//! M1 Step 3.4: clap derive + From<CliArgs> for Settings。
//! M1 Step 3.5: K2PDFOPT env merge + bidirectional serialize.
//! M1 Step 3.6: help text + subcommands.

#![forbid(unsafe_code)]

pub mod args;
pub mod env;
pub mod help;
pub mod subcommands;

pub use args::CliArgs;
pub use env::{build_settings, merge_env_and_cli, parse_env};
pub use subcommands::{cmd_compat_report, cmd_dry_run, cmd_echo_cmd, cmd_list_devices};
