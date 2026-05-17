//! Lightweight UI preferences persistence.
//!
//! Stores user choices that survive across sessions but don't belong
//! in `freejoyx-config.ron` (which is the *device* config, not app
//! state). Today: just the light/dark theme. Format is intentionally
//! trivial — one `key=value` line per setting — so adding a new
//! preference doesn't pull in a serializer.

use std::fs;
use std::path::PathBuf;

use crate::log_dir;

/// File the preferences live in. Sibling of the `logs/` directory so
/// both pieces of app state live in the same OS-conventional spot.
fn settings_path() -> PathBuf {
    let logs = log_dir::resolve();
    let parent = logs.parent().map_or_else(|| logs.clone(), Into::into);
    parent.join("settings.txt")
}

/// Read the persisted theme choice. Returns `true` for dark (the
/// default) when no setting file exists or the file is unparseable.
#[must_use]
pub fn load_dark() -> bool {
    let Ok(body) = fs::read_to_string(settings_path()) else {
        return true;
    };
    for line in body.lines() {
        if let Some(v) = line.strip_prefix("theme=") {
            return v.trim() != "light";
        }
    }
    true
}

/// Persist the theme choice. Best-effort: creates the parent
/// directory if missing and logs at `warn` level on any IO failure
/// rather than propagating — a missing preference shouldn't break a
/// session.
pub fn save_dark(dark: bool) {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let body = format!("theme={}\n", if dark { "dark" } else { "light" });
    if let Err(e) = fs::write(&path, body) {
        tracing::warn!("could not persist theme to {}: {e}", path.display());
    }
}
