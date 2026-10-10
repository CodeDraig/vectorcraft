//! Filesystem recovery entries, atomic writes and held file locks.

use super::{Lock, RecoveryStore};

/// A folder of files (desktop): an area is a sub-folder whose `.lock` file its app keeps locked
/// while it runs. Created when the first copy is written.
#[cfg(not(target_arch = "wasm32"))]
pub struct FolderStore(std::path::PathBuf);

/// The lock file of a [`FolderStore`] area.
#[cfg(not(target_arch = "wasm32"))]
const LOCK_FILE: &str = ".lock";

#[cfg(not(target_arch = "wasm32"))]
impl FolderStore {
    pub fn new(folder: impl Into<std::path::PathBuf>) -> Self {
        Self(folder.into())
    }

    /// Is `part` a plain file or folder name (no separators, not hidden)?
    fn plain(part: &str) -> bool {
        !part.starts_with('.') && std::path::Path::new(part).file_name().is_some_and(|f| f == part)
    }

    /// The folder of area `area`.
    fn area(&self, area: &str) -> std::result::Result<std::path::PathBuf, String> {
        if !Self::plain(area) {
            return Err(format!("`{area}` is no recovery area name"));
        }
        Ok(self.0.join(area))
    }

    /// The file of entry `name` (`<area>/<file>`: anything else is refused).
    fn file(&self, name: &str) -> std::result::Result<std::path::PathBuf, String> {
        match name.split_once('/') {
            Some((area, file)) if Self::plain(file) => Ok(self.area(area)?.join(file)),
            _ => Err(format!("`{name}` is no recovery entry name")),
        }
    }

    /// The plain names of the files (or, `dirs`, folders) in `dir`; none when it doesn't exist.
    fn names(dir: &std::path::Path, dirs: bool) -> std::result::Result<Vec<String>, String> {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(format!("{}: {e}", dir.display())),
        };
        Ok(entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|t| if dirs { t.is_dir() } else { t.is_file() }))
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            // Lock files, and files left by an interrupted atomic write.
            .filter(|n| Self::plain(n))
            .collect())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl RecoveryStore for FolderStore {
    fn list(&self) -> std::result::Result<Vec<String>, String> {
        let mut out = vec![];
        for area in self.areas()? {
            out.extend(Self::names(&self.0.join(&area), false)?.into_iter().map(|f| format!("{area}/{f}")));
        }
        Ok(out)
    }
    fn read(&self, name: &str) -> std::result::Result<Vec<u8>, String> {
        let f = self.file(name)?;
        std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))
    }
    fn write(&self, name: &str, bytes: &[u8]) -> std::result::Result<(), String> {
        let f = self.file(name)?;
        if let Some(dir) = f.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        vectorcraft_format::write_atomic(&f, bytes).map_err(|e| format!("{}: {e}", f.display()))
    }
    fn remove(&self, name: &str) -> std::result::Result<(), String> {
        let f = self.file(name)?;
        match std::fs::remove_file(&f) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", f.display())),
            _ => Ok(()),
        }
    }
    fn location(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn areas(&self) -> std::result::Result<Vec<String>, String> {
        Self::names(&self.0, true)
    }
    fn lock(&self, area: &str) -> std::result::Result<Lock, String> {
        let dir = self.area(area)?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Lock::Held(Box::new(file))),
            Err(std::fs::TryLockError::WouldBlock) => Ok(Lock::Busy),
            Err(std::fs::TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
        }
    }
    fn remove_area(&self, area: &str) {
        // Best effort: whatever is left is tidied at a later launch. A folder that isn't empty
        // (copies came back meanwhile) stays.
        if let Ok(dir) = self.area(area) {
            let _ = std::fs::remove_file(dir.join(LOCK_FILE));
            let _ = std::fs::remove_dir(&dir);
        }
    }
}
