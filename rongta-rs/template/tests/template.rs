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
