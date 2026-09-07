use std::{io::Cursor, ptr::null_mut, thread, time::Duration};

use desktop_notes_core::{CapturedClipboard, ClipboardRead, ClipboardReader};
use image::{DynamicImage, ImageFormat};
use windows_sys::Win32::System::{
    DataExchange::{
        CloseClipboard, CountClipboardFormats, GetClipboardData, GetClipboardSequenceNumber,
        IsClipboardFormatAvailable, OpenClipboard,
    },
    Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    Ole::{CF_DIB, CF_DIBV5, CF_UNICODETEXT},
};

const MAX_CLIPBOARD_TEXT_BYTES: usize = 1024 * 1024;
const MAX_DIB_BYTES: usize = 192 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 8192;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;

pub struct WindowsClipboard {
    max_open_attempts: u8,
    retry_delay: Duration,
}

impl Default for WindowsClipboard {
    fn default() -> Self {
        Self {
            max_open_attempts: 4,
            retry_delay: Duration::from_millis(12),
        }
    }
}

impl ClipboardReader for WindowsClipboard {
    fn read_once(&self) -> ClipboardRead {
        // The sequence number is sampled before the bounded acquisition loop so
        // a later copy cannot be mistaken for the content present at the hotkey.
        let initial_sequence = unsafe { GetClipboardSequenceNumber() };
        for attempt in 1..=self.max_open_attempts {
            if unsafe { OpenClipboard(null_mut()) } != 0 {
                let _open = ClipboardOpenGuard;
                let acquired_sequence = unsafe { GetClipboardSequenceNumber() };
                let content = if initial_sequence != 0 && acquired_sequence != initial_sequence {
                    CapturedClipboard::Busy
                } else {
                    read_open_clipboard()
                };
                return ClipboardRead {
                    content,
                    attempts: attempt,
                };
            }
            if attempt < self.max_open_attempts {
                thread::sleep(self.retry_delay);
            }
        }
        ClipboardRead {
            content: CapturedClipboard::Busy,
            attempts: self.max_open_attempts,
        }
    }
}

struct ClipboardOpenGuard;

impl Drop for ClipboardOpenGuard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

struct GlobalMemoryGuard {
    handle: *mut core::ffi::c_void,
    pointer: *const u8,
}

impl GlobalMemoryGuard {
    fn lock(handle: *mut core::ffi::c_void) -> Option<Self> {
        let pointer = unsafe { GlobalLock(handle) }.cast::<u8>();
        (!pointer.is_null()).then_some(Self { handle, pointer })
    }
}

impl Drop for GlobalMemoryGuard {
    fn drop(&mut self) {
        unsafe {
            GlobalUnlock(self.handle);
        }
    }
}

fn read_open_clipboard() -> CapturedClipboard {
    let image = if unsafe { IsClipboardFormatAvailable(u32::from(CF_DIBV5)) } != 0 {
        Some(read_dib_format(u32::from(CF_DIBV5)))
    } else if unsafe { IsClipboardFormatAvailable(u32::from(CF_DIB)) } != 0 {
        Some(read_dib_format(u32::from(CF_DIB)))
    } else {
        None
    };
    match image {
        Some(CapturedClipboard::ImagePng(png)) => {
            if unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT)) } != 0
                && let CapturedClipboard::Text(text) = read_unicode_text()
            {
                return CapturedClipboard::TextAndImage { text, png };
            }
            return CapturedClipboard::ImagePng(png);
        }
        Some(image_error) => {
            if unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT)) } != 0 {
                return read_unicode_text();
            }
            return image_error;
        }
        None if unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT)) } != 0 => {
            return read_unicode_text();
        }
        None => {}
    }
    if unsafe { CountClipboardFormats() } == 0 {
        CapturedClipboard::Empty
    } else {
        CapturedClipboard::Unsupported
    }
}

fn read_unicode_text() -> CapturedClipboard {
    let handle = unsafe { GetClipboardData(u32::from(CF_UNICODETEXT)) };
    if handle.is_null() {
        return CapturedClipboard::Failed;
    }
    let byte_size = unsafe { GlobalSize(handle) };
    if !(2..=MAX_CLIPBOARD_TEXT_BYTES).contains(&byte_size) || !byte_size.is_multiple_of(2) {
        return CapturedClipboard::Failed;
    }
    let Some(memory) = GlobalMemoryGuard::lock(handle) else {
        return CapturedClipboard::Failed;
    };
    let units = unsafe {
        std::slice::from_raw_parts(memory.pointer.cast::<u16>(), byte_size / size_of::<u16>())
    };
    let length = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    match String::from_utf16(&units[..length]) {
        Ok(text) if text.is_empty() => CapturedClipboard::Empty,
        Ok(text) => CapturedClipboard::Text(text),
        Err(_) => CapturedClipboard::Failed,
    }
}

fn read_dib_format(format: u32) -> CapturedClipboard {
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return CapturedClipboard::Failed;
    }
    let byte_size = unsafe { GlobalSize(handle) };
    if !(40..=MAX_DIB_BYTES).contains(&byte_size) {
        return CapturedClipboard::Failed;
    }
    let Some(memory) = GlobalMemoryGuard::lock(handle) else {
        return CapturedClipboard::Failed;
    };
    let dib = unsafe { std::slice::from_raw_parts(memory.pointer, byte_size) };
    let Ok(bmp) = dib_to_bmp(dib) else {
        return CapturedClipboard::Failed;
    };
    let Ok(decoded) = image::load_from_memory_with_format(&bmp, ImageFormat::Bmp) else {
        return CapturedClipboard::Failed;
    };
    let width = decoded.width();
    let height = decoded.height();
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return CapturedClipboard::Failed;
    }
    let mut output = Cursor::new(Vec::new());
    if DynamicImage::ImageRgba8(decoded.to_rgba8())
        .write_to(&mut output, ImageFormat::Png)
        .is_err()
    {
        return CapturedClipboard::Failed;
    }
    CapturedClipboard::ImagePng(output.into_inner())
}

fn dib_to_bmp(dib: &[u8]) -> Result<Vec<u8>, ()> {
    let header_size = read_u32(dib, 0)? as usize;
    if header_size < 12 || header_size > dib.len() {
        return Err(());
    }
    let (width, height, bits_per_pixel, compression, colors_used, palette_entry_size) =
        if header_size == 12 {
            (
                u32::from(read_u16(dib, 4)?),
                u32::from(read_u16(dib, 6)?),
                read_u16(dib, 10)?,
                0,
                0,
                3_usize,
            )
        } else if header_size >= 40 {
            let width = read_i32(dib, 4)?.unsigned_abs();
            let height = read_i32(dib, 8)?.unsigned_abs();
            if read_u16(dib, 12)? != 1 {
                return Err(());
            }
            (
                width,
                height,
                read_u16(dib, 14)?,
                read_u32(dib, 16)?,
                read_u32(dib, 32)?,
                4_usize,
            )
        } else {
            return Err(());
        };
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err(());
    }

    let mask_bytes = if header_size == 40 {
        match compression {
            3 => 12_usize,
            6 => 16_usize,
            _ => 0_usize,
        }
    } else {
        0
    };
    let palette_entries = if colors_used > 0 {
        usize::try_from(colors_used).map_err(|_| ())?
    } else if bits_per_pixel <= 8 {
        1_usize.checked_shl(u32::from(bits_per_pixel)).ok_or(())?
    } else {
        0
    };
    let pixel_offset_in_dib = header_size
        .checked_add(mask_bytes)
        .and_then(|value| value.checked_add(palette_entries.checked_mul(palette_entry_size)?))
        .ok_or(())?;
    if pixel_offset_in_dib >= dib.len() {
        return Err(());
    }

    let file_size = 14_usize.checked_add(dib.len()).ok_or(())?;
    let pixel_offset = 14_usize.checked_add(pixel_offset_in_dib).ok_or(())?;
    let mut bmp = Vec::with_capacity(file_size);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&u32::try_from(file_size).map_err(|_| ())?.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&u32::try_from(pixel_offset).map_err(|_| ())?.to_le_bytes());
    bmp.extend_from_slice(dib);
    Ok(bmp)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ()> {
    let value = bytes.get(offset..offset + 2).ok_or(())?;
    Ok(u16::from_le_bytes(value.try_into().map_err(|_| ())?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ()> {
    let value = bytes.get(offset..offset + 4).ok_or(())?;
    Ok(u32::from_le_bytes(value.try_into().map_err(|_| ())?))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, ()> {
    let value = bytes.get(offset..offset + 4).ok_or(())?;
    Ok(i32::from_le_bytes(value.try_into().map_err(|_| ())?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_a_bounded_32_bit_dib_to_a_decodable_bmp() {
        let mut dib = vec![0_u8; 44];
        dib[0..4].copy_from_slice(&40_u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1_i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1_i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32_u16.to_le_bytes());
        dib[20..24].copy_from_slice(&4_u32.to_le_bytes());
        dib[40..44].copy_from_slice(&[0, 0, 255, 255]);

        let bmp = dib_to_bmp(&dib).expect("valid synthetic DIB");
        let image = image::load_from_memory_with_format(&bmp, ImageFormat::Bmp)
            .expect("generated BMP must decode");
        assert_eq!((image.width(), image.height()), (1, 1));
    }

    #[test]
    fn rejects_oversized_dimensions_before_decoding_pixels() {
        let mut dib = vec![0_u8; 44];
        dib[0..4].copy_from_slice(&40_u32.to_le_bytes());
        dib[4..8].copy_from_slice(&9000_i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1_i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32_u16.to_le_bytes());
        assert!(dib_to_bmp(&dib).is_err());
    }
}
