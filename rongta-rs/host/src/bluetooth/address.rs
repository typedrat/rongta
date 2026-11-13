use std::{fmt, str::FromStr};

use anyhow::{Context, Error, Result};

/// Bluetooth Classic device address, most significant byte first.
///
/// Displays and serializes as `AA:BB:CC:DD:EE:FF`. Parsing also accepts the
/// `aa-bb-cc-dd-ee-ff` form used by macOS and Windows tools.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address(pub [u8; 6]);

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02X}:{b:02X}:{c:02X}:{d:02X}:{e:02X}:{g:02X}")
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for Address {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let separator = if s.contains('-') { '-' } else { ':' };
        let octets = s
            .split(separator)
            .map(|octet| {
                anyhow::ensure!((1..=2).contains(&octet.len()), "bad octet");
                Ok(u8::from_str_radix(octet, 16)?)
            })
            .collect::<Result<Vec<_>>>()
            .ok()
            .and_then(|octets| <[u8; 6]>::try_from(octets).ok())
            .with_context(|| format!("invalid Bluetooth address: {s}"))?;
        Ok(Self(octets))
    }
}

impl serde::Serialize for Address {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Address {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::Address;

    #[test]
    fn parses_colon_and_dash_forms() {
        let expected = Address([0xAA, 0xBB, 0x0C, 0xDD, 0xEE, 0xFF]);
        assert_eq!("AA:BB:0C:DD:EE:FF".parse::<Address>().unwrap(), expected);
        assert_eq!("aa-bb-c-dd-ee-ff".parse::<Address>().unwrap(), expected);
        assert_eq!(expected.to_string(), "AA:BB:0C:DD:EE:FF");
    }

    #[test]
    fn rejects_malformed_addresses() {
        for bad in [
            "",
            "AA:BB:CC:DD:EE",
            "AA:BB:CC:DD:EE:FF:00",
            "AA:BB:CC:DD:EE:GG",
            "AA:BB:CC-DD:EE:FF",
            "AAA:BB:CC:DD:EE:FF",
            "AA::CC:DD:EE:FF",
        ] {
            assert!(bad.parse::<Address>().is_err(), "{bad:?}");
        }
    }
}
