//! Early linear-framebuffer diagnostics for x86_64 Limine boots.
//!
//! This is a boot-console mechanism only.  The supervised display stack still
//! owns long-lived graphics policy once `displayd` and driver services exist.

use core::fmt::{self, Write};

use mirage_fb::{FramebufferMode, PixelFormat, PixelMasks};

use crate::arch::x86_64::boot::{BootInfo, FramebufferInfo};
use crate::kernel::sync::SpinLock;

const DEFAULT_FOREGROUND: (u8, u8, u8) = (0xff, 0xff, 0xff);
const DEFAULT_BACKGROUND: (u8, u8, u8) = (0x00, 0x00, 0x00);
const CELL_WIDTH: usize = 8;
const CELL_HEIGHT: usize = 16;

static CONSOLE: SpinLock<Option<FramebufferConsole>> = SpinLock::new(None);

/// Initialize the early framebuffer console from Limine boot information.
///
/// The caller must invoke this only after the framebuffer virtual address in
/// [`BootInfo`] is mapped in the active page tables. On Mirage's x86_64 path,
/// `init_architecture` calls this after `setup_memory_layout` installs the early
/// framebuffer mapping.
pub fn init_from_boot_info(boot_info: &BootInfo) {
    let Some(framebuffer) = boot_info.framebuffer else {
        crate::kprintln!("framebuffer unavailable; serial console only");
        return;
    };

    match FramebufferConsole::new(framebuffer) {
        Ok(mut console) => {
            console.clear(DEFAULT_BACKGROUND);
            let _ = console.write_str("GNU/Mirage framebuffer console ready\n");
            *CONSOLE.lock() = Some(console);
            crate::kprintln!(
                "framebuffer console initialized: {}x{}x{} pitch={}",
                framebuffer.width,
                framebuffer.height,
                framebuffer.bits_per_pixel,
                framebuffer.pitch
            );
        }
        Err(error) => {
            crate::kprintln!(
                "framebuffer console unavailable: {:?}; serial console only",
                error
            );
        }
    }
}

/// Write formatted text to the framebuffer console if it has been initialized.
///
/// Serial remains the authoritative early diagnostic path; this helper is for
/// callers that explicitly want best-effort framebuffer fanout.
pub fn early_print(args: fmt::Arguments<'_>) {
    if let Some(console) = CONSOLE.lock().as_mut() {
        let _ = console.write_fmt(args);
    }
}

#[derive(Debug)]
pub enum FramebufferConsoleError {
    AddressUnavailable,
    DimensionOverflow,
    InvalidMode(mirage_fb::FramebufferError),
}

struct FramebufferConsole {
    base: *mut u8,
    mode: FramebufferMode,
    cursor_column: usize,
    cursor_row: usize,
    foreground: (u8, u8, u8),
    background: (u8, u8, u8),
}

// The framebuffer console is protected by `CONSOLE`; its raw pointer refers to
// bootloader-provided MMIO memory that is valid after x86_64 paging setup maps
// the Limine framebuffer virtual address.
unsafe impl Send for FramebufferConsole {}

impl FramebufferConsole {
    fn new(info: FramebufferInfo) -> Result<Self, FramebufferConsoleError> {
        let base = info.address.0 as *mut u8;
        if base.is_null() {
            return Err(FramebufferConsoleError::AddressUnavailable);
        }

        let mode = FramebufferMode::new(
            usize::try_from(info.width).map_err(|_| FramebufferConsoleError::DimensionOverflow)?,
            usize::try_from(info.height).map_err(|_| FramebufferConsoleError::DimensionOverflow)?,
            usize::try_from(info.pitch).map_err(|_| FramebufferConsoleError::DimensionOverflow)?,
            usize::from(info.bits_per_pixel),
            pixel_format(info),
        )
        .map_err(FramebufferConsoleError::InvalidMode)?;

        Ok(Self {
            base,
            mode,
            cursor_column: 0,
            cursor_row: 0,
            foreground: DEFAULT_FOREGROUND,
            background: DEFAULT_BACKGROUND,
        })
    }

    fn clear(&mut self, color: (u8, u8, u8)) {
        for y in 0..self.mode.height() {
            for x in 0..self.mode.width() {
                self.put_pixel(x, y, color);
            }
        }
        self.cursor_column = 0;
        self.cursor_row = 0;
    }

    fn write_byte(&mut self, byte: u8) {
        if byte == b'\n' {
            self.newline();
            return;
        }

        let x = self.cursor_column * CELL_WIDTH;
        let y = self.cursor_row * CELL_HEIGHT;
        if x + CELL_WIDTH <= self.mode.width() && y + CELL_HEIGHT <= self.mode.height() {
            self.draw_rect(x, y, CELL_WIDTH, CELL_HEIGHT, self.background);
            if byte != b' ' {
                self.draw_rect(
                    x + 2,
                    y + 2,
                    CELL_WIDTH - 4,
                    CELL_HEIGHT - 4,
                    self.foreground,
                );
            }
        }

        self.advance_cursor();
    }

    fn draw_rect(&mut self, x: usize, y: usize, width: usize, height: usize, color: (u8, u8, u8)) {
        for row in y..y.saturating_add(height).min(self.mode.height()) {
            for column in x..x.saturating_add(width).min(self.mode.width()) {
                self.put_pixel(column, row, color);
            }
        }
    }

    fn put_pixel(&mut self, x: usize, y: usize, color: (u8, u8, u8)) {
        if let Ok(offset) = self.mode.pixel_offset(x, y) {
            self.write_pixel_at_offset(offset, color);
        }
    }

    fn write_pixel_at_offset(&mut self, offset: usize, color: (u8, u8, u8)) {
        let (red, green, blue) = color;
        let bytes = match self.mode.pixel_format() {
            PixelFormat::Rgb => encode_rgb(self.mode.bytes_per_pixel(), red, green, blue),
            PixelFormat::Bgr => encode_bgr(self.mode.bytes_per_pixel(), red, green, blue),
            PixelFormat::Xrgb => [0, red, green, blue],
            PixelFormat::Masks(masks) => encode_masked(masks, red, green, blue).to_le_bytes(),
        };

        let bytes_per_pixel = self.mode.bytes_per_pixel();
        for (index, byte) in bytes.iter().take(bytes_per_pixel).enumerate() {
            // MMIO framebuffer writes must be volatile so the compiler does not
            // elide visible diagnostics during early boot.
            unsafe { self.base.add(offset + index).write_volatile(*byte) };
        }
    }

    fn advance_cursor(&mut self) {
        self.cursor_column += 1;
        if self.cursor_column >= self.columns() {
            self.newline();
        }
    }

    fn newline(&mut self) {
        self.cursor_column = 0;
        self.cursor_row += 1;
        if self.cursor_row >= self.rows() {
            self.cursor_row = 0;
        }
    }

    fn columns(&self) -> usize {
        (self.mode.width() / CELL_WIDTH).max(1)
    }

    fn rows(&self) -> usize {
        (self.mode.height() / CELL_HEIGHT).max(1)
    }
}

impl Write for FramebufferConsole {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            self.write_byte(byte);
        }
        Ok(())
    }
}

fn pixel_format(info: FramebufferInfo) -> PixelFormat {
    PixelFormat::Masks(PixelMasks::new(
        mask(info.red_mask_size, info.red_mask_shift),
        mask(info.green_mask_size, info.green_mask_shift),
        mask(info.blue_mask_size, info.blue_mask_shift),
        reserved_mask(info),
    ))
}

fn mask(size: u8, shift: u8) -> u32 {
    if size == 0 || shift >= u32::BITS as u8 {
        return 0;
    }

    let width_mask = if size >= u32::BITS as u8 {
        u32::MAX
    } else {
        (1u32 << size) - 1
    };
    width_mask.checked_shl(u32::from(shift)).unwrap_or(0)
}

fn reserved_mask(info: FramebufferInfo) -> u32 {
    let used = mask(info.red_mask_size, info.red_mask_shift)
        | mask(info.green_mask_size, info.green_mask_shift)
        | mask(info.blue_mask_size, info.blue_mask_shift);
    let active_bits = if info.bits_per_pixel >= u32::BITS as u16 {
        u32::MAX
    } else {
        (1u32 << info.bits_per_pixel) - 1
    };
    active_bits & !used
}

fn encode_rgb(bytes_per_pixel: usize, red: u8, green: u8, blue: u8) -> [u8; 4] {
    let mut bytes = [red, green, blue, 0];
    if bytes_per_pixel < 4 {
        bytes[3] = 0;
    }
    bytes
}

fn encode_bgr(bytes_per_pixel: usize, red: u8, green: u8, blue: u8) -> [u8; 4] {
    let mut bytes = [blue, green, red, 0];
    if bytes_per_pixel < 4 {
        bytes[3] = 0;
    }
    bytes
}

fn encode_masked(masks: PixelMasks, red: u8, green: u8, blue: u8) -> u32 {
    encode_channel(masks.red, red)
        | encode_channel(masks.green, green)
        | encode_channel(masks.blue, blue)
}

fn encode_channel(mask: u32, value: u8) -> u32 {
    if mask == 0 {
        return 0;
    }

    let shift = mask.trailing_zeros();
    let width = mask.count_ones();
    let max = if width >= u32::BITS {
        u32::MAX
    } else {
        (1u32 << width) - 1
    };
    let scaled = (u32::from(value) * max + 127) / 255;
    (scaled << shift) & mask
}
