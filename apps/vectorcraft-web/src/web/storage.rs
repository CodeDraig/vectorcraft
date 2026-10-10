//! Browser recovery entries in local storage and Web Locks.

use super::{Hold, RecoveryStore, WebLocks, js_err};

/// Data Recovery's store on the web: the browser's local storage (kept across visits and shared by
/// the site's tabs), each entry as base64 under [`RECOVERY_PREFIX`]`<area>/<name>`. It has no
/// locks: each tab holds its area with a heartbeat, judged by the browser's clock, and with a Web
/// Lock named after it where the browser has them (`locks`), which a tab whose timers are paused
/// in the background keeps.
pub(super) struct BrowserStore {
    pub(super) locks: Option<WebLocks>,
}

pub(super) const RECOVERY_PREFIX: &str = "vectorcraft-recovery/";

fn local_storage() -> Result<web_sys::Storage, String> {
    web_sys::window().ok_or("no window")?.local_storage().map_err(js_err)?.ok_or_else(|| "browser storage is turned off".into())
}

impl RecoveryStore for BrowserStore {
    fn list(&self) -> Result<Vec<String>, String> {
        let s = local_storage()?;
        let n = s.length().map_err(js_err)?;
        Ok((0..n).filter_map(|i| s.key(i).ok().flatten()).filter_map(|k| k.strip_prefix(RECOVERY_PREFIX).map(str::to_string)).collect())
    }
    fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let text = local_storage()?.get_item(&format!("{RECOVERY_PREFIX}{name}")).map_err(js_err)?;
        text.and_then(|t| vectorcraft_format::base64_decode(&t)).ok_or_else(|| format!("{name}: no such recovery entry"))
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        local_storage()?
            .set_item(&format!("{RECOVERY_PREFIX}{name}"), &vectorcraft_format::base64_encode(bytes))
            .map_err(|e| format!("browser storage is full or turned off: {}", js_err(e)))
    }
    fn remove(&self, name: &str) -> Result<(), String> {
        local_storage()?.remove_item(&format!("{RECOVERY_PREFIX}{name}")).map_err(js_err)
    }
    fn location(&self) -> String {
        "browser storage".into()
    }
    fn now(&self) -> Option<i64> {
        Some((js_sys::Date::now() / 1000.0) as i64)
    }
    fn announce(&self, area: &str) -> Option<Hold> {
        self.locks.as_ref()?.hold(area)
    }
    fn announced(&self, area: &str) -> Option<bool> {
        self.locks.as_ref().map(|l| l.held(area))
    }
}
