//! helpers shared by the unit tests

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// a fresh folder under the system temporary folder, deleted when dropped
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "sailtree-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temporary folder creation");
        // resolve symlinks so paths compare equal to what the code under test reports
        TempDir(path.canonicalize().expect("temporary folder path"))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
