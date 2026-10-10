//! In-memory recovery entries, controllable clocks and area locks.

use super::{Arc, BTreeMap, BTreeSet, Hold, Lock, Mutex, RecoveryStore};

/// A store in memory (tests, and hosts without storage). Areas are locked in memory, so sessions
/// sharing one store behave like apps sharing a folder; [`MemoryStore::without_locks`] uses
/// heartbeats like browser storage, judged by a clock tests set ([`MemoryStore::set_now`]), and
/// [`MemoryStore::announcing`] adds announcements like the browser's Web Locks.
pub struct MemoryStore {
    entries: Mutex<BTreeMap<String, Vec<u8>>>,
    held: Arc<Mutex<BTreeSet<String>>>,
    locks: bool,
    announces: bool,
    now: Mutex<Option<i64>>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self { entries: Default::default(), held: Default::default(), locks: true, announces: false, now: Mutex::new(Some(1_000_000)) }
    }
}

impl MemoryStore {
    /// A store without locks: areas are held with heartbeats.
    pub fn without_locks() -> Self {
        Self { locks: false, ..Self::default() }
    }

    /// A store without locks whose apps also announce the areas they hold for as long as they run
    /// (as the browser's Web Locks do).
    pub fn announcing() -> Self {
        Self { announces: true, ..Self::without_locks() }
    }

    fn hold(&self, area: &str) -> Option<Hold> {
        if !self.held.lock().unwrap_or_else(|e| e.into_inner()).insert(area.to_string()) {
            return None;
        }
        Some(Box::new(MemoryHold { held: self.held.clone(), area: area.to_string() }))
    }

    /// Set the clock heartbeats are judged by.
    pub fn set_now(&self, t: i64) {
        *self.now.lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Vec<u8>>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// An area of a [`MemoryStore`] held until dropped.
struct MemoryHold {
    held: Arc<Mutex<BTreeSet<String>>>,
    area: String,
}

impl Drop for MemoryHold {
    fn drop(&mut self) {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.area);
    }
}

impl RecoveryStore for MemoryStore {
    fn list(&self) -> std::result::Result<Vec<String>, String> {
        Ok(self.entries().keys().cloned().collect())
    }
    fn read(&self, name: &str) -> std::result::Result<Vec<u8>, String> {
        self.entries().get(name).cloned().ok_or_else(|| format!("{name}: no such recovery entry"))
    }
    fn write(&self, name: &str, bytes: &[u8]) -> std::result::Result<(), String> {
        self.entries().insert(name.to_string(), bytes.to_vec());
        Ok(())
    }
    fn remove(&self, name: &str) -> std::result::Result<(), String> {
        self.entries().remove(name);
        Ok(())
    }
    fn location(&self) -> String {
        "memory".into()
    }
    fn lock(&self, area: &str) -> std::result::Result<Lock, String> {
        if !self.locks {
            return Ok(Lock::Unsupported);
        }
        Ok(self.hold(area).map_or(Lock::Busy, Lock::Held))
    }
    fn announce(&self, area: &str) -> Option<Hold> {
        if !self.announces {
            return None;
        }
        self.hold(area)
    }
    fn announced(&self, area: &str) -> Option<bool> {
        self.announces.then(|| self.held.lock().unwrap_or_else(|e| e.into_inner()).contains(area))
    }
    fn now(&self) -> Option<i64> {
        *self.now.lock().unwrap_or_else(|e| e.into_inner())
    }
}
