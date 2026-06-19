//! No-heap boot diagnostics, breadcrumbs, and persistent failure reporting.
//!
//! This module is intentionally small and allocation-free so it can be used
//! from early boot, panic, and fault paths.  It records concise static strings
//! plus machine registers and mirrors important evidence to serial while the
//! framebuffer boot UI remains a separate renderer.

#[cfg(feature = "hw-framebuffer")]
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::kernel::boot_phase::{boot_phase_current, PhaseState};
use crate::kernel::sync::SpinLock;

pub const BOOT_LOG_CAPACITY: usize = 96;
pub const BOOT_LOG_MESSAGE_BYTES: usize = 96;
pub const DEFAULT_FREEZE_ON_FAIL: bool = true;
pub const DEFAULT_NO_FB_CLEAR_AFTER_BOOT: bool = true;
pub const DEFAULT_RAW_HW_DUMP: bool = cfg!(feature = "bootdiag-raw-hw");
pub const DEFAULT_SERIAL_DIAGNOSTICS: bool =
    cfg!(feature = "bootdiag-serial") || cfg!(feature = "bootdiag-verbose");
pub const DEFAULT_FB_LOG_OVERLAY: bool = cfg!(feature = "bootdiag-framebuffer");

static FRAMEBUFFER_ONLINE: AtomicBool = AtomicBool::new(false);
static FAILURE_SCREEN_DRAWN: AtomicBool = AtomicBool::new(false);
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
static EVENTS_CAPTURED: AtomicU64 = AtomicU64::new(0);
static EVENTS_IGNORED: AtomicU64 = AtomicU64::new(0);
static SERIAL_WRITES: AtomicU64 = AtomicU64::new(0);
static RAW_DUMPS_SUPPRESSED: AtomicU64 = AtomicU64::new(0);
static DIAGNOSTICS: SpinLock<BootDiagnostics> = SpinLock::new(BootDiagnostics::new());
static BOOT_LOG: SpinLock<BootLogRing> = SpinLock::new(BootLogRing::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootLogLevel {
    Info,
    Warn,
    Error,
    Fault,
}

impl BootLogLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
            Self::Fault => "FAULT",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootLogEntry {
    pub sequence: u64,
    pub tick: u64,
    pub phase: &'static str,
    pub level: BootLogLevel,
    pub message: [u8; BOOT_LOG_MESSAGE_BYTES],
    pub message_len: u8,
    pub address: u64,
    pub code: u64,
    pub status: u64,
}

impl BootLogEntry {
    pub const fn empty() -> Self {
        Self {
            sequence: 0,
            tick: 0,
            phase: "",
            level: BootLogLevel::Info,
            message: [0; BOOT_LOG_MESSAGE_BYTES],
            message_len: 0,
            address: 0,
            code: 0,
            status: 0,
        }
    }

    fn new(level: BootLogLevel, phase: &'static str, message: &'static str) -> Self {
        let mut entry = Self::empty();
        entry.sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst);
        entry.tick = entry.sequence;
        entry.phase = phase;
        entry.level = level;
        copy_message(&mut entry.message, &mut entry.message_len, message);
        entry
    }

    pub fn message(&self) -> &str {
        core::str::from_utf8(&self.message[..self.message_len as usize]).unwrap_or("<invalid>")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootLogRing {
    entries: [BootLogEntry; BOOT_LOG_CAPACITY],
    next: usize,
    len: usize,
    overwritten: u64,
}

impl BootLogRing {
    pub const fn new() -> Self {
        Self {
            entries: [BootLogEntry::empty(); BOOT_LOG_CAPACITY],
            next: 0,
            len: 0,
            overwritten: 0,
        }
    }

    fn push(&mut self, entry: BootLogEntry) {
        if self.len == BOOT_LOG_CAPACITY {
            self.overwritten = self.overwritten.saturating_add(1);
        } else {
            self.len += 1;
        }
        self.entries[self.next] = entry;
        self.next = (self.next + 1) % BOOT_LOG_CAPACITY;
    }

    pub const fn overwritten(&self) -> u64 {
        self.overwritten
    }

    pub fn for_each_recent(&self, max: usize, mut visit: impl FnMut(&BootLogEntry)) {
        let count = core::cmp::min(max, self.len);
        let start = (self.next + BOOT_LOG_CAPACITY - count) % BOOT_LOG_CAPACITY;
        let mut index = 0usize;
        while index < count {
            visit(&self.entries[(start + index) % BOOT_LOG_CAPACITY]);
            index += 1;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootDiagnostics {
    pub last_phase_name: &'static str,
    pub last_phase_state: PhaseState,
    pub last_substep_id: &'static str,
    pub last_substep_message: &'static str,
    pub last_source_file: &'static str,
    pub last_source_line: u32,
    pub last_fault_vector: Option<u8>,
    pub last_fault_error_code: u64,
    pub last_rip: u64,
    pub last_rsp: u64,
    pub last_rflags: u64,
    pub last_cr2: u64,
    pub last_panic_message: &'static str,
    pub monotonic_tick: u64,
    pub failure_reason: &'static str,
}

impl BootDiagnostics {
    pub const fn new() -> Self {
        Self {
            last_phase_name: "Seed-rs",
            last_phase_state: PhaseState::Pending,
            last_substep_id: "boot 00",
            last_substep_message: "diagnostics initialized",
            last_source_file: "<early>",
            last_source_line: 0,
            last_fault_vector: None,
            last_fault_error_code: 0,
            last_rip: 0,
            last_rsp: 0,
            last_rflags: 0,
            last_cr2: 0,
            last_panic_message: "",
            monotonic_tick: 0,
            failure_reason: "",
        }
    }
}

fn copy_message(dst: &mut [u8; BOOT_LOG_MESSAGE_BYTES], len: &mut u8, message: &'static str) {
    let bytes = message.as_bytes();
    let mut index = 0usize;
    let max = core::cmp::min(bytes.len(), BOOT_LOG_MESSAGE_BYTES);
    while index < max {
        dst[index] = bytes[index];
        index += 1;
    }
    *len = max as u8;
}

pub fn mark_framebuffer_online() {
    FRAMEBUFFER_ONLINE.store(true, Ordering::SeqCst);
    if !cfg!(feature = "bootdiag") {
        return;
    }
    log(
        BootLogLevel::Info,
        "Framebuffer",
        "Framebuffer [ONLINE]; later clears are gated",
    );
}

pub fn framebuffer_online() -> bool {
    FRAMEBUFFER_ONLINE.load(Ordering::SeqCst)
}

pub const fn no_fb_clear_after_boot() -> bool {
    DEFAULT_NO_FB_CLEAR_AFTER_BOOT
}

pub const fn raw_hw_dump_enabled() -> bool {
    DEFAULT_RAW_HW_DUMP || option_env!("MIRAGE_DEBUG_RAW_HW_DUMP").is_some()
}

pub fn note_raw_dump_suppressed() {
    RAW_DUMPS_SUPPRESSED.fetch_add(1, Ordering::Relaxed);
}

pub const fn debug_pci_enabled() -> bool {
    cfg!(feature = "bootdiag-raw-hw") || option_env!("MIRAGE_DEBUG_PCI").is_some()
}

pub const fn debug_ryzen_enabled() -> bool {
    cfg!(feature = "bootdiag-raw-hw") || option_env!("MIRAGE_DEBUG_RYZEN").is_some()
}

pub fn log(level: BootLogLevel, phase: &'static str, message: &'static str) {
    if !cfg!(feature = "bootdiag") && !matches!(level, BootLogLevel::Error | BootLogLevel::Fault) {
        EVENTS_IGNORED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    EVENTS_CAPTURED.fetch_add(1, Ordering::Relaxed);
    let entry = BootLogEntry::new(level, phase, message);
    BOOT_LOG.lock().push(entry);
    if DEFAULT_SERIAL_DIAGNOSTICS || matches!(level, BootLogLevel::Error | BootLogLevel::Fault) {
        SERIAL_WRITES.fetch_add(1, Ordering::Relaxed);
        crate::arch::x86_64::early_console::panic_write_fmt(format_args!(
            "[bootdiag {:06}] {} {}: {}\n",
            entry.sequence,
            level.as_str(),
            phase,
            message
        ));
    }
}

pub fn boot_trace_phase_started(name: &'static str) {
    if !cfg!(feature = "bootdiag") {
        let _ = name;
        EVENTS_IGNORED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    trace_phase(
        name,
        PhaseState::Started,
        "phase started",
        BootLogLevel::Info,
        file!(),
        line!(),
    );
}

pub fn boot_trace_phase_ok(name: &'static str) {
    if !cfg!(feature = "bootdiag") {
        let _ = name;
        EVENTS_IGNORED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    trace_phase(
        name,
        PhaseState::Ok,
        "phase ok",
        BootLogLevel::Info,
        file!(),
        line!(),
    );
}

pub fn boot_trace_phase_failed(name: &'static str, reason: &'static str) {
    trace_phase(
        name,
        PhaseState::Failed,
        reason,
        BootLogLevel::Error,
        file!(),
        line!(),
    );
}

pub fn boot_trace_substep(id: &'static str, message: &'static str) {
    boot_trace_substep_at(id, message, file!(), line!());
}

pub fn boot_trace_substep_at(
    id: &'static str,
    message: &'static str,
    source_file: &'static str,
    source_line: u32,
) {
    if !cfg!(feature = "bootdiag") {
        let _ = (id, message, source_file, source_line);
        EVENTS_IGNORED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let mut diag = DIAGNOSTICS.lock();
    diag.last_substep_id = id;
    diag.last_substep_message = message;
    diag.last_source_file = source_file;
    diag.last_source_line = source_line;
    diag.monotonic_tick = SEQUENCE.load(Ordering::SeqCst);
    drop(diag);
    log(BootLogLevel::Info, id, message);
}

pub fn boot_trace_fault(vector: u8, error_code: u64, rip: u64, rsp: u64, cr2: u64) {
    let mut diag = DIAGNOSTICS.lock();
    diag.last_fault_vector = Some(vector);
    diag.last_fault_error_code = error_code;
    diag.last_rip = rip;
    diag.last_rsp = rsp;
    diag.last_cr2 = cr2;
    diag.failure_reason = "CPU fault";
    diag.monotonic_tick = SEQUENCE.load(Ordering::SeqCst);
    drop(diag);
    log(BootLogLevel::Fault, "fault", "CPU fault captured");
}

pub fn boot_trace_panic(message: &'static str) {
    let mut diag = DIAGNOSTICS.lock();
    diag.last_panic_message = message;
    diag.failure_reason = message;
    diag.monotonic_tick = SEQUENCE.load(Ordering::SeqCst);
    drop(diag);
    log(BootLogLevel::Fault, "panic", message);
}

pub fn boot_failure(reason: &'static str) -> ! {
    boot_trace_phase_failed(boot_phase_current().name(), reason);
    draw_failure_screen(reason, crate::kernel::input::any_keyboard_online());
    crate::arch::x86_64::panic_halt()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootDiagCounters {
    pub events_captured: u64,
    pub events_ignored: u64,
    pub framebuffer_renders: u64,
    pub serial_writes: u64,
    pub raw_dumps_suppressed: u64,
    pub ring_entries_dropped: u64,
}

pub fn counters() -> BootDiagCounters {
    BootDiagCounters {
        events_captured: EVENTS_CAPTURED.load(Ordering::Relaxed),
        events_ignored: EVENTS_IGNORED.load(Ordering::Relaxed),
        framebuffer_renders: crate::kernel::boot_screen::framebuffer_render_count(),
        serial_writes: SERIAL_WRITES.load(Ordering::Relaxed),
        raw_dumps_suppressed: RAW_DUMPS_SUPPRESSED.load(Ordering::Relaxed),
        ring_entries_dropped: BOOT_LOG.lock().overwritten(),
    }
}

pub fn snapshot() -> BootDiagnostics {
    *DIAGNOSTICS.lock()
}

pub fn dump_log_to_serial(max: usize) {
    let ring = *BOOT_LOG.lock();
    crate::arch::x86_64::early_console::panic_write_fmt(format_args!(
        "\nMirage boot log (last {}, overwritten={}):\n",
        max,
        ring.overwritten()
    ));
    ring.for_each_recent(max, |entry| {
        crate::arch::x86_64::early_console::panic_write_fmt(format_args!(
            "#{:06} [{}] {}: {}\n",
            entry.sequence,
            entry.level.as_str(),
            entry.phase,
            entry.message()
        ));
    });
}

pub fn draw_failure_screen(reason: &'static str, _keyboard_available: bool) {
    if FAILURE_SCREEN_DRAWN.swap(true, Ordering::SeqCst) {
        dump_log_to_serial(32);
        return;
    }

    {
        let mut diag = DIAGNOSTICS.lock();
        diag.failure_reason = reason;
        diag.monotonic_tick = SEQUENCE.load(Ordering::SeqCst);
    }

    dump_log_to_serial(64);

    #[cfg(feature = "hw-framebuffer")]
    {
        use crate::arch::x86_64::framebuffer_console::{self, BootRenderMode, RgbColor};
        framebuffer_console::set_render_mode(BootRenderMode::FailureScreen);
        framebuffer_console::write_colored("\n\nMIRAGE BOOT FAILURE\n", RgbColor::RED);
        let diag = snapshot();
        let _ = FailureWriter(RgbColor::WHITE).write_fmt(format_args!("reason: {}\n", reason));
        let _ = FailureWriter(RgbColor::YELLOW).write_fmt(format_args!(
            "current phase: {}\nlast phase: {} [{}]\nlast substep: {} - {}\nsource: {}:{}\n",
            boot_phase_current().friendly_name(),
            diag.last_phase_name,
            diag.last_phase_state.as_str(),
            diag.last_substep_id,
            diag.last_substep_message,
            diag.last_source_file,
            diag.last_source_line
        ));
        if let Some(vector) = diag.last_fault_vector {
            let _ = FailureWriter(RgbColor::RED).write_fmt(format_args!(
                "fault vector: {} error={:#x} rip={:#x} rsp={:#x} rflags={:#x} cr2={:#x}\n",
                vector,
                diag.last_fault_error_code,
                diag.last_rip,
                diag.last_rsp,
                diag.last_rflags,
                diag.last_cr2
            ));
        }
        if !diag.last_panic_message.is_empty() {
            let _ = FailureWriter(RgbColor::RED)
                .write_fmt(format_args!("panic: {}\n", diag.last_panic_message));
        }
        framebuffer_console::write_colored("\nLast boot log entries:\n", RgbColor::WHITE);
        let ring = *BOOT_LOG.lock();
        ring.for_each_recent(20, |entry| {
            let color = match entry.level {
                BootLogLevel::Info => RgbColor::GRAY,
                BootLogLevel::Warn => RgbColor::YELLOW,
                BootLogLevel::Error | BootLogLevel::Fault => RgbColor::RED,
            };
            let _ = FailureWriter(color).write_fmt(format_args!(
                "#{:06} [{}] {}: {}\n",
                entry.sequence,
                entry.level.as_str(),
                entry.phase,
                entry.message()
            ));
        });
        if _keyboard_available {
            framebuffer_console::write_colored("\nPress ESC for debug shell\n", RgbColor::CYAN);
        } else {
            framebuffer_console::write_colored("\nKeyboard unavailable\n", RgbColor::YELLOW);
        }
    }
}

fn trace_phase(
    name: &'static str,
    state: PhaseState,
    message: &'static str,
    level: BootLogLevel,
    source_file: &'static str,
    source_line: u32,
) {
    let mut diag = DIAGNOSTICS.lock();
    diag.last_phase_name = name;
    diag.last_phase_state = state;
    diag.last_source_file = source_file;
    diag.last_source_line = source_line;
    diag.monotonic_tick = SEQUENCE.load(Ordering::SeqCst);
    if state == PhaseState::Failed {
        diag.failure_reason = message;
    }
    drop(diag);
    log(level, name, message);
}

#[cfg(feature = "hw-framebuffer")]
struct FailureWriter(crate::arch::x86_64::framebuffer_console::RgbColor);

#[cfg(feature = "hw-framebuffer")]
impl Write for FailureWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        crate::arch::x86_64::framebuffer_console::write_colored(text, self.0);
        Ok(())
    }
}

#[macro_export]
macro_rules! boot_trace_substep {
    ($id:expr, $message:expr) => {{
        #[cfg(feature = "bootdiag")]
        {
            $crate::kernel::boot_diagnostics::boot_trace_substep_at(
                $id,
                $message,
                file!(),
                line!(),
            );
        }
        #[cfg(not(feature = "bootdiag"))]
        {
            let _ = ($id, $message);
        }
    }};
}
