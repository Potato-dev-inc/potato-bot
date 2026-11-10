//! Pictures for the model: the decision to shrink, and the shrinking itself, on small made-up pictures.

use base64::Engine as _;

use super::attachment_images::*;

fn png_bytes(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
        writer.finish().unwrap();
    }
    out
}

/// A picture of one colour (it packs small).
fn flat(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
    png_bytes(w, h, &px.repeat(w as usize * h as usize))
}

fn decode(part: &ImagePart) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(&part.data).unwrap()
}

fn size_of(bytes: &[u8]) -> (u32, u32) {
    let reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
    (reader.info().width, reader.info().height)
}

#[test]
fn a_small_picture_is_left_as_it_is() {
    assert!(downscale_plan(800, 600, 50_000).is_noop());
    assert!(downscale_plan(1568, 1568, REENCODE_BYTES).is_noop(), "exactly at the limits is fine");
}

#[test]
fn a_long_side_over_the_limit_is_shrunk_keeping_the_shape() {
    assert_eq!(downscale_plan(3136, 1568, 10_000), Plan { size: Some((1568, 784)), jpeg: false });
    assert_eq!(downscale_plan(1000, 4000, 10_000).size, Some((392, 1568)));
    // Never a side of 0.
    assert_eq!(downscale_plan(100_000, 10, 10_000).size, Some((1568, 1)));
}

#[test]
fn a_file_over_one_and_a_half_megabytes_becomes_a_jpeg() {
    assert_eq!(downscale_plan(1000, 1000, REENCODE_BYTES + 1), Plan { size: None, jpeg: true });
    assert_eq!(downscale_plan(4000, 3000, 2_000_000), Plan { size: Some((1568, 1176)), jpeg: true });
    assert_eq!(downscale_plan(0, 0, 10).size, None, "an empty size is not divided");
}

#[test]
fn a_small_png_goes_unchanged_with_its_type() {
    let png = flat(4, 4, [10, 20, 30, 255]);
    let part = prepare(&png).unwrap();
    assert_eq!(part.mime_type, "image/png");
    assert_eq!(decode(&part), png);
}

#[test]
fn a_big_png_is_shrunk_to_the_long_side_limit() {
    let png = flat(3200, 1600, [200, 10, 10, 255]);
    let part = prepare(&png).unwrap();
    assert_eq!(part.mime_type, "image/png");
    assert_eq!(size_of(&decode(&part)), (1568, 784));
}

#[test]
fn other_kinds_go_as_they_are_and_a_non_picture_is_nothing() {
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
    let part = prepare(&jpeg).unwrap();
    assert_eq!((part.mime_type.as_str(), decode(&part)), ("image/jpeg", jpeg.to_vec()));
    assert_eq!(prepare(b"GIF89a....").unwrap().mime_type, "image/gif");
    assert_eq!(prepare(b"RIFF\x01\x00\x00\x00WEBPVP8 ").unwrap().mime_type, "image/webp");
    assert!(prepare(b"plain text").is_none());
    // A PNG that cannot be opened goes as it is rather than being lost.
    let broken = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 1];
    assert_eq!(decode(&prepare(&broken).unwrap()), broken.to_vec());
}

#[test]
fn the_shrink_averages_blocks_and_a_jpeg_is_flat_over_white() {
    // 2x1: red then blue, shrunk to 1x1, is purple.
    assert_eq!(shrink_rgba(&[255, 0, 0, 255, 0, 0, 255, 255], 2, 1, 1, 1), vec![127, 0, 127, 255]);
    // A fully clear pixel over white is white; a half clear black one is mid grey.
    assert_eq!(flatten(&[0, 0, 0, 0]), vec![255, 255, 255]);
    assert_eq!(flatten(&[0, 0, 0, 128]), vec![127, 127, 127]);
}

#[test]
fn the_image_part_is_the_shape_the_brain_reads() {
    let part = ImagePart { mime_type: "image/png".into(), data: "AAAA".into() };
    assert_eq!(part.to_json(), serde_json::json!({"mimeType": "image/png", "data": "AAAA"}));
}
