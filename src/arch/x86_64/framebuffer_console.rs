//! Early linear-framebuffer diagnostics for x86_64 Limine boots.
//!
//! This is a boot-console mechanism only.  The supervised display stack still
//! owns long-lived graphics policy once `displayd` and driver services exist.

use core::fmt::{self, Write};

use mirage_fb::{BootFramebuffer, FramebufferMode, PixelFormat, PixelMasks};

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
    InvalidMask,
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
        let boot_framebuffer = boot_framebuffer_from_info(info)?;
        let base = boot_framebuffer.physical_address as *mut u8;
        if base.is_null() {
            return Err(FramebufferConsoleError::AddressUnavailable);
        }

        let mode = boot_framebuffer
            .mode()
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

/// Convert typed x86_64 boot framebuffer metadata into the shared Mirage
/// framebuffer descriptor used by the first hardware framebuffer writer.
pub fn boot_framebuffer_from_info(
    info: FramebufferInfo,
) -> Result<BootFramebuffer, FramebufferConsoleError> {
    if info.bits_per_pixel != 32 {
        return Err(FramebufferConsoleError::InvalidMode(
            mirage_fb::FramebufferError::InvalidBitsPerPixel,
        ));
    }

    let width =
        usize::try_from(info.width).map_err(|_| FramebufferConsoleError::DimensionOverflow)?;
    let height =
        usize::try_from(info.height).map_err(|_| FramebufferConsoleError::DimensionOverflow)?;
    let pitch =
        usize::try_from(info.pitch).map_err(|_| FramebufferConsoleError::DimensionOverflow)?;
    let red_mask = checked_mask(info.red_mask_size, info.red_mask_shift)?;
    let green_mask = checked_mask(info.green_mask_size, info.green_mask_shift)?;
    let blue_mask = checked_mask(info.blue_mask_size, info.blue_mask_shift)?;
    let reserved_mask = reserved_mask(red_mask, green_mask, blue_mask, info.bits_per_pixel)?;
    let pixel_format = PixelFormat::Masks(PixelMasks::new(
        red_mask,
        green_mask,
        blue_mask,
        reserved_mask,
    ));

    Ok(BootFramebuffer {
        physical_address: info.address.0,
        width,
        height,
        pitch,
        bits_per_pixel: usize::from(info.bits_per_pixel),
        pixel_format,
        red_mask,
        green_mask,
        blue_mask,
        reserved_mask,
    })
}

fn checked_mask(size: u8, shift: u8) -> Result<u32, FramebufferConsoleError> {
    if size == 0 {
        return Ok(0);
    }

    if u16::from(size) + u16::from(shift) > u32::BITS as u16 {
        return Err(FramebufferConsoleError::InvalidMask);
    }

    let width_mask = 1u32
        .checked_shl(u32::from(size))
        .and_then(|value| value.checked_sub(1))
        .ok_or(FramebufferConsoleError::InvalidMask)?;
    width_mask
        .checked_shl(u32::from(shift))
        .ok_or(FramebufferConsoleError::InvalidMask)
}

fn reserved_mask(
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
    bits_per_pixel: u16,
) -> Result<u32, FramebufferConsoleError> {
    let active_bits = if bits_per_pixel == u32::BITS as u16 {
        u32::MAX
    } else {
        1u32.checked_shl(u32::from(bits_per_pixel))
            .and_then(|value| value.checked_sub(1))
            .ok_or(FramebufferConsoleError::InvalidMask)?
    };
    Ok(active_bits & !(red_mask | green_mask | blue_mask))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::x86_64::boot::VirtualAddress;

    fn framebuffer_info() -> FramebufferInfo {
        FramebufferInfo {
            address: VirtualAddress(0x1_0000_0000),
            width: 1024,
            height: 768,
            pitch: 4096,
            bits_per_pixel: 32,
            red_mask_size: 8,
            red_mask_shift: 16,
            green_mask_size: 8,
            green_mask_shift: 8,
            blue_mask_size: 8,
            blue_mask_shift: 0,
        }
    }

    #[test]
    fn conversion_preserves_64_bit_address_and_masks() {
        let framebuffer = boot_framebuffer_from_info(framebuffer_info()).unwrap();

        assert_eq!(framebuffer.physical_address, 0x1_0000_0000);
        assert_eq!(framebuffer.red_mask, 0x00ff_0000);
        assert_eq!(framebuffer.green_mask, 0x0000_ff00);
        assert_eq!(framebuffer.blue_mask, 0x0000_00ff);
        assert_eq!(framebuffer.reserved_mask, 0xff00_0000);
        assert_eq!(
            framebuffer.pixel_format,
            PixelFormat::Masks(PixelMasks::new(
                0x00ff_0000,
                0x0000_ff00,
                0x0000_00ff,
                0xff00_0000,
            ))
        );
    }

    #[test]
    fn conversion_rejects_non_32_bpp_modes() {
        let mut info = framebuffer_info();
        info.bits_per_pixel = 24;

        assert!(matches!(
            boot_framebuffer_from_info(info),
            Err(FramebufferConsoleError::InvalidMode(
                mirage_fb::FramebufferError::InvalidBitsPerPixel
            ))
        ));
    }

    #[test]
    fn conversion_rejects_overflowing_masks() {
        let mut info = framebuffer_info();
        info.red_mask_size = 32;

        assert!(matches!(
            boot_framebuffer_from_info(info),
            Err(FramebufferConsoleError::InvalidMask)
        ));

        info.red_mask_size = 31;
        info.red_mask_shift = 2;

        assert!(matches!(
            boot_framebuffer_from_info(info),
            Err(FramebufferConsoleError::InvalidMask)
        ));
    }

    #[test]
    fn zero_sized_masks_are_empty() {
        let mut info = framebuffer_info();
        info.red_mask_size = 0;
        info.green_mask_size = 0;
        info.blue_mask_size = 0;

        let framebuffer = boot_framebuffer_from_info(info).unwrap();

        assert_eq!(framebuffer.red_mask, 0);
        assert_eq!(framebuffer.green_mask, 0);
        assert_eq!(framebuffer.blue_mask, 0);
        assert_eq!(framebuffer.reserved_mask, u32::MAX);
    }
}
