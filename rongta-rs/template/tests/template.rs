use rongta_template::{Case, Template, Values};

const EXAMPLE: &str = include_str!("../examples/access-code.rongta.toml");

fn values(pairs: &[(&str, &str)]) -> Values {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn example_template_parses() {
    let template = Template::parse(EXAMPLE).unwrap();
    assert_eq!(template.name, "Access code");
    assert_eq!(template.media.to_string(), "gap:50x25");
    assert_eq!(template.fields.len(), 1);
    assert_eq!(template.fields[0].id, "code");
    assert_eq!(template.fields[0].case, Case::Upper);
    assert_eq!(template.printable_size(), (384, 184));
}

#[test]
fn values_are_trimmed_cased_and_checked_against_the_whole_pattern() {
    let template = Template::parse(EXAMPLE).unwrap();
    let normalized = template.normalize(&values(&[("code", "  ab12cd ")]));
    assert_eq!(normalized["code"], "AB12CD");
    assert!(template.check(&normalized).is_empty());

    let missing = template.check(&Values::new());
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].to_string(), "code: Required");

    // The pattern must match the whole value, not a substring.
    let long = template.normalize(&values(&[("code", "AB12CD-EXTRA")]));
    assert_eq!(
        template.check(&long)[0].message,
        "6 to 12 letters or digits"
    );
}

#[test]
fn invalid_templates_explain_the_problem() {
    for (source, needle) in [
        (EXAMPLE.replace("version = 1", "version = 2"), "version"),
        (EXAMPLE.replace("gap:50x25", "circle:40"), "gap"),
        (EXAMPLE.replace("code={code}", "code={coed}"), "coed"),
        (EXAMPLE.replace("[A-Z0-9]{6,12}", "[A-Z"), "pattern"),
        (
            EXAMPLE.replace("size_mm = 3.0", "size_mm = 40.0"),
            "size_mm",
        ),
        (format!("{EXAMPLE}\nsurprise = true\n"), "surprise"),
    ] {
        let error = format!("{:#}", Template::parse(&source).unwrap_err());
        assert!(error.contains(needle), "{needle:?} not in {error:?}");
    }
}

fn decode_qr(image: &image::RgbaImage) -> String {
    let gray = image::DynamicImage::ImageRgba8(image.clone()).into_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(gray);
    let grids = prepared.detect_grids();
    assert_eq!(grids.len(), 1, "expected exactly one QR code");
    grids[0].decode().unwrap().1
}

#[test]
fn rendered_label_has_a_scannable_qr_with_the_encoded_value() {
    let template = Template::parse(EXAMPLE).unwrap();
    let values = template.normalize(&values(&[("code", "ab12cd")]));
    let image = template.render(&values).unwrap();
    assert_eq!(image.dimensions(), (384, 184));
    assert!(
        image
            .pixels()
            .all(|p| p.0 == [0, 0, 0, 255] || p.0 == [255; 4])
    );
    assert_eq!(decode_qr(&image), "https://example.com/redeem?code=AB12CD");
    // The text column holds ink; it lies left of the QR's 148-dot square.
    let text_ink = image
        .enumerate_pixels()
        .filter(|(x, _, p)| *x < 384 - 148 && p.0[0] == 0)
        .count();
    assert!(text_ink > 200, "{text_ink}");
}

#[test]
fn qr_values_are_percent_encoded_but_text_is_not() {
    let source = EXAMPLE
        .replace("pattern = \"[A-Z0-9]{6,12}\"\n", "")
        .replace("case = \"upper\"\n", "");
    let template = Template::parse(&source).unwrap();
    let image = template.render(&values(&[("code", "a b&c")])).unwrap();
    assert_eq!(
        decode_qr(&image),
        "https://example.com/redeem?code=a%20b%26c"
    );
}

#[test]
fn raster_fills_the_label_canvas() {
    let template = Template::parse(EXAMPLE).unwrap();
    let raster = template
        .raster(&values(&[("code", "AB12CD")]), 384)
        .unwrap();
    assert_eq!((raster.width(), raster.height()), (384, 200));
}

#[test]
fn render_refuses_invalid_values_and_oversized_content() {
    let template = Template::parse(EXAMPLE).unwrap();
    let error = template.render(&Values::new()).unwrap_err().to_string();
    assert!(error.contains("code: Required"), "{error}");

    let long_url = EXAMPLE.replace(
        "https://example.com/redeem?code={code}",
        &format!("https://example.com/{}?code={{code}}", "x".repeat(300)),
    );
    let error = Template::parse(&long_url)
        .unwrap()
        .render(&values(&[("code", "AB12CD")]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("QR"), "{error}");
}
