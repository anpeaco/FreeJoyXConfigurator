//! Debug-tab structured log capture.
//!
//! The configurator surfaces an in-app "Debug" tab that streams every
//! significant action the UI / worker takes: device discovery, HID I/O,
//! config read/write, axis edits, button bitmap edges, etc. This
//! module owns the shared state that backs the tab:
//!
//! - [`LogEvent`] — one captured event (timestamp + level + category +
//!   message + structured fields).
//! - [`LogBuffer`] — bounded ring of recent events, sequenced so the UI
//!   can poll incrementally without re-scanning the whole buffer.
//! - [`DebugFilter`] — live filter (min level + category bitset). The
//!   tracing [`Layer`](crate::debug_log::BufferLayer) consults it on
//!   every event so filtered-out work never hits the buffer at all.
//! - [`BufferLayer`] — `tracing-subscriber` layer that maps each event
//!   into a [`LogEvent`] and pushes it into the buffer when the filter
//!   passes. Wired in `freejoyx-app::init_tracing` alongside the
//!   existing stderr + rolling-file layers.
//!
//! The stderr + daily-rotation file layers stay live unchanged; this
//! is purely additive.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// Cap on the ring buffer. Old entries drop off the front when the
/// buffer fills. 5000 is roughly 30 s of params ticks at 30 Hz worth
/// of buffer — enough to capture an incident without burning RAM.
pub const BUFFER_CAPACITY: usize = 5000;

/// Compact level the UI surfaces. Mirrors `tracing::Level` but encoded
/// as a stable u8 the Slint layer can switch off without an enum
/// crossing the FFI boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl LogLevel {
    /// Map from `tracing::Level`. Tracing's ordering is reversed
    /// (ERROR is the highest value internally), so the explicit match
    /// keeps the intent obvious.
    #[must_use]
    pub fn from_tracing(level: &Level) -> Self {
        match *level {
            Level::TRACE => Self::Trace,
            Level::DEBUG => Self::Debug,
            Level::INFO => Self::Info,
            Level::WARN => Self::Warn,
            Level::ERROR => Self::Error,
        }
    }

    #[must_use]
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO ",
            Self::Warn => "WARN ",
            Self::Error => "ERROR",
        }
    }

    /// 0-based index used by the Slint UI to switch a level → colour.
    #[must_use]
    pub fn ui_index(self) -> i32 {
        self as i32
    }
}

/// Logical event group. Drives the per-category filter on the Debug
/// tab. New categories slot in by extending this enum + the
/// `target_to_category` map below — call sites just use `target =
/// "freejoyx::<name>"` on their `tracing::info!` / `debug!` macros.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EventCategory {
    Device = 0,
    Hid = 1,
    Config = 2,
    Params = 3,
    Button = 4,
    Axis = 5,
    Pin = 6,
    Ui = 7,
    Other = 8,
}

impl EventCategory {
    pub const ALL: [Self; 9] = [
        Self::Device,
        Self::Hid,
        Self::Config,
        Self::Params,
        Self::Button,
        Self::Axis,
        Self::Pin,
        Self::Ui,
        Self::Other,
    ];

    #[must_use]
    pub fn ui_index(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub fn from_ui_index(i: i32) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.ui_index() == i)
    }

    #[must_use]
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Device => "DEVICE",
            Self::Hid => "HID",
            Self::Config => "CONFIG",
            Self::Params => "PARAMS",
            Self::Button => "BUTTON",
            Self::Axis => "AXIS",
            Self::Pin => "PIN",
            Self::Ui => "UI",
            Self::Other => "OTHER",
        }
    }

    /// Bit position for [`DebugFilter::category_mask`].
    #[must_use]
    pub fn bit(self) -> u32 {
        1u32 << (self as u32)
    }
}

/// Map a `tracing` event target (e.g. `"freejoyx::button"`) to a
/// [`EventCategory`]. Targets are typically the module path; call
/// sites can override via `target = "freejoyx::<name>"` on the macro.
/// Unknown targets fold to [`EventCategory::Other`] so legacy
/// `tracing::warn!` calls without an explicit target still surface.
#[must_use]
pub fn target_to_category(target: &str) -> EventCategory {
    // Order matters: longer prefixes first when they overlap. Today
    // there's no overlap so a flat match works.
    if target.starts_with("freejoyx::device") || target.starts_with("freejoyx_device") {
        EventCategory::Device
    } else if target.starts_with("freejoyx::hid") {
        EventCategory::Hid
    } else if target.starts_with("freejoyx::config") {
        EventCategory::Config
    } else if target.starts_with("freejoyx::params") {
        EventCategory::Params
    } else if target.starts_with("freejoyx::button") {
        EventCategory::Button
    } else if target.starts_with("freejoyx::axis") {
        EventCategory::Axis
    } else if target.starts_with("freejoyx::pin") {
        EventCategory::Pin
    } else if target.starts_with("freejoyx::ui") {
        EventCategory::Ui
    } else {
        EventCategory::Other
    }
}

/// One captured event ready for UI rendering.
#[derive(Debug, Clone)]
pub struct LogEvent {
    /// Monotonic sequence number assigned by [`LogBuffer::push`] —
    /// drives incremental UI polling without re-scanning the whole
    /// buffer.
    pub seq: u64,
    /// Wall-clock timestamp. Slint side formats as `HH:MM:SS.mmm`.
    pub timestamp: SystemTime,
    pub level: LogLevel,
    pub category: EventCategory,
    /// The `tracing` event's `message` field, joined with any other
    /// fields whose names start with `_` (free-form text). Structured
    /// fields are kept separately in [`Self::fields`].
    pub message: String,
    /// Key/value pairs collected from the tracing event. Rendered
    /// trailing the message as `[k=v k=v …]`.
    pub fields: Vec<(String, String)>,
}

impl LogEvent {
    /// Render the fields as a single trailing snippet for the UI. Kept
    /// here rather than in Slint so the formatting logic lives next
    /// to the data shape.
    #[must_use]
    pub fn fields_summary(&self) -> String {
        if self.fields.is_empty() {
            return String::new();
        }
        let mut out = String::with_capacity(self.fields.len() * 16);
        out.push('[');
        let mut first = true;
        for (k, v) in &self.fields {
            if !first {
                out.push(' ');
            }
            first = false;
            let _ = write!(&mut out, "{k}={v}");
        }
        out.push(']');
        out
    }
}

/// Live filter state. Shared between the [`BufferLayer`] and the
/// Debug-tab UI; the layer reads on every event to decide whether to
/// allocate, the UI writes when the user adjusts controls.
#[derive(Debug, Clone)]
pub struct DebugFilter {
    /// Minimum level to admit. Events below this are dropped early.
    pub min_level: LogLevel,
    /// Bitset of admitted categories — bit position is `category.bit()`.
    pub category_mask: u32,
}

impl DebugFilter {
    /// Sensible default: Info+ events, every category except Params
    /// (per-tick noise) and Ui (every tab switch). Mirrors the v0.1
    /// scope decisions in the planning doc.
    #[must_use]
    pub fn default_filter() -> Self {
        let mut mask: u32 = 0;
        for cat in EventCategory::ALL {
            if matches!(cat, EventCategory::Params | EventCategory::Ui) {
                continue;
            }
            mask |= cat.bit();
        }
        Self {
            min_level: LogLevel::Info,
            category_mask: mask,
        }
    }

    #[must_use]
    pub fn admits(&self, level: LogLevel, category: EventCategory) -> bool {
        level >= self.min_level && (self.category_mask & category.bit()) != 0
    }

    pub fn set_category(&mut self, category: EventCategory, on: bool) {
        if on {
            self.category_mask |= category.bit();
        } else {
            self.category_mask &= !category.bit();
        }
    }

    #[must_use]
    pub fn has_category(&self, category: EventCategory) -> bool {
        self.category_mask & category.bit() != 0
    }
}

impl Default for DebugFilter {
    fn default() -> Self {
        Self::default_filter()
    }
}

/// Shared bounded ring of recent events. Cheap clone — wraps an
/// `Arc<Mutex<…>>`.
#[derive(Debug, Clone, Default)]
pub struct LogBuffer {
    inner: Arc<Mutex<BufferInner>>,
}

#[derive(Debug)]
struct BufferInner {
    events: VecDeque<LogEvent>,
    /// First-issued sequence is 1 so the UI can pass `after_seq = 0`
    /// as "haven't seen anything yet" and pull every event on the
    /// initial drain. Wraps on `u64::MAX` overflow, which would take
    /// ~5 billion years at 100 Hz — not a real concern.
    next_seq: u64,
}

impl Default for BufferInner {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            next_seq: 1,
        }
    }
}

impl LogBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push an event. Drops the oldest entry when the buffer hits
    /// [`BUFFER_CAPACITY`].
    pub fn push(&self, mut event: LogEvent) {
        let Ok(mut g) = self.inner.lock() else {
            return;
        };
        event.seq = g.next_seq;
        g.next_seq = g.next_seq.wrapping_add(1);
        if g.events.len() == BUFFER_CAPACITY {
            g.events.pop_front();
        }
        g.events.push_back(event);
    }

    /// Return every event with `seq > after_seq`. Returns the new
    /// max-seq as the second element so the caller can update its
    /// cursor and skip already-seen entries next tick.
    #[must_use]
    pub fn drain_new(&self, after_seq: u64) -> (Vec<LogEvent>, u64) {
        let Ok(g) = self.inner.lock() else {
            return (Vec::new(), after_seq);
        };
        let mut max_seen = after_seq;
        let mut out = Vec::new();
        for ev in &g.events {
            if ev.seq > after_seq {
                if ev.seq > max_seen {
                    max_seen = ev.seq;
                }
                out.push(ev.clone());
            }
        }
        (out, max_seen)
    }

    /// Return a copy of every event currently in the buffer. Used by
    /// the "Export" button.
    #[must_use]
    pub fn snapshot(&self) -> Vec<LogEvent> {
        let Ok(g) = self.inner.lock() else {
            return Vec::new();
        };
        g.events.iter().cloned().collect()
    }

    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.events.clear();
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.events.len()).unwrap_or(0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Shared handle to a [`DebugFilter`]. Cloning copies the `Arc`.
#[derive(Debug, Clone, Default)]
pub struct DebugFilterHandle {
    inner: Arc<Mutex<DebugFilter>>,
}

impl DebugFilterHandle {
    #[must_use]
    pub fn new(filter: DebugFilter) -> Self {
        Self {
            inner: Arc::new(Mutex::new(filter)),
        }
    }

    /// Snapshot the current filter. Returns the default if the mutex
    /// is poisoned (UI keeps rendering rather than panicking).
    #[must_use]
    pub fn snapshot(&self) -> DebugFilter {
        self.inner.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn update(&self, f: impl FnOnce(&mut DebugFilter)) {
        if let Ok(mut g) = self.inner.lock() {
            f(&mut g);
        }
    }

    #[must_use]
    pub fn admits(&self, level: LogLevel, category: EventCategory) -> bool {
        self.inner
            .lock()
            .map(|g| g.admits(level, category))
            .unwrap_or(false)
    }
}

/// `tracing-subscriber` Layer that lifts events into the shared
/// [`LogBuffer`]. The Layer is `Send + Sync` so it can be installed
/// alongside the existing stderr + rolling-file layers.
pub struct BufferLayer {
    buffer: LogBuffer,
    filter: DebugFilterHandle,
}

impl BufferLayer {
    #[must_use]
    pub fn new(buffer: LogBuffer, filter: DebugFilterHandle) -> Self {
        Self { buffer, filter }
    }
}

impl<S: Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = LogLevel::from_tracing(metadata.level());
        let category = target_to_category(metadata.target());
        if !self.filter.admits(level, category) {
            return;
        }

        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);

        let log_event = LogEvent {
            seq: 0, // filled in on push
            timestamp: SystemTime::now(),
            level,
            category,
            message: visitor.message,
            fields: visitor.fields,
        };
        self.buffer.push(log_event);
    }
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let name = field.name();
        if name == "message" {
            let _ = write!(&mut self.message, "{value:?}");
        } else {
            self.fields.push((name.to_string(), format!("{value:?}")));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        let name = field.name();
        if name == "message" {
            self.message.push_str(value);
        } else {
            self.fields.push((name.to_string(), value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }
}

/// Format `SystemTime` as `HH:MM:SS.mmm` for the UI. Falls back to the
/// raw seconds-since-epoch if the system clock is before UNIX_EPOCH
/// (which shouldn't happen but we don't want to panic on it).
#[must_use]
pub fn format_timestamp(t: SystemTime) -> String {
    let Ok(d) = t.duration_since(UNIX_EPOCH) else {
        return "????????.???".to_string();
    };
    let total_secs = d.as_secs();
    let millis = d.subsec_millis();
    let hours = (total_secs / 3600) % 24;
    let minutes = (total_secs / 60) % 60;
    let seconds = total_secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_drops_oldest_at_capacity() {
        let buf = LogBuffer::new();
        for i in 0..(BUFFER_CAPACITY + 10) {
            buf.push(LogEvent {
                seq: 0,
                timestamp: SystemTime::now(),
                level: LogLevel::Info,
                category: EventCategory::Other,
                message: format!("event {i}"),
                fields: Vec::new(),
            });
        }
        assert_eq!(buf.len(), BUFFER_CAPACITY);
    }

    #[test]
    fn drain_new_returns_only_unseen() {
        let buf = LogBuffer::new();
        for i in 0..5 {
            buf.push(LogEvent {
                seq: 0,
                timestamp: SystemTime::now(),
                level: LogLevel::Info,
                category: EventCategory::Other,
                message: format!("event {i}"),
                fields: Vec::new(),
            });
        }
        let (first, cursor) = buf.drain_new(0);
        assert_eq!(first.len(), 5);
        assert_eq!(cursor, 5); // seqs are 1..=5, so max-seen is 5
        let (second, cursor2) = buf.drain_new(cursor);
        assert!(second.is_empty());
        assert_eq!(cursor2, cursor);
    }

    #[test]
    fn filter_default_excludes_params_and_ui() {
        let f = DebugFilter::default_filter();
        assert!(!f.admits(LogLevel::Info, EventCategory::Params));
        assert!(!f.admits(LogLevel::Info, EventCategory::Ui));
        assert!(f.admits(LogLevel::Info, EventCategory::Device));
        assert!(f.admits(LogLevel::Info, EventCategory::Button));
    }

    #[test]
    fn filter_min_level_drops_below() {
        let f = DebugFilter::default_filter();
        assert!(!f.admits(LogLevel::Debug, EventCategory::Device));
        assert!(f.admits(LogLevel::Info, EventCategory::Device));
        assert!(f.admits(LogLevel::Error, EventCategory::Device));
    }

    #[test]
    fn target_categorisation() {
        assert_eq!(
            target_to_category("freejoyx::button"),
            EventCategory::Button
        );
        assert_eq!(
            target_to_category("freejoyx::button::logical"),
            EventCategory::Button
        );
        assert_eq!(
            target_to_category("freejoyx_device::worker"),
            EventCategory::Device
        );
        assert_eq!(
            target_to_category("random_other_crate"),
            EventCategory::Other
        );
    }

    #[test]
    fn timestamp_renders_hms_millis() {
        // 12:34:56.789 since epoch
        let t = UNIX_EPOCH
            + std::time::Duration::from_secs(12 * 3600 + 34 * 60 + 56)
            + std::time::Duration::from_millis(789);
        assert_eq!(format_timestamp(t), "12:34:56.789");
    }
}
