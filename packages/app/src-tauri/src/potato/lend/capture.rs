//! Window capture for lending: PrintWindow(PW_RENDERFULLCONTENT) -> PNG bytes. Windows only.
//! Read-only: draws into an off-screen bitmap, never touches the real window or screen.

use std::ffi::c_void;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDIBits, GetWindowDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};

use super::win::{hwnd, is_alive, os_err};
use super::{Hwnd, LendError, Rect, Result};

/// PW_RENDERFULLCONTENT: asks DWM to render layered/composited content too.
const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);

/// Capture a window as PNG. Returns (png bytes, width, height).
#[cfg_attr(not(test), allow(dead_code))] // kept for the latency live test
pub fn png(h: Hwnd) -> Result<(Vec<u8>, u32, u32)> {
    if !is_alive(h) {
        return Err(LendError::Gone);
    }
    let hw = hwnd(h);
    let rect = super::win::info(h)?.rect;
    let (w, ht) = size_from_rect(rect)
        .ok_or_else(|| LendError::Os("window has no size (minimized?)".to_string()))?;
    let mut bgra = unsafe { print_window_bgra(hw, w, ht)? };
    bgra_to_rgba_opaque(&mut bgra);
    let png = encode_png(w, ht, &bgra)?;
    Ok((png, w, ht))
}

/// Longest side (px) of the picture `app_observe` sends to the model.
pub const OBSERVE_MAX_SIDE: u32 = 1280;
/// JPEG quality of that picture.
pub const OBSERVE_JPEG_QUALITY: u8 = 80;

/// Capture a window as a JPEG whose long side is at most `max_side`. Returns (jpeg, out_w, out_h, window_w, window_h).
/// BGRA is converted to RGB while shrinking, in one pass; the full-size PNG path is much slower, especially in dev builds.
pub fn jpeg_scaled(h: Hwnd, max_side: u32, quality: u8) -> Result<(Vec<u8>, u32, u32, u32, u32)> {
    if !is_alive(h) {
        return Err(LendError::Gone);
    }
    let rect = super::win::info(h)?.rect;
    let (w, ht) = size_from_rect(rect)
        .ok_or_else(|| LendError::Os("window has no size (minimized?)".to_string()))?;
    let bgra = unsafe { print_window_bgra(hwnd(h), w, ht)? };
    let (nw, nh, rgb) = bgra_to_rgb_scaled(&bgra, w, ht, max_side);
    let jpeg = encode_jpeg_rgb(nw, nh, &rgb, quality)?;
    Ok((jpeg, nw, nh, w, ht))
}

/// Pure: BGRA -> RGB, box-averaged down so the long side is at most `max_side` (never enlarges). One pass over the source.
pub(crate) fn bgra_to_rgb_scaled(bgra: &[u8], w: u32, h: u32, max_side: u32) -> (u32, u32, Vec<u8>) {
    to_rgb_scaled(bgra, w, h, max_side, [2, 1, 0])
}

/// Pure: RGBA -> RGB, box-averaged down so the long side is at most `max_side`.
pub(crate) fn rgba_to_rgb_scaled(rgba: &[u8], w: u32, h: u32, max_side: u32) -> (u32, u32, Vec<u8>) {
    to_rgb_scaled(rgba, w, h, max_side, [0, 1, 2])
}

/// `order`: the byte index of red, green and blue inside one 4-byte pixel.
fn to_rgb_scaled(bgra: &[u8], w: u32, h: u32, max_side: u32, order: [usize; 3]) -> (u32, u32, Vec<u8>) {
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 || bgra.len() < w * h * 4 {
        return (w as u32, h as u32, Vec::new());
    }
    let long = w.max(h);
    let (nw, nh) = if max_side == 0 || long <= max_side as usize {
        (w, h)
    } else {
        let s = max_side as usize;
        (((w * s) / long).max(1), ((h * s) / long).max(1))
    };
    let xs: Vec<(usize, usize)> = (0..nw).map(|x| (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1).min(w))).collect();
    let mut out = Vec::with_capacity(nw * nh * 3);
    let mut acc = vec![[0u32; 3]; nw];
    for dy in 0..nh {
        let y0 = dy * h / nh;
        let y1 = ((dy + 1) * h / nh).max(y0 + 1).min(h);
        for a in acc.iter_mut() {
            *a = [0; 3];
        }
        for sy in y0..y1 {
            let row = &bgra[sy * w * 4..(sy + 1) * w * 4];
            for (a, &(x0, x1)) in acc.iter_mut().zip(&xs) {
                for p in row[x0 * 4..x1 * 4].chunks_exact(4) {
                    a[0] += p[order[0]] as u32;
                    a[1] += p[order[1]] as u32;
                    a[2] += p[order[2]] as u32;
                }
            }
        }
        let rows = (y1 - y0) as u32;
        for (a, &(x0, x1)) in acc.iter().zip(&xs) {
            let n = rows * (x1 - x0) as u32;
            out.extend_from_slice(&[(a[0] / n) as u8, (a[1] / n) as u8, (a[2] / n) as u8]);
        }
    }
    (nw as u32, nh as u32, out)
}

/// Pure: RGB pixels to JPEG bytes.
pub(crate) fn encode_jpeg_rgb(w: u32, h: u32, rgb: &[u8], quality: u8) -> Result<Vec<u8>> {
    let (w16, h16) = (
        u16::try_from(w).map_err(|_| LendError::Os("picture too wide".to_string()))?,
        u16::try_from(h).map_err(|_| LendError::Os("picture too tall".to_string()))?,
    );
    if rgb.len() != w as usize * h as usize * 3 {
        return Err(LendError::Os("pixel buffer has the wrong size".to_string()));
    }
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, quality)
        .encode(rgb, w16, h16, jpeg_encoder::ColorType::Rgb)
        .map_err(os_err)?;
    Ok(out)
}

/// Capture a window as raw BGRA (alpha undefined). Returns (pixels, width, height). The mirror hashes and shrinks these.
pub fn bgra(h: Hwnd) -> Result<(Vec<u8>, u32, u32)> {
    if !is_alive(h) {
        return Err(LendError::Gone);
    }
    let rect = super::win::info(h)?.rect;
    let (w, ht) = size_from_rect(rect)
        .ok_or_else(|| LendError::Os("window has no size (minimized?)".to_string()))?;
    let px = unsafe { print_window_bgra(hwnd(h), w, ht)? };
    Ok((px, w, ht))
}

/// Capture a window as raw opaque RGBA. Returns (pixels, width, height). The mirror hashes and JPEG-encodes these.
#[cfg_attr(not(test), allow(dead_code))] // kept for the latency live test
pub fn rgba(h: Hwnd) -> Result<(Vec<u8>, u32, u32)> {
    if !is_alive(h) {
        return Err(LendError::Gone);
    }
    let rect = super::win::info(h)?.rect;
    let (w, ht) = size_from_rect(rect)
        .ok_or_else(|| LendError::Os("window has no size (minimized?)".to_string()))?;
    let mut bgra = unsafe { print_window_bgra(hwnd(h), w, ht)? };
    bgra_to_rgba_opaque(&mut bgra);
    Ok((bgra, w, ht))
}

/// Capture a window as a PNG no wider than `max_w` px (nearest-neighbour downscale). For picker thumbnails.
pub fn png_scaled(h: Hwnd, max_w: u32) -> Result<Vec<u8>> {
    if !is_alive(h) {
        return Err(LendError::Gone);
    }
    let rect = super::win::info(h)?.rect;
    let (w, ht) = size_from_rect(rect)
        .ok_or_else(|| LendError::Os("window has no size (minimized?)".to_string()))?;
    let mut bgra = unsafe { print_window_bgra(hwnd(h), w, ht)? };
    bgra_to_rgba_opaque(&mut bgra);
    let (nw, nh, px) = downscale_nearest(&bgra, w, ht, max_w);
    encode_png(nw, nh, &px)
}

/// Pure: RGBA nearest-neighbour downscale so the width is at most `max_w` (never enlarges).
pub(crate) fn downscale_nearest(rgba: &[u8], w: u32, h: u32, max_w: u32) -> (u32, u32, Vec<u8>) {
    if max_w == 0 || w <= max_w || w == 0 || h == 0 {
        return (w, h, rgba.to_vec());
    }
    let nw = max_w;
    let nh = ((h as u64 * nw as u64 / w as u64) as u32).max(1);
    let mut out = Vec::with_capacity(nw as usize * nh as usize * 4);
    for y in 0..nh {
        let sy = (y as u64 * h as u64 / nh as u64) as usize;
        for x in 0..nw {
            let sx = (x as u64 * w as u64 / nw as u64) as usize;
            let i = (sy * w as usize + sx) * 4;
            out.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    (nw, nh, out)
}

/// Pure: window size from its rect, or None when it is empty.
pub(super) fn size_from_rect(r: Rect) -> Option<(u32, u32)> {
    if r.w <= 0 || r.h <= 0 {
        None
    } else {
        Some((r.w as u32, r.h as u32))
    }
}

/// Pure: BGRA -> RGBA in place, alpha forced opaque (PrintWindow leaves alpha undefined).
pub(super) fn bgra_to_rgba_opaque(px: &mut [u8]) {
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2);
        p[3] = 255;
    }
}

/// Pure: RGBA pixels to PNG bytes.
pub(super) fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let expected = w as usize * h as usize * 4;
    if rgba.len() != expected {
        return Err(LendError::Os(format!(
            "pixel buffer is {} bytes, expected {expected}",
            rgba.len()
        )));
    }
    let mut out: Vec<u8> = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_compression(png::Compression::Fast);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(os_err)?;
        writer.write_image_data(rgba).map_err(os_err)?;
        writer.finish().map_err(os_err)?;
    }
    Ok(out)
}

/// Owns every GDI handle taken during a capture and releases them in reverse order.
struct PwGuard {
    hwnd: HWND,
    window_dc: HDC,
    mem_dc: HDC,
    bitmap: HBITMAP,
    old_obj: HGDIOBJ,
}

impl Drop for PwGuard {
    fn drop(&mut self) {
        unsafe {
            if !self.old_obj.is_invalid() && !self.mem_dc.is_invalid() {
                SelectObject(self.mem_dc, self.old_obj);
            }
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            }
            if !self.mem_dc.is_invalid() {
                let _ = DeleteDC(self.mem_dc);
            }
            if !self.window_dc.is_invalid() {
                ReleaseDC(Some(self.hwnd), self.window_dc);
            }
        }
    }
}

/// PrintWindow into a 32-bit top-down DIB; returns raw BGRA bytes.
unsafe fn print_window_bgra(hw: HWND, w: u32, h: u32) -> Result<Vec<u8>> {
    let mut guard = PwGuard {
        hwnd: hw,
        window_dc: HDC::default(),
        mem_dc: HDC::default(),
        bitmap: HBITMAP::default(),
        old_obj: HGDIOBJ::default(),
    };
    unsafe {
        guard.window_dc = GetWindowDC(Some(hw));
        if guard.window_dc.is_invalid() {
            return Err(LendError::Os("GetWindowDC returned null".to_string()));
        }
        guard.mem_dc = CreateCompatibleDC(Some(guard.window_dc));
        if guard.mem_dc.is_invalid() {
            return Err(LendError::Os("CreateCompatibleDC returned null".to_string()));
        }
        guard.bitmap = CreateCompatibleBitmap(guard.window_dc, w as i32, h as i32);
        if guard.bitmap.is_invalid() {
            return Err(LendError::Os("CreateCompatibleBitmap returned null".to_string()));
        }
        guard.old_obj = SelectObject(guard.mem_dc, HGDIOBJ(guard.bitmap.0));

        if !PrintWindow(hw, guard.mem_dc, PW_RENDERFULLCONTENT).as_bool() {
            return Err(LendError::Os(
                "PrintWindow returned FALSE (window minimized, protected, or not drawable)".to_string(),
            ));
        }

        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            // Negative height = top-down rows.
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bgra = vec![0u8; w as usize * h as usize * 4];
        let lines = GetDIBits(
            guard.mem_dc,
            guard.bitmap,
            0,
            h,
            Some(bgra.as_mut_ptr() as *mut c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        if lines != h as i32 {
            return Err(LendError::Os(format!("GetDIBits copied {lines} of {h} rows")));
        }
        Ok(bgra)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_from_rect_rejects_empty() {
        assert_eq!(size_from_rect(Rect { x: 0, y: 0, w: 0, h: 10 }), None);
        assert_eq!(size_from_rect(Rect { x: 0, y: 0, w: 10, h: -1 }), None);
        assert_eq!(size_from_rect(Rect { x: -5, y: -5, w: 640, h: 480 }), Some((640, 480)));
    }

    #[test]
    fn bgra_becomes_opaque_rgba() {
        let mut px = vec![1, 2, 3, 0, 9, 8, 7, 128];
        bgra_to_rgba_opaque(&mut px);
        assert_eq!(px, vec![3, 2, 1, 255, 7, 8, 9, 255]);
    }

    #[test]
    fn png_round_trips_pixels() {
        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let bytes = encode_png(2, 1, &rgba).expect("encode");
        assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().expect("read_info");
        let mut buf = vec![0u8; reader.output_buffer_size().expect("size")];
        let info = reader.next_frame(&mut buf).expect("frame");
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(&buf[..info.buffer_size()], &rgba[..]);
    }

    #[test]
    fn scaled_conversion_averages_and_swaps() {
        // 4x2 BGRA, long side capped at 2 -> 2x1; each output pixel averages a 2x2 block.
        let row0 = [[0, 0, 200, 255], [0, 0, 200, 255], [0, 0, 100, 255], [0, 0, 100, 255]].concat();
        let bgra = [row0.clone(), row0].concat();
        let (w, h, rgb) = bgra_to_rgb_scaled(&bgra, 4, 2, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgb, vec![200, 0, 0, 100, 0, 0]);
        let (w, h, rgb) = bgra_to_rgb_scaled(&bgra, 4, 2, 0);
        assert_eq!((w, h, rgb.len()), (4, 2, 24));
        assert_eq!(&rgb[..3], &[200, 0, 0]);
    }

    #[test]
    fn jpeg_rgb_encodes() {
        let rgb = vec![10u8; 8 * 8 * 3];
        let jpeg = encode_jpeg_rgb(8, 8, &rgb, 80).expect("jpeg");
        assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF]);
        assert!(encode_jpeg_rgb(8, 8, &rgb[..5], 80).is_err());
    }

    #[test]
    fn encode_rejects_wrong_length() {
        assert!(encode_png(2, 2, &[0u8; 4]).is_err());
    }
}
