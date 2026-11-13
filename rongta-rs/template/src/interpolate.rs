//! `{field}` and `{field.group}` placeholders; `{{` and `}}` are literal
//! braces.
use anyhow::{Result, bail};

#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Literal(String),
    Field(Placeholder),
}

/// A field's whole value, or one named group of its pattern's match.
#[derive(Clone, Debug, PartialEq)]
pub struct Placeholder {
    pub field: String,
    pub group: Option<String>,
}

impl std::fmt::Display for Placeholder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.group {
            Some(group) => write!(f, "{{{}.{group}}}", self.field),
            None => write!(f, "{{{}}}", self.field),
        }
    }
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
                        Some(c)
                            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' =>
                        {
                            name.push(c)
                        }
                        Some(c) => bail!("Invalid character {c:?} in a placeholder in {source:?}"),
                        None => bail!("Unclosed placeholder in {source:?}"),
                    }
                }
                let placeholder = match name.split('.').collect::<Vec<_>>()[..] {
                    [field] if !field.is_empty() => Placeholder {
                        field: field.to_owned(),
                        group: None,
                    },
                    [field, group] if !field.is_empty() && !group.is_empty() => Placeholder {
                        field: field.to_owned(),
                        group: Some(group.to_owned()),
                    },
                    _ => bail!("Invalid placeholder {{{name}}} in {source:?}"),
                };
                if !literal.is_empty() {
                    pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                }
                pieces.push(Piece::Field(placeholder));
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

pub fn fill(pieces: &[Piece], value: impl Fn(&Placeholder) -> String) -> String {
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
        let field = |field: &str, group: Option<&str>| {
            Field(Placeholder {
                field: field.into(),
                group: group.map(Into::into),
            })
        };
        assert_eq!(
            parse("a{code}b{{c}}{code.g1}").unwrap(),
            vec![
                Literal("a".into()),
                field("code", None),
                Literal("b{c}".into()),
                field("code", Some("g1")),
            ]
        );
        let pieces = parse("x={code}&y={code.g1}").unwrap();
        assert_eq!(
            fill(&pieces, |p| p.to_string().to_uppercase()),
            "x={CODE}&y={CODE.G1}"
        );
    }

    #[test]
    fn malformed_placeholders_are_rejected() {
        for source in [
            "{", "{}", "}", "{a b}", "{a", "{.a}", "{a.}", "{a.b.c}", "{.}",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
    }
}
