#![no_std]
#![cfg_attr(not(feature = "hw-framebuffer"), forbid(unsafe_code))]
#![cfg_attr(feature = "hw-framebuffer", deny(unsafe_op_in_unsafe_fn))]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
#[cfg(feature = "hw-framebuffer")]
use core::ptr::{self, NonNull};

/// Pixel layouts supported by the Mirage framebuffer abstraction.
///
/// The byte order is the order stored in linear framebuffer memory. For `Xrgb`,
/// the unused byte is written as zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PixelMasks {
    pub red: u32,
    pub green: u32,
    pub blue: u32,
    pub reserved: u32,
}

impl PixelMasks {
    pub const fn new(red: u32, green: u32, blue: u32, reserved: u32) -> Self {
        Self {
            red,
            green,
            blue,
            reserved,
        }
    }

    pub const fn from_size_shift(
        red_size: usize,
        red_shift: usize,
        green_size: usize,
        green_shift: usize,
        blue_size: usize,
        blue_shift: usize,
        reserved_size: usize,
        reserved_shift: usize,
    ) -> Result<Self, FramebufferError> {
        let red = match Self::channel_mask(red_size, red_shift) {
            Ok(mask) => mask,
            Err(error) => return Err(error),
        };
        let green = match Self::channel_mask(green_size, green_shift) {
            Ok(mask) => mask,
            Err(error) => return Err(error),
        };
        let blue = match Self::channel_mask(blue_size, blue_shift) {
            Ok(mask) => mask,
            Err(error) => return Err(error),
        };
        let reserved = match Self::channel_mask(reserved_size, reserved_shift) {
            Ok(mask) => mask,
            Err(error) => return Err(error),
        };

        Ok(Self::new(red, green, blue, reserved))
    }

    pub const fn channel_mask(size: usize, shift: usize) -> Result<u32, FramebufferError> {
        if size == 0 {
            return Ok(0);
        }

        if size > u32::BITS as usize || shift >= u32::BITS as usize {
            return Err(FramebufferError::SizeOverflow);
        }

        let end = match shift.checked_add(size) {
            Some(value) => value,
            None => return Err(FramebufferError::SizeOverflow),
        };
        if end > u32::BITS as usize {
            return Err(FramebufferError::SizeOverflow);
        }

        let unshifted = if size == u32::BITS as usize {
            u32::MAX
        } else {
            (1u32 << size) - 1
        };

        Ok(unshifted << shift)
    }

    pub const fn rgb888() -> Self {
        Self::new(0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000)
    }

    pub const fn bgr888() -> Self {
        Self::new(0x0000_00ff, 0x0000_ff00, 0x00ff_0000, 0xff00_0000)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PixelFormat {
    /// Red, green, blue byte order. Supported at 24 or 32 bits per pixel.
    Rgb,
    /// Blue, green, red byte order. Supported at 24 or 32 bits per pixel.
    Bgr,
    /// Unused byte, red, green, blue byte order. Supported at 32 bits per pixel.
    Xrgb,
    /// Bit-mask described packed pixels supplied by boot firmware.
    Masks(PixelMasks),
}

impl PixelFormat {
    pub const fn supports_bits_per_pixel(self, bits_per_pixel: usize) -> bool {
        match self {
            Self::Rgb | Self::Bgr => bits_per_pixel == 24 || bits_per_pixel == 32,
            Self::Xrgb => bits_per_pixel == 32,
            Self::Masks(_) => bits_per_pixel <= 32 && bits_per_pixel >= 8,
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
    NullAddress,
}

/// Linear framebuffer backed by mock memory for tests and display-service demos.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockFramebuffer {
    mode: FramebufferMode,
    memory: Vec<u8>,
}

impl MockFramebuffer {
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
        validate_rect(self.mode, x, y, width, height)?;

        let end_x = x + width;
        let end_y = y + height;
        for row in y..end_y {
            for column in x..end_x {
                let offset = self.mode.pixel_offset(column, row)?;
                write_pixel_at_offset(self.mode, &mut self.memory, offset, color);
            }
        }

        Ok(())
    }

    pub fn blit(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        pixels: &[(u8, u8, u8)],
    ) -> Result<(), FramebufferError> {
        validate_rect(self.mode, x, y, width, height)?;
        let required_pixels = width
            .checked_mul(height)
            .ok_or(FramebufferError::SizeOverflow)?;
        if pixels.len() < required_pixels {
            return Err(FramebufferError::BufferTooSmall);
        }

        for row in 0..height {
            for column in 0..width {
                let offset = self.mode.pixel_offset(x + column, y + row)?;
                let color = pixels[row * width + column];
                write_pixel_at_offset(self.mode, &mut self.memory, offset, color);
            }
        }

        Ok(())
    }

    fn write_pixel_at_offset(&mut self, offset: usize, color: (u8, u8, u8)) {
        write_pixel_at_offset(self.mode, &mut self.memory, offset, color);
    }
}

fn validate_rect(
    mode: FramebufferMode,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Result<(), FramebufferError> {
    if width == 0 || height == 0 {
        return Ok(());
    }

    let end_x = x.checked_add(width).ok_or(FramebufferError::SizeOverflow)?;
    let end_y = y
        .checked_add(height)
        .ok_or(FramebufferError::SizeOverflow)?;
    if x >= mode.width() || y >= mode.height() || end_x > mode.width() || end_y > mode.height() {
        return Err(FramebufferError::OutOfBounds);
    }

    Ok(())
}

fn write_pixel_at_offset(
    mode: FramebufferMode,
    memory: &mut [u8],
    offset: usize,
    color: (u8, u8, u8),
) {
    let (red, green, blue) = color;
    match mode.pixel_format() {
        PixelFormat::Rgb => {
            memory[offset] = red;
            memory[offset + 1] = green;
            memory[offset + 2] = blue;
            if mode.bytes_per_pixel() == 4 {
                memory[offset + 3] = 0;
            }
        }
        PixelFormat::Bgr => {
            memory[offset] = blue;
            memory[offset + 1] = green;
            memory[offset + 2] = red;
            if mode.bytes_per_pixel() == 4 {
                memory[offset + 3] = 0;
            }
        }
        PixelFormat::Xrgb => {
            memory[offset] = 0;
            memory[offset + 1] = red;
            memory[offset + 2] = green;
            memory[offset + 3] = blue;
        }
        PixelFormat::Masks(masks) => {
            let packed = encode_masked_pixel(masks, red, green, blue);
            let bytes = packed.to_le_bytes();
            let bytes_per_pixel = mode.bytes_per_pixel();
            memory[offset..offset + bytes_per_pixel].copy_from_slice(&bytes[..bytes_per_pixel]);
        }
    }
}

fn encode_masked_pixel(masks: PixelMasks, red: u8, green: u8, blue: u8) -> u32 {
    encode_masked_channel(masks.red, red)
        | encode_masked_channel(masks.green, green)
        | encode_masked_channel(masks.blue, blue)
}

fn encode_masked_channel(mask: u32, value: u8) -> u32 {
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

#[cfg(feature = "hw-framebuffer")]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct BootFramebuffer {
    pub physical_address: u64,
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bits_per_pixel: usize,
    pub pixel_format: PixelFormat,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub reserved_mask: u32,
}

#[cfg(feature = "hw-framebuffer")]
impl BootFramebuffer {
    pub const fn masks(self) -> PixelMasks {
        PixelMasks::new(
            self.red_mask,
            self.green_mask,
            self.blue_mask,
            self.reserved_mask,
        )
    }

    pub const fn mode(self) -> Result<FramebufferMode, FramebufferError> {
        FramebufferMode::new(
            self.width,
            self.height,
            self.pitch,
            self.bits_per_pixel,
            self.pixel_format,
        )
    }
}

#[cfg(feature = "hw-framebuffer")]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PhysicalFramebuffer {
    boot_info: BootFramebuffer,
    mode: FramebufferMode,
    byte_len: usize,
}

#[cfg(feature = "hw-framebuffer")]
impl PhysicalFramebuffer {
    pub const fn from_boot_info(boot_info: BootFramebuffer) -> Result<Self, FramebufferError> {
        let mode = match boot_info.mode() {
            Ok(mode) => mode,
            Err(error) => return Err(error),
        };
        let byte_len = match mode.framebuffer_len() {
            Ok(value) => value,
            Err(error) => return Err(error),
        };

        Ok(Self {
            boot_info,
            mode,
            byte_len,
        })
    }

    pub const fn boot_info(&self) -> BootFramebuffer {
        self.boot_info
    }

    pub const fn physical_address(&self) -> u64 {
        self.boot_info.physical_address
    }

    pub const fn mode(&self) -> FramebufferMode {
        self.mode
    }

    pub const fn info(&self) -> FramebufferInfo {
        FramebufferInfo {
            mode: self.mode,
            byte_len: self.byte_len,
        }
    }

    pub fn map<'map>(
        &self,
        memory: &'map mut [u8],
    ) -> Result<FramebufferMapping<'map>, FramebufferError> {
        FramebufferInfo::new(self.mode, memory.len())?;
        Ok(FramebufferMapping {
            info: self.info(),
            memory,
        })
    }
}

#[cfg(feature = "hw-framebuffer")]
impl MockFramebuffer {
    pub const fn from_boot_info(
        boot_info: BootFramebuffer,
    ) -> Result<PhysicalFramebuffer, FramebufferError> {
        PhysicalFramebuffer::from_boot_info(boot_info)
    }
}

#[cfg(feature = "hw-framebuffer")]
#[derive(Debug)]
pub struct FramebufferMapping<'map> {
    info: FramebufferInfo,
    memory: &'map mut [u8],
}

#[cfg(feature = "hw-framebuffer")]
impl<'map> FramebufferMapping<'map> {
    pub const fn info(&self) -> FramebufferInfo {
        self.info
    }

    pub const fn mode(&self) -> FramebufferMode {
        self.info.mode()
    }

    pub fn memory(&self) -> &[u8] {
        self.memory
    }

    pub fn memory_mut(&mut self) -> &mut [u8] {
        self.memory
    }

    pub fn writer(&mut self) -> FramebufferWriter<'_> {
        FramebufferWriter {
            mode: self.mode(),
            memory: self.memory,
        }
    }

    pub fn clear(&mut self, color: (u8, u8, u8)) {
        self.writer().clear(color);
    }

    pub fn put_pixel(
        &mut self,
        x: usize,
        y: usize,
        color: (u8, u8, u8),
    ) -> Result<(), FramebufferError> {
        self.writer().put_pixel(x, y, color)
    }

    pub fn blit(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        pixels: &[(u8, u8, u8)],
    ) -> Result<(), FramebufferError> {
        self.writer().blit(x, y, width, height, pixels)
    }
}

#[cfg(feature = "hw-framebuffer")]
#[derive(Debug)]
pub struct HardwareFramebufferMapping {
    base: NonNull<u8>,
    info: FramebufferInfo,
}

#[cfg(feature = "hw-framebuffer")]
impl HardwareFramebufferMapping {
    /// Creates a framebuffer mapping from a Limine-provided virtual framebuffer address.
    ///
    /// This constructor only accepts validated 32-bits-per-pixel modes. Other encoders
    /// are intentionally rejected until Mirage adds explicit hardware framebuffer
    /// support for them.
    ///
    /// # Safety
    ///
    /// `address` must be a non-null, writable virtual address mapped for at least
    /// `mode.framebuffer_len()` bytes for the entire lifetime of the returned mapping.
    /// The caller must ensure no other code concurrently mutates the same framebuffer
    /// memory in a way that violates Rust's aliasing rules or the active display
    /// service's ownership model.
    pub unsafe fn from_limine_address(
        address: u64,
        mode: FramebufferMode,
    ) -> Result<Self, FramebufferError> {
        if mode.bits_per_pixel() != 32 {
            return Err(FramebufferError::InvalidBitsPerPixel);
        }

        let byte_len = mode.framebuffer_len()?;
        let address = usize::try_from(address).map_err(|_| FramebufferError::SizeOverflow)?;
        let base = NonNull::new(ptr::with_exposed_provenance_mut(address))
            .ok_or(FramebufferError::NullAddress)?;
        let info = FramebufferInfo::new(mode, byte_len)?;

        Ok(Self { base, info })
    }

    pub const fn base_address(&self) -> NonNull<u8> {
        self.base
    }

    pub const fn info(&self) -> FramebufferInfo {
        self.info
    }

    pub const fn mode(&self) -> FramebufferMode {
        self.info.mode()
    }

    pub fn clear(&mut self, color: (u8, u8, u8)) {
        for y in 0..self.mode().height() {
            for x in 0..self.mode().width() {
                if let Ok(offset) = self.mode().pixel_offset(x, y) {
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
        let offset = self.mode().pixel_offset(x, y)?;
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
        validate_rect(self.mode(), x, y, width, height)?;
        let end_x = x + width;
        let end_y = y + height;
        for row in y..end_y {
            for column in x..end_x {
                let offset = self.mode().pixel_offset(column, row)?;
                self.write_pixel_at_offset(offset, color);
            }
        }
        Ok(())
    }

    pub fn blit(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        pixels: &[(u8, u8, u8)],
    ) -> Result<(), FramebufferError> {
        validate_rect(self.mode(), x, y, width, height)?;
        let required_pixels = width
            .checked_mul(height)
            .ok_or(FramebufferError::SizeOverflow)?;
        if pixels.len() < required_pixels {
            return Err(FramebufferError::BufferTooSmall);
        }

        for row in 0..height {
            for column in 0..width {
                let offset = self.mode().pixel_offset(x + column, y + row)?;
                let color = pixels[row * width + column];
                self.write_pixel_at_offset(offset, color);
            }
        }
        Ok(())
    }

    fn write_pixel_at_offset(&mut self, offset: usize, color: (u8, u8, u8)) {
        let bytes = encode_hardware_pixel(self.mode().pixel_format(), color);
        for (index, byte) in bytes.iter().enumerate() {
            // SAFETY: Construction validated that `base` references at least
            // `info.byte_len()` writable bytes, and all callers derive `offset` from
            // pitch-aware, bounds-checked framebuffer coordinates in a 32 bpp mode.
            unsafe {
                ptr::write_volatile(self.base.as_ptr().add(offset + index), *byte);
            }
        }
    }
}

#[cfg(feature = "hw-framebuffer")]
fn encode_hardware_pixel(pixel_format: PixelFormat, color: (u8, u8, u8)) -> [u8; 4] {
    let (red, green, blue) = color;
    match pixel_format {
        PixelFormat::Rgb => [red, green, blue, 0],
        PixelFormat::Bgr => [blue, green, red, 0],
        PixelFormat::Xrgb => [0, red, green, blue],
        PixelFormat::Masks(masks) => encode_masked_pixel(masks, red, green, blue).to_le_bytes(),
    }
}

#[cfg(feature = "hw-framebuffer")]
#[derive(Debug)]
pub struct FramebufferWriter<'map> {
    mode: FramebufferMode,
    memory: &'map mut [u8],
}

#[cfg(feature = "hw-framebuffer")]
impl<'map> FramebufferWriter<'map> {
    pub const fn mode(&self) -> FramebufferMode {
        self.mode
    }

    pub fn clear(&mut self, color: (u8, u8, u8)) {
        for y in 0..self.mode.height() {
            for x in 0..self.mode.width() {
                if let Ok(offset) = self.mode.pixel_offset(x, y) {
                    write_pixel_at_offset(self.mode, self.memory, offset, color);
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
        write_pixel_at_offset(self.mode, self.memory, offset, color);
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
        validate_rect(self.mode, x, y, width, height)?;
        for row in y..y + height {
            for column in x..x + width {
                let offset = self.mode.pixel_offset(column, row)?;
                write_pixel_at_offset(self.mode, self.memory, offset, color);
            }
        }
        Ok(())
    }

    pub fn blit(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        pixels: &[(u8, u8, u8)],
    ) -> Result<(), FramebufferError> {
        validate_rect(self.mode, x, y, width, height)?;
        let required_pixels = width
            .checked_mul(height)
            .ok_or(FramebufferError::SizeOverflow)?;
        if pixels.len() < required_pixels {
            return Err(FramebufferError::BufferTooSmall);
        }

        for row in 0..height {
            for column in 0..width {
                let offset = self.mode.pixel_offset(x + column, y + row)?;
                write_pixel_at_offset(self.mode, self.memory, offset, pixels[row * width + column]);
            }
        }
        Ok(())
    }
}

/// Backwards-compatible name for the memory-backed framebuffer backend.
///
/// New code that intentionally wants the in-memory test backend should use
/// [`MockFramebuffer`] so it is not confused with bootloader-provided physical
/// framebuffers behind the `hw-framebuffer` feature.
pub type Framebuffer = MockFramebuffer;

pub trait FramebufferTarget {
    fn mode(&self) -> FramebufferMode;

    fn draw_rectangle(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        color: (u8, u8, u8),
    ) -> Result<(), FramebufferError>;
}

impl FramebufferTarget for MockFramebuffer {
    fn mode(&self) -> FramebufferMode {
        self.mode()
    }

    fn draw_rectangle(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        color: (u8, u8, u8),
    ) -> Result<(), FramebufferError> {
        MockFramebuffer::draw_rectangle(self, x, y, width, height, color)
    }
}

#[cfg(feature = "hw-framebuffer")]
impl<'map> FramebufferTarget for FramebufferWriter<'map> {
    fn mode(&self) -> FramebufferMode {
        self.mode()
    }

    fn draw_rectangle(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        color: (u8, u8, u8),
    ) -> Result<(), FramebufferError> {
        FramebufferWriter::draw_rectangle(self, x, y, width, height, color)
    }
}

/// Minimal text console facade over a framebuffer.
///
/// This is intentionally a stub: it tracks a text cursor and paints coarse cell
/// blocks rather than embedding a production font renderer in the kernel-facing
/// framebuffer crate.
#[derive(Debug)]
pub struct FramebufferConsole<'fb, F: FramebufferTarget + ?Sized = MockFramebuffer> {
    framebuffer: &'fb mut F,
    cursor_column: usize,
    cursor_row: usize,
    foreground: (u8, u8, u8),
    background: (u8, u8, u8),
}

impl<'fb, F: FramebufferTarget + ?Sized> FramebufferConsole<'fb, F> {
    pub const CELL_WIDTH: usize = 8;
    pub const CELL_HEIGHT: usize = 16;

    pub fn new(framebuffer: &'fb mut F) -> Self {
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
    fn channel_masks_are_generated_from_size_and_shift() {
        assert_eq!(PixelMasks::channel_mask(8, 16), Ok(0x00ff_0000));
        assert_eq!(PixelMasks::channel_mask(8, 8), Ok(0x0000_ff00));
        assert_eq!(PixelMasks::channel_mask(8, 0), Ok(0x0000_00ff));
        assert_eq!(PixelMasks::channel_mask(0, 31), Ok(0));
    }

    #[test]
    fn channel_mask_generation_rejects_overflow_fields() {
        assert_eq!(
            PixelMasks::channel_mask(33, 0),
            Err(FramebufferError::SizeOverflow)
        );
        assert_eq!(
            PixelMasks::channel_mask(1, 32),
            Err(FramebufferError::SizeOverflow)
        );
        assert_eq!(
            PixelMasks::channel_mask(17, 16),
            Err(FramebufferError::SizeOverflow)
        );
    }

    #[test]
    fn pixel_masks_convert_boot_style_channel_fields() {
        assert_eq!(
            PixelMasks::from_size_shift(8, 16, 8, 8, 8, 0, 8, 24),
            Ok(PixelMasks::rgb888())
        );
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
    fn masked_32_bpp_pixels_encode_with_pitch_padding() {
        let mode =
            FramebufferMode::new(2, 2, 12, 32, PixelFormat::Masks(PixelMasks::rgb888())).unwrap();
        let mut framebuffer = Framebuffer::from_memory(mode, vec![0xaa; 24]).unwrap();

        framebuffer.put_pixel(1, 1, (0xff, 0x80, 0x00)).unwrap();

        assert_eq!(framebuffer.pixel_offset(1, 1), Ok(16));
        assert_eq!(&framebuffer.memory()[16..20], &[0x00, 0x80, 0xff, 0x00]);
        assert_eq!(&framebuffer.memory()[8..12], &[0xaa, 0xaa, 0xaa, 0xaa]);
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

    #[test]
    fn mock_framebuffer_name_is_available() {
        let mode = rgb_mode(1, 1, 3);
        let framebuffer = MockFramebuffer::new(mode).unwrap();

        assert_eq!(framebuffer.mode(), mode);
    }

    #[test]
    fn blit_checks_bounds_and_buffer_size() {
        let mode = rgb_mode(3, 3, 12);
        let mut framebuffer = MockFramebuffer::new(mode).unwrap();
        let pixels = [(1, 2, 3), (4, 5, 6), (7, 8, 9), (10, 11, 12)];

        assert_eq!(framebuffer.blit(1, 1, 2, 2, &pixels), Ok(()));
        assert_eq!(&framebuffer.memory()[15..18], &[1, 2, 3]);
        assert_eq!(&framebuffer.memory()[18..21], &[4, 5, 6]);
        assert_eq!(
            framebuffer.blit(2, 2, 2, 2, &pixels),
            Err(FramebufferError::OutOfBounds)
        );
        assert_eq!(
            framebuffer.blit(0, 0, 2, 2, &pixels[..3]),
            Err(FramebufferError::BufferTooSmall)
        );
    }

    #[cfg(feature = "hw-framebuffer")]
    fn boot_framebuffer() -> BootFramebuffer {
        BootFramebuffer {
            physical_address: 0xfeed_cafe,
            width: 3,
            height: 2,
            pitch: 16,
            bits_per_pixel: 32,
            pixel_format: PixelFormat::Masks(PixelMasks::rgb888()),
            red_mask: PixelMasks::rgb888().red,
            green_mask: PixelMasks::rgb888().green,
            blue_mask: PixelMasks::rgb888().blue,
            reserved_mask: PixelMasks::rgb888().reserved,
        }
    }

    #[cfg(feature = "hw-framebuffer")]
    #[test]
    fn boot_framebuffer_maps_physical_description_without_linux_api() {
        let physical = Framebuffer::from_boot_info(boot_framebuffer()).unwrap();
        let mut memory = vec![0; physical.info().byte_len()];
        let mapping = physical.map(&mut memory).unwrap();

        assert_eq!(physical.physical_address(), 0xfeed_cafe);
        assert_eq!(mapping.mode().pixel_offset(2, 1), Ok(24));
    }

    #[cfg(feature = "hw-framebuffer")]
    #[test]
    fn mapping_converts_masked_pixels_and_preserves_pitch_padding() {
        let physical = Framebuffer::from_boot_info(boot_framebuffer()).unwrap();
        let mut memory = vec![0xaa; physical.info().byte_len()];
        let mut mapping = physical.map(&mut memory).unwrap();

        mapping.clear((0, 0, 0));
        assert_eq!(&mapping.memory()[12..16], &[0xaa, 0xaa, 0xaa, 0xaa]);
        mapping.put_pixel(1, 1, (0xff, 0x80, 0x00)).unwrap();

        assert_eq!(&mapping.memory()[20..24], &[0x00, 0x80, 0xff, 0x00]);
    }

    #[cfg(feature = "hw-framebuffer")]
    #[test]
    fn mapping_bounds_checks_pixel_and_blit_operations() {
        let physical = Framebuffer::from_boot_info(boot_framebuffer()).unwrap();
        let mut memory = vec![0; physical.info().byte_len()];
        let mut mapping = physical.map(&mut memory).unwrap();
        let pixels = [(1, 2, 3), (4, 5, 6)];

        assert_eq!(
            mapping.put_pixel(3, 0, (1, 2, 3)),
            Err(FramebufferError::OutOfBounds)
        );
        assert_eq!(
            mapping.blit(2, 1, 2, 1, &pixels),
            Err(FramebufferError::OutOfBounds)
        );
        assert_eq!(
            mapping.blit(0, 0, 2, 1, &pixels[..1]),
            Err(FramebufferError::BufferTooSmall)
        );
    }

    #[cfg(feature = "hw-framebuffer")]
    #[test]
    fn hardware_mapping_rejects_null_and_non_32_bpp_modes() {
        let mode_32 = FramebufferMode::new(1, 1, 4, 32, PixelFormat::Rgb).unwrap();
        let mode_24 = FramebufferMode::new(1, 1, 3, 24, PixelFormat::Rgb).unwrap();

        assert_eq!(
            unsafe { HardwareFramebufferMapping::from_limine_address(0, mode_32) }.unwrap_err(),
            FramebufferError::NullAddress
        );
        assert_eq!(
            unsafe { HardwareFramebufferMapping::from_limine_address(0x1000, mode_24) }
                .unwrap_err(),
            FramebufferError::InvalidBitsPerPixel
        );
    }

    #[cfg(feature = "hw-framebuffer")]
    #[test]
    fn hardware_mapping_uses_volatile_32_bpp_pitch_aware_writes() {
        let mode = FramebufferMode::new(2, 2, 12, 32, PixelFormat::Bgr).unwrap();
        let mut memory = vec![0xaau8; mode.framebuffer_len().unwrap()];
        let address = memory.as_mut_ptr().expose_provenance() as u64;
        let mut mapping =
            unsafe { HardwareFramebufferMapping::from_limine_address(address, mode) }.unwrap();

        assert_eq!(mapping.info().byte_len(), 24);
        assert_eq!(mapping.put_pixel(1, 1, (0x11, 0x22, 0x33)), Ok(()));
        assert_eq!(
            mapping.put_pixel(2, 0, (1, 2, 3)),
            Err(FramebufferError::OutOfBounds)
        );
        drop(mapping);

        assert_eq!(&memory[16..20], &[0x33, 0x22, 0x11, 0x00]);
        assert_eq!(&memory[8..12], &[0xaa, 0xaa, 0xaa, 0xaa]);
    }

    #[cfg(feature = "hw-framebuffer")]
    #[test]
    fn console_writes_to_memory_backed_mapping() {
        let boot_info = BootFramebuffer {
            width: 16,
            height: 16,
            pitch: 64,
            ..boot_framebuffer()
        };
        let physical = Framebuffer::from_boot_info(boot_info).unwrap();
        let mut memory = vec![0; physical.info().byte_len()];
        let mut mapping = physical.map(&mut memory).unwrap();

        {
            let mut writer = mapping.writer();
            let mut console = FramebufferConsole::new(&mut writer);
            assert_eq!(console.write_str("A\nB"), Ok(3));
            assert_eq!(console.cursor(), (1, 0));
        }

        assert!(mapping.memory().iter().any(|byte| *byte == 0xff));
    }
}
