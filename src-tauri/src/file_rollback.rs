//! Keeps original files recoverable until a library mutation has committed.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub(crate) struct FileRollback {
    directory: PathBuf,
    copies: Vec<(PathBuf, Option<PathBuf>)>,
    committed: bool,
}
impl FileRollback {
    pub fn new(root: &Path, paths: &[&Path]) -> Result<Self, String> {
        let directory = root.join(format!(
            "{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let mut copies = Vec::new();
        for path in paths {
            if copies
                .iter()
                .any(|(p, _): &(PathBuf, Option<PathBuf>)| p == path)
            {
                continue;
            }
            if path.exists() {
                let copy = directory.join(copies.len().to_string());
                if let Err(e) = fs::copy(path, &copy) {
                    let _ = fs::remove_dir_all(&directory);
                    return Err(format!("Could not back up replacement file: {e}"));
                }
                copies.push((path.to_path_buf(), Some(copy)));
            } else {
                copies.push((path.to_path_buf(), None));
            }
        }
        Ok(Self {
            directory,
            copies,
            committed: false,
        })
    }
    pub fn commit(mut self) {
        self.committed = true;
        let _ = fs::remove_dir_all(&self.directory);
    }
}
impl Drop for FileRollback {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut restored = true;
        for (path, copy) in &self.copies {
            let result = match copy {
                Some(copy) => fs::copy(copy, path).map(|_| ()),
                None => {
                    if path.exists() {
                        fs::remove_file(path)
                    } else {
                        Ok(())
                    }
                }
            };
            if let Err(error) = result {
                restored = false;
                tracing::error!(%error,backup=%self.directory.display(),"Replacement rollback needs recovery from backup");
            }
        }
        if restored {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_restores_old_file_and_removes_new_target() {
        let root = std::env::temp_dir().join(format!("player-rollback-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let original = root.join("old.flac");
        let target = root.join("new.flac");
        fs::write(&original, b"original").unwrap();
        {
            let _guard = FileRollback::new(&root.join("backup"), &[&original, &target]).unwrap();
            fs::remove_file(&original).unwrap();
            fs::write(&target, b"replacement").unwrap();
        }
        assert_eq!(fs::read(&original).unwrap(), b"original");
        assert!(!target.exists());
        let _ = fs::remove_dir_all(root);
    }
}
