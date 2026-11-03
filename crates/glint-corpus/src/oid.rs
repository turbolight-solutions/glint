use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// An object identifier as a list of numeric arcs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Oid(Vec<u128>);

/// A dotted object identifier that could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a dotted object identifier")]
pub struct OidParseError(String);

impl Oid {
    pub fn new(arcs: Vec<u128>) -> Self {
        Self(arcs)
    }

    pub fn arcs(&self) -> &[u128] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn child(&self, arc: u128) -> Self {
        let mut arcs = self.0.clone();
        arcs.push(arc);
        Self(arcs)
    }

    pub fn parent(&self) -> Option<Self> {
        let (_, rest) = self.0.split_last()?;
        Some(Self(rest.to_vec()))
    }

    /// Whether `self` is `other` or an ancestor of it.
    pub fn is_prefix_of(&self, other: &Self) -> bool {
        other.0.starts_with(&self.0)
    }

    pub fn last(&self) -> Option<u128> {
        self.0.last().copied()
    }

    /// `self` with its last arc replaced.
    pub fn with_last(&self, arc: u128) -> Self {
        let mut arcs = self.0.clone();
        if let Some(slot) = arcs.last_mut() {
            *slot = arc;
        }
        Self(arcs)
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, arc) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(".")?;
            }
            write!(f, "{arc}")?;
        }
        Ok(())
    }
}

impl FromStr for Oid {
    type Err = OidParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || OidParseError(text.to_owned());
        let arcs = text
            .trim()
            .split('.')
            .map(|part| part.parse::<u128>().map_err(|_| error()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self(arcs))
    }
}

impl From<Oid> for String {
    fn from(oid: Oid) -> Self {
        oid.to_string()
    }
}

impl TryFrom<String> for Oid {
    type Error = OidParseError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::Oid;

    #[test]
    fn dotted_form_round_trips() {
        let oid: Oid = "1.3.6.1.5.5.7".parse().unwrap_or_default_for_test();
        assert_eq!(oid.to_string(), "1.3.6.1.5.5.7");
        assert_eq!(oid.len(), 7);
    }

    #[test]
    fn prefix_and_parent() {
        let a: Oid = "1.3.6".parse().unwrap_or_default_for_test();
        let b: Oid = "1.3.6.1".parse().unwrap_or_default_for_test();
        assert!(a.is_prefix_of(&b));
        assert!(!b.is_prefix_of(&a));
        assert_eq!(b.parent(), Some(a));
    }

    #[test]
    fn rejects_garbage() {
        assert!("1.x".parse::<Oid>().is_err());
        assert!("".parse::<Oid>().is_err());
    }

    trait UnwrapForTest<T> {
        fn unwrap_or_default_for_test(self) -> T;
    }

    impl<T, E: std::fmt::Debug> UnwrapForTest<T> for Result<T, E> {
        fn unwrap_or_default_for_test(self) -> T {
            self.unwrap()
        }
    }
}
