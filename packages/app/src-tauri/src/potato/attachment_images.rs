//! Pictures an owner attaches, made ready for a model that can look at them.
//!
//! The saved file is never changed. A copy goes to the brain inside the `send` item (`images`), at most 4 of them:
//! - a picture whose long side is over `LONG_SIDE` (1568 px, the size models read well) is shrunk to that;
//! - a picture over `REENCODE_BYTES` (about 1.5 MB) is sent as a JPEG;
//! - anything else is sent as it is.
//!
//! Only PNG can be opened here (the `png` crate is the one decoder the app carries). A JPEG, GIF or WebP goes as it is:
//! the app already refuses a file over 2 MB, so none is big, and the model's own service shrinks the rest. When a PNG
//! cannot be opened, or is far too large to open safely, it goes as it is too. Nothing is ever read from a path the
//! caller names: the caller passes bytes.

use std::io::{BufRead, Read, Seek, SeekFrom};

use base64::Engine as _;
use serde_json::{json, Value};

/// The longest side a picture is sent at.
pub const LONG_SIDE: u32 = 1568;
/// A picture bigger than this many bytes is sent as a JPEG.
pub const REENCODE_BYTES: usize = 1_500_000;
/// Pictures on one message.
pub const MAX_IMAGES: usize = 4;
/// A PNG with more pixels than this is not opened (its pixels would take over 100 MB).
const MAX_PIXELS: u64 = 24_000_000;
const JPEG_QUALITY: u8 = 85;

/// A picture as the brain gets it: its type and its bytes in base64.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImagePart {
    pub mime_type: String,
    pub data: String,
}

impl ImagePart {
    pub fn to_json(&self) -> Value {
        json!({ "mimeType": self.mime_type, "data": self.data })
    }
}

/// What to do with a picture of `w` x `h` pixels and `bytes` bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Shrink to this size (the long side is `LONG_SIDE`); `None` keeps the size.
    pub size: Option<(u32, u32)>,
    /// Send as a JPEG.
    pub jpeg: bool,
}

impl Plan {
    /// Nothing to do: send the file as it is.
    pub fn is_noop(&self) -> bool {
        self.size.is_none() && !self.jpeg
    }
}

/// Pure: the decision. The shrunk size keeps the shape (rounded, never 0).
pub fn downscale_plan(w: u32, h: u32, bytes: usize) -> Plan {
    let long = w.max(h);
    let size = if w > 0 && h > 0 && long > LONG_SIDE {
        let scale = |side: u32| ((u64::from(side) * u64::from(LONG_SIDE) + u64::from(long) / 2) / u64::from(long)).max(1) as u32;
        Some((scale(w), scale(h)))
    } else {
        None
    };
    Plan { size, jpeg: bytes > REENCODE_BYTES }
}

/// The media type of a picture by the extension `attachments::image_ext` found.
pub fn mime_of(ext: &str) -> Option<&'static str> {
    match ext {
        "png" => Some("image/png"),
        "jpg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// The bytes of a file read in place (the decoder needs to read and to step back).
struct Mem<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Read for Mem<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.bytes.get(self.at..).unwrap_or(&[]).read(buf)?;
        self.at += n;
        Ok(n)
    }
}

impl BufRead for Mem<'_> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        Ok(self.bytes.get(self.at..).unwrap_or(&[]))
    }
    fn consume(&mut self, amt: usize) {
        self.at = (self.at + amt).min(self.bytes.len());
    }
}

impl Seek for Mem<'_> {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.bytes.len() as i128 + i128::from(n),
            SeekFrom::Current(n) => self.at as i128 + i128::from(n),
        };
        if target < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "before the start"));
        }
        self.at = (target as usize).min(self.bytes.len());
        Ok(self.at as u64)
    }
}

fn part(mime: &str, bytes: &[u8]) -> ImagePart {
    ImagePart { mime_type: mime.to_string(), data: base64::engine::general_purpose::STANDARD.encode(bytes) }
}

/// The picture ready to send, or `None` when the bytes are not a picture.
pub fn prepare(bytes: &[u8]) -> Option<ImagePart> {
    let ext = super::attachments::image_ext(bytes)?;
    let mime = mime_of(ext)?;
    if ext == "png" {
        if let Some(changed) = prepare_png(bytes) {
            return Some(changed);
        }
    }
    Some(part(mime, bytes))
}

/// A PNG that needs shrinking or re-encoding, done; `None` to send the file as it is.
fn prepare_png(bytes: &[u8]) -> Option<ImagePart> {
    let (w, h) = png_size(bytes)?;
    let plan = downscale_plan(w, h, bytes.len());
    if plan.is_noop() || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    let (w, h, rgba) = png_pixels(bytes)?;
    let (nw, nh) = plan.size.unwrap_or((w, h));
    let rgba = if (nw, nh) == (w, h) { rgba } else { shrink_rgba(&rgba, w, h, nw, nh) };
    if plan.jpeg {
        let jpeg = encode_jpeg(nw, nh, &flatten(&rgba))?;
        // A re-encode that did not make it smaller, at the same size, is not worth the loss.
        if plan.size.is_none() && jpeg.len() >= bytes.len() {
            return None;
        }
        return Some(part("image/jpeg", &jpeg));
    }
    Some(part("image/png", &encode_png(nw, nh, &rgba)?))
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let reader = png::Decoder::new(Mem { bytes, at: 0 }).read_info().ok()?;
    let info = reader.info();
    Some((info.width, info.height))
}

/// 8-bit RGBA pixels of a PNG of any colour type.
fn png_pixels(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(Mem { bytes, at: 0 });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let data = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Indexed => return None,
    };
    if rgba.len() != info.width as usize * info.height as usize * 4 {
        return None;
    }
    Some((info.width, info.height, rgba))
}

/// Pure: box-average RGBA pixels down to `nw` x `nh`.
pub(super) fn shrink_rgba(rgba: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let (w, h, nw, nh) = (w as usize, h as usize, nw as usize, nh as usize);
    let mut out = Vec::with_capacity(nw * nh * 4);
    for dy in 0..nh {
        let y0 = dy * h / nh;
        let y1 = ((dy + 1) * h / nh).max(y0 + 1).min(h);
        for dx in 0..nw {
            let x0 = dx * w / nw;
            let x1 = ((dx + 1) * w / nw).max(x0 + 1).min(w);
            let mut sum = [0u64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
                    for (s, c) in sum.iter_mut().zip(p) {
                        *s += u64::from(*c);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            out.extend(sum.iter().map(|s| (s / n) as u8));
        }
    }
    out
}

/// Pure: RGBA over white, as RGB (a JPEG has no transparency).
pub(super) fn flatten(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| {
            let a = u32::from(p[3]);
            [0, 1, 2].map(|i| ((u32::from(p[i]) * a + 255 * (255 - a)) / 255) as u8)
        })
        .collect()
}

fn encode_jpeg(w: u32, h: u32, rgb: &[u8]) -> Option<Vec<u8>> {
    let (w16, h16) = (u16::try_from(w).ok()?, u16::try_from(h).ok()?);
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, JPEG_QUALITY).encode(rgb, w16, h16, jpeg_encoder::ColorType::Rgb).ok()?;
    Some(out)
}

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
        writer.finish().ok()?;
    }
    Some(out)
}
