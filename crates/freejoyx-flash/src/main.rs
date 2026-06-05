//! CLI entry point. Contract (consumed by the Qt configurator's
//! `DfuInstallSession`):
//!
//! ```text
//! freejoyx-flash probe    --board f411 [--check-driver] [--verbose]
//! freejoyx-flash install  --board f411 --boot <bootBin> --app <appBin>
//! freejoyx-flash bind     --board f411
//! freejoyx-flash selftest --board f411 [--level quick|full]
//! ```
//!
//! `selftest` is a non-destructive flash exercise: it erases the config scratch
//! sector, writes a known pattern (a few KB for `quick`, the whole 64 KB sector
//! for `full`), reads it back, and re-erases it — never touching the bootloader
//! or app. It reuses the exact erase/write/verify path an install uses, so a
//! pass means a real install would write cleanly, and the retry tally it reports
//! is a direct read on USB-link / power health.
//!
//! `probe` reports `present` / `needs-driver` / `absent`. The bare form is the
//! cheap nusb-only check (the configurator's every-tick poll). `--check-driver`
//! additionally consults the WinUSB driver layer so a board that's in DFU but
//! not yet WinUSB-bound reports `needs-driver` instead of `absent`, *without*
//! the verbose logging — the configurator asks for it on a slow cadence so the
//! "Install WinUSB driver" action can appear on its own. `--verbose` implies
//! `--check-driver` and also narrates the enumeration (for a manual re-check).
//!
//! `bind` installs/repairs the WinUSB binding for the ROM DFU device on its
//! own (the same step `install` runs first). The configurator calls it when a
//! probe reports `needs-driver` — the device is present at the OS driver layer
//! but not yet usable by the flasher.
//!
//! All progress/results go to stdout as the line protocol in [`proto`].
//! Exit code 0 == success, non-zero == failure (with an `ERROR` line first).

use freejoyx_flash::dfuse::{self, Dfu, APP_ADDR, BOOT_ADDR};
use freejoyx_flash::{driver, proto};
use proto::Stage;

/// Config storage sector base (F411 S4). Erased on a full reinstall so the
/// device comes up factory-default rather than with a stale mapping.
const CONFIG_ADDR: u32 = 0x0801_0000;

fn main() {
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        proto::error(
            "usage",
            "expected `probe`, `install`, `selftest`, or `bind` subcommand",
        );
        return 2;
    }

    match args[0].as_str() {
        "probe" => cmd_probe(&args),
        "install" => cmd_install(&args),
        "selftest" => cmd_selftest(&args),
        // Install/repair the WinUSB binding by itself (the same step `install`
        // runs first). Surfaced by the configurator's "Install WinUSB driver"
        // action when a probe reports `needs-driver`.
        "bind" => cmd_bind(&args),
        // Internal: the self-elevated WinUSB driver install (invoked with UAC
        // by `bind`/`install`). Not meant to be run by hand.
        "bind-winusb" => cmd_bind_winusb(),
        other => {
            proto::error("usage", &format!("unknown subcommand `{other}`"));
            2
        }
    }
}

#[cfg(all(windows, feature = "winusb-autobind"))]
fn cmd_bind_winusb() -> i32 {
    match driver::bind_winusb_now() {
        Ok(()) => 0,
        Err(e) => {
            proto::error("bind", &e);
            1
        }
    }
}

#[cfg(not(all(windows, feature = "winusb-autobind")))]
fn cmd_bind_winusb() -> i32 {
    proto::error("bind", "WinUSB auto-bind is not built into this helper");
    2
}

fn cmd_probe(args: &[String]) -> i32 {
    // Board is parsed for contract stability but unused — a ROM DFU device
    // looks the same regardless of which app is (or isn't) flashed.
    let _ = flag(args, "--board");
    // `--verbose` makes the probe narrate what it enumerated via LOG lines so
    // an "absent" verdict is diagnosable from the configurator's log pane
    // (which device IDs were seen, whether 0483:df11 was among them, the bound
    // driver). The configurator passes it only for a manual re-check, never the
    // background poll, so the log doesn't fill with noise.
    let verbose = has_flag(args, "--verbose");
    // `--check-driver` consults the WinUSB driver layer (libwdi) to tell a
    // present-but-unbound ROM DFU (`needs-driver`) from a truly absent board —
    // WITHOUT the verbose per-device enumeration logging. This is deliberately
    // decoupled from `--verbose`: the background poll needs the `needs-driver`
    // verdict to surface the "Install WinUSB driver" action on its own, but it
    // must not spam the log every tick. `--verbose` still implies the check
    // (and adds the narration) for a manual re-check. The libwdi enumeration is
    // heavier than nusb's, so the configurator only asks for it on a slow
    // cadence, not every poll.
    let check_driver = verbose || has_flag(args, "--check-driver");
    if verbose {
        dfuse::probe_verbose();
    }
    // `present` = nusb can enumerate (and so open) the WinUSB-bound ROM DFU.
    // When it can't, the device may still be physically present but not yet
    // WinUSB-bound — nusb is blind to that, but libwdi isn't. A `needs-driver`
    // verdict lets the configurator offer to install the binding.
    let result = if dfuse::device_present() {
        proto::Probe::Present
    } else if check_driver && driver::driver_layer_present() {
        proto::Probe::NeedsDriver
    } else {
        proto::Probe::Absent
    };
    proto::probe(result);
    0
}

/// Install/repair the WinUSB binding for the ROM DFU device, standalone. This
/// is the same `ensure_reachable` step `install` runs first; exposing it lets
/// the configurator fix the driver before nusb can see the device (the probe
/// `needs-driver` case). One UAC prompt on Windows; a no-op elsewhere.
fn cmd_bind(args: &[String]) -> i32 {
    let _ = flag(args, "--board");
    match driver::ensure_reachable() {
        Ok(()) => {
            proto::log("WinUSB driver step complete");
            0
        }
        Err(e) => {
            proto::error("bind", &e);
            1
        }
    }
}

fn cmd_install(args: &[String]) -> i32 {
    let board = flag(args, "--board").unwrap_or_default();
    if board != "f411" {
        proto::error("board", "only --board f411 is supported");
        return 2;
    }
    let boot_path = match flag(args, "--boot") {
        Some(p) => p,
        None => {
            proto::error("usage", "missing --boot <path>");
            return 2;
        }
    };
    let app_path = match flag(args, "--app") {
        Some(p) => p,
        None => {
            proto::error("usage", "missing --app <path>");
            return 2;
        }
    };

    let boot = match std::fs::read(&boot_path) {
        Ok(b) => b,
        Err(e) => {
            proto::error("file", &format!("can't read bootloader {boot_path}: {e}"));
            return 1;
        }
    };
    let app = match std::fs::read(&app_path) {
        Ok(b) => b,
        Err(e) => {
            proto::error("file", &format!("can't read app {app_path}: {e}"));
            return 1;
        }
    };
    if boot.is_empty() || app.is_empty() {
        proto::error("file", "bootloader or app binary is empty");
        return 1;
    }

    match run_install(&boot, &app) {
        Ok(()) => {
            proto::stage(Stage::Done);
            0
        }
        Err(msg) => {
            proto::error("dfu", &msg);
            1
        }
    }
}

/// How many times to re-open the device and redo the whole erase+write+verify
/// sequence when an attempt fails on a transient (link/power) wobble. A fresh
/// re-open clears a wedged WinUSB pipe and a re-erase wipes any half-written
/// sector, so a second pass often sails through where one block stalled out the
/// first. Overridable for a really stubborn board. `FREEJOYX_FLASH_INSTALL_ATTEMPTS`.
fn install_attempts() -> u32 {
    std::env::var("FREEJOYX_FLASH_INSTALL_ATTEMPTS")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(3)
}

fn run_install(boot: &[u8], app: &[u8]) -> Result<(), String> {
    proto::log(&format!(
        "install: bootloader {} bytes -> 0x{BOOT_ADDR:08x}, app {} bytes -> 0x{APP_ADDR:08x}",
        boot.len(),
        app.len(),
    ));
    proto::stage(Stage::BindDriver);
    driver::ensure_reachable()?;

    // Each attempt re-opens the device from scratch: a failed write leaves a
    // wedged pipe / partially-written sector, and a clean re-open + re-erase is
    // the strongest recovery available on Windows (where a USB port reset isn't).
    let attempts = install_attempts();
    let mut last_err = String::new();
    for attempt in 1..=attempts {
        if attempt > 1 {
            proto::log(&format!(
                "retrying the whole install from a fresh DFU session (attempt {attempt}/{attempts})"
            ));
        }
        match install_once(boot, app) {
            Ok(dfu) => {
                // Only now manifest + reset — leaving DFU on a failed attempt
                // would strand a half-written board. With manual BOOT0 entry the
                // user still has to release BOOT0 and replug; the configurator's
                // instructions cover that.
                dfu.leave();
                return Ok(());
            }
            Err(e) => {
                proto::log(&format!("install attempt {attempt} failed: {e}"));
                last_err = e;
            }
        }
    }
    Err(format!(
        "{last_err} (after {attempts} attempt(s) — this usually means a flaky USB \
         cable/port or insufficient power to the board; try a different short cable \
         and a rear USB 2.0 port, or flash with an ST-Link)"
    ))
}

/// One full erase+write+verify pass on a freshly opened device. Returns the open
/// [`Dfu`] (still in DFU, not yet `leave()`n) on success so the caller can
/// manifest only when the whole install — verify included — has passed.
fn install_once(boot: &[u8], app: &[u8]) -> Result<Dfu, String> {
    let dfu = open_with_retry()?;
    dfu.to_idle()?;

    proto::stage(Stage::Erase);
    proto::log("erasing bootloader, config and app sectors");
    dfu.erase_region(BOOT_ADDR, boot.len() as u32)?;
    dfu.erase_region(CONFIG_ADDR, 1)?; // wipe config -> factory defaults
    dfu.erase_region(APP_ADDR, app.len() as u32)?;

    proto::stage(Stage::WriteBoot);
    dfu.write_image(BOOT_ADDR, boot, proto::progress)?;

    proto::stage(Stage::WriteApp);
    dfu.write_image(APP_ADDR, app, proto::progress)?;

    // Read both images back and confirm they landed. A genuine mismatch fails
    // the attempt (so the retry re-erases and rewrites); a board that simply
    // refuses UPLOAD can't be confirmed either way, so we keep the install but
    // warn loudly rather than claim a success we couldn't check.
    proto::stage(Stage::Verify);
    verify_or_fail(&dfu, BOOT_ADDR, boot, "bootloader")?;
    verify_or_fail(&dfu, APP_ADDR, app, "app")?;

    let s = dfu.stats();
    if s.block_retries > 0 {
        proto::log(&format!(
            "note: needed {} block retr{} across {} written block(s) — the link/power \
             is marginal but the write verified",
            s.block_retries,
            if s.block_retries == 1 { "y" } else { "ies" },
            s.blocks_written,
        ));
    }
    Ok(dfu)
}

/// Read-back verify that gates success. A `Mismatch` is a real bad write — fail
/// the attempt so it's re-erased and rewritten. `Unreadable` (the device refuses
/// UPLOAD) isn't proof of a bad write, so it can't be a hard failure on its own
/// — but we surface it as a prominent warning so a board that "installed" yet
/// won't boot isn't a silent mystery.
fn verify_or_fail(dfu: &Dfu, base: u32, data: &[u8], what: &str) -> Result<(), String> {
    use freejoyx_flash::dfuse::VerifyError;
    match dfu.verify_image(base, data, proto::progress) {
        Ok(()) => {
            proto::log(&format!(
                "{what} verified ({} bytes read back OK)",
                data.len()
            ));
            Ok(())
        }
        Err(VerifyError::Mismatch { addr }) => Err(format!(
            "{what} verify FAILED — flash differs at 0x{addr:08x}"
        )),
        Err(VerifyError::Unreadable(e)) => {
            proto::log(&format!(
                "WARNING: could not verify {what} — {e}. The write was accepted but this \
                 board won't allow read-back, so it can't be confirmed. If the board \
                 doesn't enumerate after you replug it, reflash or use an ST-Link."
            ));
            Ok(())
        }
    }
}

fn cmd_selftest(args: &[String]) -> i32 {
    let board = flag(args, "--board").unwrap_or_else(|| "f411".to_string());
    if board != "f411" {
        proto::error("board", "only --board f411 is supported");
        return 2;
    }
    let level = flag(args, "--level").unwrap_or_else(|| "quick".to_string());
    let bytes = match level.as_str() {
        // A few blocks: a fast "is the link sane?" check.
        "quick" => 8 * 1024,
        // The whole config scratch sector: stresses the write path about as hard
        // as a real bootloader+app install does.
        "full" => dfuse::SCRATCH_LEN as usize,
        other => {
            proto::error(
                "usage",
                &format!("unknown --level `{other}` (use quick or full)"),
            );
            return 2;
        }
    };
    match run_selftest(bytes, &level) {
        Ok(()) => {
            proto::stage(Stage::Done);
            0
        }
        Err(msg) => {
            proto::error("selftest", &msg);
            1
        }
    }
}

/// Non-destructive flash exercise on the config scratch sector: erase, write a
/// known pattern, read it back, then re-erase so the board is left in the same
/// factory-default-config state a fresh install leaves. Reuses the real
/// erase/write/verify path, so a pass means an install would write cleanly; the
/// retry tally is the headline diagnostic.
fn run_selftest(bytes: usize, level: &str) -> Result<(), String> {
    use freejoyx_flash::dfuse::VerifyError;

    proto::log(&format!(
        "selftest ({level}): exercising {bytes} bytes on the config scratch sector \
         0x{:08x} (does not touch the bootloader or app)",
        dfuse::SCRATCH_ADDR,
    ));
    proto::stage(Stage::BindDriver);
    driver::ensure_reachable()?;

    let dfu = open_with_retry()?;
    dfu.to_idle()?;

    let pattern = dfuse::selftest_pattern(bytes);

    proto::stage(Stage::Erase);
    proto::log("erasing scratch sector");
    dfu.erase_region(dfuse::SCRATCH_ADDR, bytes as u32)?;

    proto::stage(Stage::Test);
    proto::log("writing test pattern");
    dfu.write_image(dfuse::SCRATCH_ADDR, &pattern, proto::progress)?;

    proto::stage(Stage::Verify);
    proto::log("reading the test pattern back");
    let verify = dfu.verify_image(dfuse::SCRATCH_ADDR, &pattern, proto::progress);

    // Always leave the scratch sector erased again so a tested board comes up
    // with factory-default config, exactly as after an install. Best-effort —
    // a failure here doesn't change the verdict but is worth noting.
    proto::stage(Stage::Erase);
    proto::log("restoring scratch sector (erasing test pattern)");
    if let Err(e) = dfu.erase_region(dfuse::SCRATCH_ADDR, bytes as u32) {
        proto::log(&format!("note: could not re-erase scratch sector: {e}"));
    }

    let s = dfu.stats();
    proto::log(&format!(
        "selftest stats: {} block(s) written, {} retr{}, {} failed",
        s.blocks_written,
        s.block_retries,
        if s.block_retries == 1 { "y" } else { "ies" },
        s.blocks_failed,
    ));

    match verify {
        Ok(()) => {
            if s.block_retries == 0 {
                proto::log(
                    "PASS: wrote and read back cleanly with no retries — link looks healthy",
                );
            } else {
                proto::log(
                    "PASS: data verified, but the write needed retries — the board flashes \
                     but the USB link/power is marginal (consider a different cable / rear \
                     USB 2.0 port)",
                );
            }
            Ok(())
        }
        Err(VerifyError::Mismatch { addr }) => Err(format!(
            "FAIL: read-back differs at 0x{addr:08x} — data is being corrupted on the way \
             to or from the chip (suspect the USB cable/port or board power)"
        )),
        Err(VerifyError::Unreadable(e)) => Err(format!(
            "INCONCLUSIVE: the write was accepted but the board refused read-back ({e}). \
             Writes can't be verified on this board over USB DFU; use an ST-Link to confirm."
        )),
    }
}

/// The user has just entered DFU (BOOT0 + reset), so the device may take a
/// moment to settle / re-enumerate. Retry the open a few times before giving
/// up with the platform-specific hint.
fn open_with_retry() -> Result<Dfu, String> {
    let mut last = String::new();
    for attempt in 0..5 {
        match Dfu::open() {
            Ok(d) => return Ok(d),
            Err(e) => {
                last = e;
                std::thread::sleep(std::time::Duration::from_millis(400));
                let _ = attempt;
            }
        }
    }
    Err(format!("{last}. {}", driver::open_failure_hint()))
}

/// Tiny flag reader: returns the value following `name`, if present.
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Presence check for a valueless flag (e.g. `--verbose`).
fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}
