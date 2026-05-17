//! User-facing log directory resolution.
//!
//! Resolves an OS-appropriate writable directory for tracing-appender's
//! rolling file output. Picked by hand from environment variables so
//! the workspace doesn't pull in `dirs` for one path lookup.
//!
//! - Windows: `%LOCALAPPDATA%\FreeJoyXConfigurator\logs`
//! - macOS:   `$HOME/Library/Logs/FreeJoyXConfigurator`
//! - Linux:   `$XDG_STATE_HOME/freejoyx-configurator/logs`
//!   (falling back to `$HOME/.local/state/freejoyx-configurator/logs`)
//!
//! The fallback in every branch is `std::env::temp_dir()` so the app
//! never panics on a missing env var.

use std::env;
use std::path::PathBuf;

/// Return a writable directory the app will use for log files.
///
/// The caller is responsible for creating the directory if it doesn't
/// exist; `tracing-appender::rolling::daily()` does this implicitly on
/// first write.
#[must_use]
pub fn resolve() -> PathBuf {
    if cfg!(target_os = "windows") {
        if let Some(base) = env::var_os("LOCALAPPDATA") {
            return PathBuf::from(base)
                .join("FreeJoyXConfigurator")
                .join("logs");
        }
    } else if cfg!(target_os = "macos") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Logs")
                .join("FreeJoyXConfigurator");
        }
    } else {
        // Linux & friends.
        if let Some(state) = env::var_os("XDG_STATE_HOME") {
            return PathBuf::from(state)
                .join("freejoyx-configurator")
                .join("logs");
        }
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("state")
                .join("freejoyx-configurator")
                .join("logs");
        }
    }
    env::temp_dir().join("freejoyx-configurator-logs")
}

/// Open the user's log folder in the system file manager.
///
/// Returns `Ok(())` when the spawn succeeded; the file manager may
/// surface its own error to the user if the path doesn't exist.
///
/// # Errors
///
/// Propagates any `std::io::Error` returned by `Command::spawn`.
pub fn open_in_file_manager() -> std::io::Result<()> {
    let path = resolve();
    let _ = std::fs::create_dir_all(&path);
    let cmd = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(cmd).arg(&path).spawn()?;
    Ok(())
}
