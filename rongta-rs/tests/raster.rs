use image::{Rgba, RgbaImage};
use rongta::raster::Raster;
use serde_json::Value;

#[test]
fn conversion_matches_native_sdk_argb_vectors() {
    let corpus: Value = serde_json::from_str(include_str!("fixtures/sdk/image.json")).unwrap();
    for v in corpus["raster_vectors"].as_array().unwrap() {
        let width = v["width"].as_u64().unwrap() as u32;
        let height = v["height"].as_u64().unwrap() as u32;
        let mut bytes = hex::decode(v["argb_hex"].as_str().unwrap()).unwrap();
        for pixel in bytes.as_chunks_mut::<4>().0 {
            pixel.rotate_left(1);
        }
        let image = RgbaImage::from_raw(width, height, bytes).unwrap();
        let raster = Raster::from_rgba(&image, v["threshold"].as_u64().unwrap() as u8).unwrap();
        assert_eq!(
            hex::encode(raster.bytes()),
            v["raster_hex"],
            "{}",
            v["name"]
        );
        assert_eq!(raster.width(), width);
        assert_eq!(raster.height(), height);
    }
}

#[test]
fn partial_rows_are_padded_white_without_wrapping_into_the_next_row() {
    let mut image = RgbaImage::from_pixel(9, 2, Rgba([255, 255, 255, 255]));
    image.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
    image.put_pixel(8, 0, Rgba([0, 0, 0, 255]));
    image.put_pixel(1, 1, Rgba([0, 0, 0, 255]));
    let raster = Raster::from_rgba(&image, 208).unwrap();
    assert_eq!(raster.width(), 16);
    assert_eq!(raster.bytes(), [0x80, 0x80, 0x40, 0]);
}

#[test]
fn dimensions_and_packed_length_are_validated() {
    for (width, height, bytes) in [
        (0, 1, vec![]),
        (8, 0, vec![]),
        (9, 1, vec![0, 0]),
        (16, 2, vec![0; 3]),
        (u32::MAX, 1, vec![]),
    ] {
        assert!(Raster::new(width, height, bytes).is_err());
    }
    assert!(Raster::from_rgba(&RgbaImage::new(0, 0), 208).is_err());
    assert!(Raster::new(16, 2, vec![0; 4]).is_ok());
}

#[test]
fn centering_preserves_bits_across_byte_boundaries_and_rejects_cropping() {
    let source = Raster::new(8, 2, vec![0x81, 0x42]).unwrap();
    let centered = source.centered(16).unwrap();
    assert_eq!(centered.bytes(), [0x08, 0x10, 0x04, 0x20]);
    assert_eq!(source.centered(8).unwrap().bytes(), source.bytes());
    assert!(source.centered(0).is_err());
    assert!(source.centered(15).is_err());
    assert!(centered.centered(8).is_err());
}
