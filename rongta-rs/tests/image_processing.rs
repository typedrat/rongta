use image::{Rgba, RgbaImage};
use rongta::{
    configuration::Media,
    image_processing::{Layout, prepare},
};

#[test]
fn full_label_is_cropped_to_printable_area_then_padded_to_feed_height() {
    let mut source = RgbaImage::from_pixel(400, 240, Rgba([255; 4]));
    // Marks at the printable boundary survive; physical margins do not.
    for (x, y) in [(0, 0), (7, 120), (8, 8), (391, 231), (392, 120), (200, 239)] {
        source.put_pixel(x, y, Rgba([0, 0, 0, 255]));
    }
    let raster = prepare(
        &source,
        &"gap:50x30".parse().unwrap(),
        384,
        Layout::Label,
        None,
        208,
    )
    .unwrap();
    assert_eq!((raster.width(), raster.height()), (384, 240));
    assert_eq!(
        raster.bytes().iter().map(|b| b.count_ones()).sum::<u32>(),
        2
    );
    assert_eq!(raster.bytes()[8 * 48], 0x80);
    assert_eq!(raster.bytes()[231 * 48 + 47], 1);
}

#[test]
fn preset_dimensions_and_white_margins_are_independent_of_threshold() {
    let source = RgbaImage::from_pixel(40, 30, Rgba([0, 0, 0, 255]));
    for (spec, height, width) in [
        ("gap:50x80", 640, 384),
        ("gap:50x50", 400, 384),
        ("gap:50x30", 240, 384),
        ("gap:40x60", 480, 320),
        ("gap:40x40", 320, 320),
        ("gap:40x30", 240, 320),
        ("gap:30x30", 240, 240),
    ] {
        let raster = prepare(
            &source,
            &spec.parse().unwrap(),
            384,
            Layout::Label,
            None,
            255,
        )
        .unwrap();
        assert_eq!(raster.height(), height);
        assert!(raster.bytes()[..8 * 48].iter().all(|b| *b == 0));
        assert_eq!(
            raster.bytes().iter().map(|b| b.count_ones()).sum::<u32>(),
            width * (height - 16)
        );
    }
}

#[test]
fn continuous_length_can_be_derived_or_explicit_and_invalid_lengths_fail() {
    let source = RgbaImage::from_pixel(400, 200, Rgba([0, 0, 0, 255]));
    let media: Media = "continuous:50".parse().unwrap();
    assert_eq!(
        prepare(&source, &media, 384, Layout::Label, None, 208)
            .unwrap()
            .height(),
        200
    );
    assert_eq!(
        prepare(&source, &media, 384, Layout::Label, Some(40.0), 208)
            .unwrap()
            .height(),
        320
    );
    assert!(prepare(&source, &media, 384, Layout::Label, Some(f64::NAN), 208).is_err());
}

#[test]
fn fit_preserves_edges_and_circle_masks_corners() {
    let source = RgbaImage::from_pixel(40, 20, Rgba([0, 0, 0, 255]));
    let raster = prepare(
        &source,
        &"gap:40x30".parse().unwrap(),
        384,
        Layout::Fit,
        None,
        208,
    )
    .unwrap();
    let preview = rongta::image_processing::preview(&raster);
    assert_eq!(preview.get_pixel(32, 40).0, [0]);
    assert_eq!(preview.get_pixel(351, 199).0, [0]);
    assert_eq!(preview.get_pixel(32, 39).0, [255]);
    let circle = prepare(
        &source,
        &"circle:40".parse().unwrap(),
        384,
        Layout::Label,
        None,
        255,
    )
    .unwrap();
    let preview = rongta::image_processing::preview(&circle);
    assert_eq!(preview.get_pixel(32, 8).0, [255]);
    assert_eq!(preview.get_pixel(192, 160).0, [0]);
}
