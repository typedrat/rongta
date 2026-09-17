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
