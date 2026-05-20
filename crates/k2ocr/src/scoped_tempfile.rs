//! 极简的"作用域临时文件"——避免引入 `tempfile` crate（与项目"零冗余依赖"原则同源）。
//!
//! 设计要点：
//! - 命名 = `<prefix>-<pid>-<nanos>-<counter><suffix>`，跨进程/线程安全无冲突
//! - `Drop` 自动 `remove_file`（即使中间 tesseract 调用失败也会清理）
//! - 仅在 [`std::env::temp_dir`] 下分配；权限/磁盘满 错误直接透传
//! - 不打开句柄 (only path-based)，避免与 tesseract 子进程争用文件锁

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 进程级单调计数器，保证同毫秒内多次构造也不冲突。
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 自动清理的临时文件路径句柄。
pub(crate) struct ScopedTempFile {
    path: PathBuf,
}

impl ScopedTempFile {
    /// 在系统临时目录下分配一个**尚未存在**的路径（不创建文件，留给调用方写）。
    ///
    /// 命名格式：`<temp>/<prefix>-<pid>-<nanos>-<counter><suffix>`。
    pub(crate) fn allocate(prefix: &str, suffix: &str) -> std::io::Result<Self> {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let filename = format!("{prefix}-{pid}-{nanos}-{id}{suffix}");
        let path = std::env::temp_dir().join(filename);
        // 注意：不主动 create，由调用方写 PNG 时一次性建立。
        Ok(Self { path })
    }

    /// 路径（绝对）。
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScopedTempFile {
    fn drop(&mut self) {
        // 文件可能因调用方未写或写失败而不存在；忽略错误。
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn allocates_unique_paths() {
        let a = ScopedTempFile::allocate("k2ocr-test", ".png").unwrap();
        let b = ScopedTempFile::allocate("k2ocr-test", ".png").unwrap();
        assert_ne!(a.path(), b.path());
    }

    #[test]
    fn path_under_temp_dir() {
        let f = ScopedTempFile::allocate("k2ocr-test", ".png").unwrap();
        assert!(f.path().starts_with(std::env::temp_dir()));
    }

    #[test]
    fn drop_removes_existing_file() {
        let path_copy;
        {
            let f = ScopedTempFile::allocate("k2ocr-drop", ".txt").unwrap();
            std::fs::write(f.path(), b"hello").unwrap();
            assert!(f.path().exists());
            path_copy = f.path().to_path_buf();
        }
        assert!(!path_copy.exists());
    }

    #[test]
    fn drop_when_file_never_created_ok() {
        let path_copy;
        {
            let f = ScopedTempFile::allocate("k2ocr-noop", ".txt").unwrap();
            path_copy = f.path().to_path_buf();
            // 不创建文件直接 drop
        }
        assert!(!path_copy.exists());
    }

    #[test]
    fn filename_uses_prefix_and_suffix() {
        let f = ScopedTempFile::allocate("custom-pfx", ".dat").unwrap();
        let name = f.path().file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("custom-pfx-"));
        assert!(name.ends_with(".dat"));
    }
}
