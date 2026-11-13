#![allow(dead_code)]
//! `{field}` placeholders; `{{` and `}}` are literal braces.
use anyhow::{Result, bail};

#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Literal(String),
    Field(String),
}

pub fn parse(source: &str) -> Result<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-' => {
                            name.push(c)
                        }
                        Some(c) => bail!("Invalid character {c:?} in a placeholder in {source:?}"),
                        None => bail!("Unclosed placeholder in {source:?}"),
                    }
                }
                if name.is_empty() {
                    bail!("Empty placeholder in {source:?}");
                }
                if !literal.is_empty() {
                    pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                }
                pieces.push(Piece::Field(name));
            }
            '}' => bail!("Unmatched }} in {source:?}; write }}}} for a literal brace"),
            c => literal.push(c),
        }
    }
    if !literal.is_empty() {
        pieces.push(Piece::Literal(literal));
    }
    Ok(pieces)
}

pub fn fill(pieces: &[Piece], value: impl Fn(&str) -> String) -> String {
    pieces
        .iter()
        .map(|piece| match piece {
            Piece::Literal(text) => text.clone(),
            Piece::Field(name) => value(name),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Piece::*, *};

    #[test]
    fn literals_fields_and_escaped_braces() {
        assert_eq!(
            parse("a{code}b{{c}}").unwrap(),
            vec![
                Literal("a".into()),
                Field("code".into()),
                Literal("b{c}".into())
            ]
        );
        let pieces = parse("x={code}&y={code}").unwrap();
        assert_eq!(fill(&pieces, |f| f.to_uppercase()), "x=CODE&y=CODE");
    }

    #[test]
    fn malformed_placeholders_are_rejected() {
        for source in ["{", "{}", "}", "{a b}", "{a"] {
            assert!(parse(source).is_err(), "{source}");
        }
    }
}
