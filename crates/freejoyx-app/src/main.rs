//! FreeJoyXConfigurator entry point.
//!
//! Slice 4/5 CLI preview. The Slint UI (Slice 5) wraps the same
//! channel API the subcommands here exercise.
//!
//! - `freejoyx-app list` — print enumerated FreeJoyX HID candidates.
//! - `freejoyx-app watch` — spawn the worker, print live params reports
//!   and connect/disconnect events.
//! - `freejoyx-app read-config` — spawn the worker, wait for connect,
//!   send `Command::ReadConfig`, print a human-readable dump of the
//!   returned `DeviceConfig`. Useful for validating the codec
//!   end-to-end against a real device.
//!
//! No `clap` dependency: the surface is a handful of verbs and the
//! workspace's "no new deps without reason" rule (Port.md §3) means
//! `std::env::args` is the right tool.

use std::env;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use freejoyx_core::wire::{format_config, ParamsReport};
use freejoyx_device::{enumerate, spawn_for_serial, Command, DeviceEvent};

fn main() -> ExitCode {
    let _log_guard = init_tracing();

    let mut args = env::args().skip(1).collect::<Vec<_>>().into_iter();
    let cmd = args.next();
    let serial = parse_serial_flag(args.collect::<Vec<_>>());

    match cmd.as_deref() {
        Some("list") => run_list(),
        Some("watch") => run_watch(serial),
        Some("read-config") => run_read_config(serial),
        Some("ui") | None => run_ui(serial),
        Some("--help" | "-h" | "help") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("freejoyx-app: unknown subcommand '{other}'\n");
            print_help();
            ExitCode::from(2)
        }
    }
}

fn run_ui(serial: Option<String>) -> ExitCode {
    match freejoyx_ui::run(serial) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("UI failed to start: {e}");
            ExitCode::from(1)
        }
    }
}

/// Pull `--serial <hex>` out of the remaining args. Multi-board users
/// need this to pick a specific device; without it `open_first()`
/// picks the first enumerated candidate, which on a busy bench is
/// arbitrary.
fn parse_serial_flag(args: Vec<String>) -> Option<String> {
    let mut iter = args.into_iter();
    while let Some(a) = iter.next() {
        if a == "--serial" {
            return iter.next();
        }
        if let Some(s) = a.strip_prefix("--serial=") {
            return Some(s.to_string());
        }
    }
    None
}

/// Initialise tracing with a stderr fmt layer and a rolling daily file
/// in [`freejoyx_ui::log_dir`]. Returns the non-blocking-writer guard
/// for the file layer; dropping it on shutdown lets pending writes
/// flush.
fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let log_dir = freejoyx_ui::log_dir::resolve();
    let (file_layer, guard) = match std::fs::create_dir_all(&log_dir) {
        Ok(()) => {
            let appender = tracing_appender::rolling::daily(&log_dir, "freejoyx-app.log");
            let (nb, guard) = tracing_appender::non_blocking(appender);
            (
                Some(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(nb),
                ),
                Some(guard),
            )
        }
        Err(_) => (None, None),
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(file_layer)
        .init();

    guard
}

fn print_help() {
    println!(
        "freejoyx-app — FreeJoyXConfigurator (Slice 5 CLI preview)\n\
         \n\
         USAGE:\n    \
             freejoyx-app [SUBCOMMAND] [--serial <hex>]\n\
         \n\
         SUBCOMMANDS:\n    \
             ui                         Launch the Slint UI (default if no subcommand)\n    \
             list                       Enumerate FreeJoyX-style HID devices\n    \
             watch [--serial <hex>]     Spawn the worker and print live params reports\n    \
             read-config [--serial <hex>]\n                                    \
                                        Read the connected device's dev_config_t\n                                    \
                                        and print a human-readable dump\n    \
             help                       Show this message\n\
         \n\
         --serial <hex> selects a specific board by HID serial number\n\
         (see the rightmost column of `list`). Without it the worker\n\
         opens the first enumerated FreeJoyX-style device, which on a\n\
         multi-board bench is arbitrary."
    );
}

fn run_list() -> ExitCode {
    match enumerate() {
        Ok(candidates) if candidates.is_empty() => {
            eprintln!("no FreeJoyX devices found");
            ExitCode::from(1)
        }
        Ok(candidates) => {
            for c in &candidates {
                println!("{}", c.display_summary());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("enumerate failed: {e}");
            ExitCode::from(1)
        }
    }
}

fn run_watch(serial: Option<String>) -> ExitCode {
    let (_handle, rx) = spawn_for_serial(serial);
    eprintln!(
        "watching device events (Ctrl-C to quit). \
         worker will hot-plug to the first FreeJoyX device that appears.\n\
         tick columns: axes[8 i16]  phy: 16-byte hex bitmap  log: 16-byte hex bitmap  shifts: u8"
    );

    while let Ok(evt) = rx.recv() {
        match evt {
            DeviceEvent::Connected(c) => eprintln!("connected: {}", c.display_summary()),
            DeviceEvent::Disconnected => {
                eprintln!("disconnected — waiting for device to return");
            }
            DeviceEvent::ParamsTick(report) => println!("{}", format_report(&report)),
            DeviceEvent::ConfigReceived(_) | DeviceEvent::ConfigSent => {
                // These don't show up in watch (no commands sent); log
                // defensively if they ever do.
                eprintln!("note: unexpected config event during watch");
            }
            DeviceEvent::ConfigError(msg) => eprintln!("config error: {msg}"),
            DeviceEvent::Candidates(_) => {
                // `watch` doesn't issue Enumerate; ignore if one
                // sneaks in from another source.
            }
            DeviceEvent::Error(msg) => eprintln!("error: {msg}"),
        }
    }
    ExitCode::from(1)
}

/// Spawn the worker, wait for a device, request the full dev_config_t,
/// print a human-readable dump, exit. Useful for validating the codec
/// end-to-end against a real board: read → dump → eyeball the pins /
/// buttons / shift registers against what you set in the Qt app.
fn run_read_config(serial: Option<String>) -> ExitCode {
    const CONNECT_DEADLINE: Duration = Duration::from_secs(5);
    const RESPONSE_DEADLINE: Duration = Duration::from_secs(8);

    let (handle, rx) = spawn_for_serial(serial);
    eprintln!(
        "waiting for a FreeJoyX device (up to {} s)…",
        CONNECT_DEADLINE.as_secs()
    );

    // Step 1: wait for a Connected event.
    let connect_deadline = Instant::now() + CONNECT_DEADLINE;
    loop {
        let remaining = connect_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            eprintln!("timed out waiting for device");
            return ExitCode::from(1);
        }
        match rx.recv_timeout(remaining) {
            Ok(DeviceEvent::Connected(c)) => {
                eprintln!("connected: {}", c.display_summary());
                break;
            }
            Ok(DeviceEvent::Error(msg)) => eprintln!("transport: {msg}"),
            Ok(_) => continue,
            Err(_) => {
                eprintln!("timed out waiting for device");
                return ExitCode::from(1);
            }
        }
    }

    // Step 2: request the config and drain events until it lands.
    if handle.send(Command::ReadConfig).is_err() {
        eprintln!("worker exited before ReadConfig could be sent");
        return ExitCode::from(1);
    }
    eprintln!(
        "ReadConfig sent; waiting for response (up to {} s)…",
        RESPONSE_DEADLINE.as_secs()
    );

    let response_deadline = Instant::now() + RESPONSE_DEADLINE;
    loop {
        let remaining = response_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            eprintln!("timed out waiting for ConfigReceived");
            return ExitCode::from(1);
        }
        match rx.recv_timeout(remaining) {
            Ok(DeviceEvent::ConfigReceived(cfg)) => {
                println!("{}", format_config(&cfg));
                return ExitCode::SUCCESS;
            }
            Ok(DeviceEvent::ConfigError(msg)) => {
                eprintln!("config error: {msg}");
                return ExitCode::from(1);
            }
            Ok(DeviceEvent::Disconnected) => {
                eprintln!("device disconnected before responding");
                return ExitCode::from(1);
            }
            Ok(DeviceEvent::Error(msg)) => eprintln!("transport: {msg}"),
            Ok(_) => {} // ParamsTick / Connected / ConfigSent — ignore
            Err(_) => {
                eprintln!("timed out waiting for ConfigReceived");
                return ExitCode::from(1);
            }
        }
    }
}

fn format_report(r: &ParamsReport) -> String {
    let axes = r
        .axis_data
        .iter()
        .map(|v| format!("{v:>6}"))
        .collect::<Vec<_>>()
        .join(" ");
    let phy = bytes_hex(&r.phy_button_data);
    let log = bytes_hex(&r.log_button_data);
    format!(
        "axes: {axes}  phy: {phy}  log: {log}  shifts: {:02x}  fw: 0x{:04x}  board: {}",
        r.shift_button_data, r.firmware_version, r.board_id,
    )
}

fn bytes_hex(b: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(b.len() * 2);
    for byte in b {
        let _ = write!(s, "{byte:02x}");
    }
    s
}
