use image::{Rgba, RgbaImage};
use rongta::monochrome::{Conversion, Dither, convert};

const METHODS: &[Dither] = &[
    Dither::FloydSteinberg,
    Dither::Atkinson,
    Dither::Jarvis,
    Dither::Stucki,
    Dither::Sierra,
    Dither::SierraLite,
    Dither::Bayer,
];

#[test]
fn matches_img2idx_one_bit_fixtures_including_alpha_and_serpentine_rows() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dither.json")).unwrap();
    let source = RgbaImage::from_raw(
        fixture["width"].as_u64().unwrap() as u32,
        fixture["height"].as_u64().unwrap() as u32,
        fixture["rgba"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect(),
    )
    .unwrap();
    for (name, expected) in fixture["rasters"].as_object().unwrap() {
        let raster = convert(
            &source,
            source.dimensions(),
            Conversion::Dither(name.parse().unwrap()),
        )
        .unwrap();
        assert_eq!(raster.width(), 24);
        assert_eq!(
            hex::encode(raster.bytes()),
            expected.as_str().unwrap(),
            "{name}"
        );
        assert!(
            raster
                .bytes()
                .as_chunks::<3>()
                .0
                .iter()
                .all(|row| row[2] & 0x1f == 0),
            "padding must be white"
        );
    }
}

#[test]
fn solid_endpoints_and_transparent_pixels_stay_solid_with_every_method() {
    for &method in METHODS {
        for (pixel, expected) in [([0, 0, 0, 255], 0xff), ([255; 4], 0), ([0; 4], 0)] {
            let source = RgbaImage::from_pixel(64, 64, Rgba(pixel));
            let raster = convert(&source, (16, 16), Conversion::Dither(method)).unwrap();
            assert!(
                raster.bytes().iter().all(|b| *b == expected),
                "{method:?} {pixel:?}"
            );
        }
    }
}

#[test]
fn diffusion_preserves_linear_luminance_instead_of_srgb_code_values() {
    let source = RgbaImage::from_pixel(256, 256, Rgba([128, 128, 128, 255]));
    let raster = convert(
        &source,
        (128, 128),
        Conversion::Dither(Dither::FloydSteinberg),
    )
    .unwrap();
    let black: u32 = raster.bytes().iter().map(|b| b.count_ones()).sum();
    // sRGB 128 is ~21.6% linear light, so ~78.4% black coverage.
    assert!((f64::from(black) / 16384.0 - 0.784).abs() < 0.01);
}

#[test]
fn alpha_is_composited_in_linear_light() {
    let source = RgbaImage::from_pixel(128, 128, Rgba([0, 0, 0, 128]));
    let raster = convert(
        &source,
        source.dimensions(),
        Conversion::Dither(Dither::FloydSteinberg),
    )
    .unwrap();
    let black: u32 = raster.bytes().iter().map(|b| b.count_ones()).sum();
    assert!((f64::from(black) / 16384.0 - 128.0 / 255.0).abs() < 0.01);
}

#[test]
fn rejects_empty_images_and_excessive_resizing_work_before_allocating() {
    let source = RgbaImage::from_pixel(8, 8, Rgba([255; 4]));
    let conversion = Conversion::Dither(Dither::Bayer);
    assert!(convert(&RgbaImage::new(0, 0), (8, 8), conversion).is_err());
    assert!(convert(&source, (0, 8), conversion).is_err());
    assert!(convert(&source, (8, u32::MAX), conversion).is_err());
    assert!(convert(&source, (u32::MAX, 8), conversion).is_err());
}
