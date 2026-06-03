#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

/// Pixel layouts supported by the Mirage framebuffer abstraction.
///
/// The byte order is the order stored in linear framebuffer memory. For `Xrgb`,
/// the unused byte is written as zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PixelFormat {
    /// Red, green, blue byte order. Supported at 24 or 32 bits per pixel.
    Rgb,
    /// Blue, green, red byte order. Supported at 24 or 32 bits per pixel.
    Bgr,
    /// Unused byte, red, green, blue byte order. Supported at 32 bits per pixel.
    Xrgb,
}

impl PixelFormat {
    pub const fn supports_bits_per_pixel(self, bits_per_pixel: usize) -> bool {
        match self {
            Self::Rgb | Self::Bgr => bits_per_pixel == 24 || bits_per_pixel == 32,
            Self::Xrgb => bits_per_pixel == 32,
        }
    }
}

/// A validated linear framebuffer mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct FramebufferMode {
    width: usize,
    height: usize,
    pitch: usize,
    bits_per_pixel: usize,
    pixel_format: PixelFormat,
}

impl FramebufferMode {
    pub const fn new(
        width: usize,
        height: usize,
        pitch: usize,
        bits_per_pixel: usize,
        pixel_format: PixelFormat,
    ) -> Result<Self, FramebufferError> {
        let mode = Self {
            width,
            height,
            pitch,
            bits_per_pixel,
            pixel_format,
        };
        match mode.validate() {
            Ok(()) => Ok(mode),
            Err(error) => Err(error),
        }
    }

    pub const fn validate(self) -> Result<(), FramebufferError> {
        if self.width == 0 || self.height == 0 {
            return Err(FramebufferError::InvalidDimensions);
        }

        if !self
            .pixel_format
            .supports_bits_per_pixel(self.bits_per_pixel)
        {
            return Err(FramebufferError::InvalidBitsPerPixel);
        }

        if self.bits_per_pixel % 8 != 0 {
            return Err(FramebufferError::InvalidBitsPerPixel);
        }

        let bytes_per_pixel = self.bits_per_pixel / 8;
        let minimum_pitch = match self.width.checked_mul(bytes_per_pixel) {
            Some(value) => value,
            None => return Err(FramebufferError::SizeOverflow),
        };

        if self.pitch < minimum_pitch {
            return Err(FramebufferError::InvalidPitch);
        }

        if self.pitch.checked_mul(self.height).is_none() {
            return Err(FramebufferError::SizeOverflow);
        }

        Ok(())
    }

    pub const fn width(self) -> usize {
        self.width
    }

    pub const fn height(self) -> usize {
        self.height
    }

    pub const fn pitch(self) -> usize {
        self.pitch
    }

    pub const fn stride(self) -> usize {
        self.pitch
    }

    pub const fn bits_per_pixel(self) -> usize {
        self.bits_per_pixel
    }

    pub const fn bytes_per_pixel(self) -> usize {
        self.bits_per_pixel / 8
    }

    pub const fn pixel_format(self) -> PixelFormat {
        self.pixel_format
    }

    pub const fn framebuffer_len(self) -> Result<usize, FramebufferError> {
        match self.pitch.checked_mul(self.height) {
            Some(value) => Ok(value),
            None => Err(FramebufferError::SizeOverflow),
        }
    }

    pub const fn pixel_offset(self, x: usize, y: usize) -> Result<usize, FramebufferError> {
        if x >= self.width || y >= self.height {
            return Err(FramebufferError::OutOfBounds);
        }

        let row = match y.checked_mul(self.pitch) {
            Some(value) => value,
            None => return Err(FramebufferError::SizeOverflow),
        };
        let column = match x.checked_mul(self.bytes_per_pixel()) {
            Some(value) => value,
            None => return Err(FramebufferError::SizeOverflow),
        };

        match row.checked_add(column) {
            Some(value) => Ok(value),
            None => Err(FramebufferError::SizeOverflow),
        }
    }
}

/// Description of a framebuffer allocation exposed to display services.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct FramebufferInfo {
    mode: FramebufferMode,
    byte_len: usize,
}

impl FramebufferInfo {
    pub const fn new(mode: FramebufferMode, byte_len: usize) -> Result<Self, FramebufferError> {
        let required_len = match mode.framebuffer_len() {
            Ok(value) => value,
            Err(error) => return Err(error),
        };
        if byte_len < required_len {
            return Err(FramebufferError::BufferTooSmall);
        }

        Ok(Self { mode, byte_len })
    }

    pub const fn mode(self) -> FramebufferMode {
        self.mode
    }

    pub const fn byte_len(self) -> usize {
        self.byte_len
    }
}

/// Errors reported by framebuffer mode validation and drawing operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FramebufferError {
    InvalidDimensions,
    InvalidBitsPerPixel,
    InvalidPitch,
    SizeOverflow,
    BufferTooSmall,
    OutOfBounds,
}

/// Linear framebuffer backed by mock memory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Framebuffer {
    mode: FramebufferMode,
    memory: Vec<u8>,
}

impl Framebuffer {
    pub fn new(mode: FramebufferMode) -> Result<Self, FramebufferError> {
        let len = mode.framebuffer_len()?;
        Ok(Self {
            mode,
            memory: vec![0; len],
        })
    }

    pub fn from_memory(mode: FramebufferMode, memory: Vec<u8>) -> Result<Self, FramebufferError> {
        FramebufferInfo::new(mode, memory.len())?;
        Ok(Self { mode, memory })
    }

    pub const fn mode(&self) -> FramebufferMode {
        self.mode
    }

    pub const fn info(&self) -> FramebufferInfo {
        FramebufferInfo {
            mode: self.mode,
            byte_len: self.memory.len(),
        }
    }

    pub fn memory(&self) -> &[u8] {
        &self.memory
    }

    pub fn memory_mut(&mut self) -> &mut [u8] {
        &mut self.memory
    }

    pub fn pixel_offset(&self, x: usize, y: usize) -> Result<usize, FramebufferError> {
        self.mode.pixel_offset(x, y)
    }

    pub fn clear(&mut self, color: (u8, u8, u8)) {
        for y in 0..self.mode.height() {
            for x in 0..self.mode.width() {
                // Coordinates are generated from the validated mode, so they cannot fail.
                if let Ok(offset) = self.mode.pixel_offset(x, y) {
                    self.write_pixel_at_offset(offset, color);
                }
            }
        }
    }

    pub fn put_pixel(
        &mut self,
        x: usize,
        y: usize,
        color: (u8, u8, u8),
    ) -> Result<(), FramebufferError> {
        let offset = self.mode.pixel_offset(x, y)?;
        self.write_pixel_at_offset(offset, color);
        Ok(())
    }

    pub fn draw_rectangle(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        color: (u8, u8, u8),
    ) -> Result<(), FramebufferError> {
        if width == 0 || height == 0 {
            return Ok(());
        }

        let end_x = x.checked_add(width).ok_or(FramebufferError::SizeOverflow)?;
        let end_y = y
            .checked_add(height)
            .ok_or(FramebufferError::SizeOverflow)?;
        if x >= self.mode.width()
            || y >= self.mode.height()
            || end_x > self.mode.width()
            || end_y > self.mode.height()
        {
            return Err(FramebufferError::OutOfBounds);
        }

        for row in y..end_y {
            for column in x..end_x {
                let offset = self.mode.pixel_offset(column, row)?;
                self.write_pixel_at_offset(offset, color);
            }
        }

        Ok(())
    }

    fn write_pixel_at_offset(&mut self, offset: usize, color: (u8, u8, u8)) {
        let (red, green, blue) = color;
        match self.mode.pixel_format() {
            PixelFormat::Rgb => {
                self.memory[offset] = red;
                self.memory[offset + 1] = green;
                self.memory[offset + 2] = blue;
                if self.mode.bytes_per_pixel() == 4 {
                    self.memory[offset + 3] = 0;
                }
            }
            PixelFormat::Bgr => {
                self.memory[offset] = blue;
                self.memory[offset + 1] = green;
                self.memory[offset + 2] = red;
                if self.mode.bytes_per_pixel() == 4 {
                    self.memory[offset + 3] = 0;
                }
            }
            PixelFormat::Xrgb => {
                self.memory[offset] = 0;
                self.memory[offset + 1] = red;
                self.memory[offset + 2] = green;
                self.memory[offset + 3] = blue;
            }
        }
    }
}

/// Minimal text console facade over a framebuffer.
///
/// This is intentionally a stub: it tracks a text cursor and paints coarse cell
/// blocks rather than embedding a production font renderer in the kernel-facing
/// framebuffer crate.
#[derive(Debug)]
pub struct FramebufferConsole<'fb> {
    framebuffer: &'fb mut Framebuffer,
    cursor_column: usize,
    cursor_row: usize,
    foreground: (u8, u8, u8),
    background: (u8, u8, u8),
}

impl<'fb> FramebufferConsole<'fb> {
    pub const CELL_WIDTH: usize = 8;
    pub const CELL_HEIGHT: usize = 16;

    pub fn new(framebuffer: &'fb mut Framebuffer) -> Self {
        Self {
            framebuffer,
            cursor_column: 0,
            cursor_row: 0,
            foreground: (0xff, 0xff, 0xff),
            background: (0, 0, 0),
        }
    }

    pub fn set_colors(&mut self, foreground: (u8, u8, u8), background: (u8, u8, u8)) {
        self.foreground = foreground;
        self.background = background;
    }

    pub const fn cursor(&self) -> (usize, usize) {
        (self.cursor_column, self.cursor_row)
    }

    pub fn write_str(&mut self, text: &str) -> Result<usize, FramebufferError> {
        let mut written = 0;
        for byte in text.bytes() {
            self.write_byte(byte)?;
            written += 1;
        }
        Ok(written)
    }

    pub fn write_byte(&mut self, byte: u8) -> Result<(), FramebufferError> {
        if byte == b'\n' {
            self.newline();
            return Ok(());
        }

        let x = self.cursor_column * Self::CELL_WIDTH;
        let y = self.cursor_row * Self::CELL_HEIGHT;
        if x + Self::CELL_WIDTH <= self.framebuffer.mode().width()
            && y + Self::CELL_HEIGHT <= self.framebuffer.mode().height()
        {
            let (br, bg, bb) = self.background;
            self.framebuffer.draw_rectangle(
                x,
                y,
                Self::CELL_WIDTH,
                Self::CELL_HEIGHT,
                (br, bg, bb),
            )?;

            if byte != b' ' {
                let (fr, fg, fb) = self.foreground;
                self.framebuffer.draw_rectangle(
                    x + 2,
                    y + 2,
                    Self::CELL_WIDTH - 4,
                    Self::CELL_HEIGHT - 4,
                    (fr, fg, fb),
                )?;
            }
        }

        self.advance_cursor();
        Ok(())
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
        let columns = self.framebuffer.mode().width() / Self::CELL_WIDTH;
        columns.max(1)
    }

    fn rows(&self) -> usize {
        let rows = self.framebuffer.mode().height() / Self::CELL_HEIGHT;
        rows.max(1)
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb_mode(width: usize, height: usize, pitch: usize) -> FramebufferMode {
        FramebufferMode::new(width, height, pitch, 24, PixelFormat::Rgb).unwrap()
    }

    #[test]
    fn mode_validation_rejects_invalid_modes() {
        assert_eq!(
            FramebufferMode::new(0, 1, 3, 24, PixelFormat::Rgb),
            Err(FramebufferError::InvalidDimensions)
        );
        assert_eq!(
            FramebufferMode::new(2, 2, 8, 24, PixelFormat::Xrgb),
            Err(FramebufferError::InvalidBitsPerPixel)
        );
        assert_eq!(
            FramebufferMode::new(4, 2, 11, 24, PixelFormat::Rgb),
            Err(FramebufferError::InvalidPitch)
        );
        assert!(FramebufferMode::new(4, 2, 12, 24, PixelFormat::Rgb).is_ok());
        assert!(FramebufferMode::new(4, 2, 16, 32, PixelFormat::Xrgb).is_ok());
    }

    #[test]
    fn pixel_offset_uses_pitch_and_bytes_per_pixel() {
        let mode = rgb_mode(4, 3, 16);

        assert_eq!(mode.pixel_offset(0, 0), Ok(0));
        assert_eq!(mode.pixel_offset(2, 1), Ok(22));
        assert_eq!(mode.pixel_offset(3, 2), Ok(41));
    }

    #[test]
    fn put_pixel_checks_bounds() {
        let mode = rgb_mode(2, 2, 6);
        let mut framebuffer = Framebuffer::new(mode).unwrap();

        assert_eq!(framebuffer.put_pixel(1, 1, (1, 2, 3)), Ok(()));
        assert_eq!(
            framebuffer.put_pixel(2, 1, (1, 2, 3)),
            Err(FramebufferError::OutOfBounds)
        );
        assert_eq!(
            framebuffer.put_pixel(1, 2, (1, 2, 3)),
            Err(FramebufferError::OutOfBounds)
        );
    }

    #[test]
    fn put_pixel_encodes_supported_formats() {
        let mode = FramebufferMode::new(1, 1, 4, 32, PixelFormat::Bgr).unwrap();
        let mut framebuffer = Framebuffer::new(mode).unwrap();
        framebuffer.put_pixel(0, 0, (0x11, 0x22, 0x33)).unwrap();
        assert_eq!(framebuffer.memory(), &[0x33, 0x22, 0x11, 0x00]);

        let mode = FramebufferMode::new(1, 1, 4, 32, PixelFormat::Xrgb).unwrap();
        let mut framebuffer = Framebuffer::new(mode).unwrap();
        framebuffer.put_pixel(0, 0, (0x11, 0x22, 0x33)).unwrap();
        assert_eq!(framebuffer.memory(), &[0x00, 0x11, 0x22, 0x33]);
    }

    #[test]
    fn clear_framebuffer_paints_visible_pixels() {
        let mode = rgb_mode(2, 2, 8);
        let mut framebuffer = Framebuffer::from_memory(mode, vec![0xaa; 16]).unwrap();

        framebuffer.clear((1, 2, 3));

        assert_eq!(
            framebuffer.memory(),
            &[1, 2, 3, 1, 2, 3, 0xaa, 0xaa, 1, 2, 3, 1, 2, 3, 0xaa, 0xaa]
        );
    }

    #[test]
    fn rectangle_draw_checks_bounds() {
        let mode = rgb_mode(4, 4, 12);
        let mut framebuffer = Framebuffer::new(mode).unwrap();

        assert_eq!(framebuffer.draw_rectangle(1, 1, 2, 2, (9, 8, 7)), Ok(()));
        assert_eq!(framebuffer.pixel_offset(1, 1), Ok(15));
        assert_eq!(&framebuffer.memory()[15..18], &[9, 8, 7]);
        assert_eq!(&framebuffer.memory()[30..33], &[9, 8, 7]);
        assert_eq!(
            framebuffer.draw_rectangle(3, 3, 2, 1, (1, 1, 1)),
            Err(FramebufferError::OutOfBounds)
        );
        assert_eq!(
            framebuffer.draw_rectangle(1, 0, usize::MAX, 1, (1, 1, 1)),
            Err(FramebufferError::SizeOverflow)
        );
    }
}
