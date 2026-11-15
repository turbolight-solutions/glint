//! Names (Codegen and Decoder Design, Naming): each backend's case rules and keywords, and the collision pass the
//! middle layer runs over a whole set. Names are deterministic, so regenerating gives a clean diff.

use std::collections::HashSet;

/// A backend's naming rules.
pub trait Namer: Sync {
    /// A type, from an ASN.1 type reference or a hoisted type's words.
    fn type_name(&self, asn1: &str) -> String;
    /// A record component.
    fn field_name(&self, asn1: &str) -> String;
    /// An alternative, enumeration item or dispatch variant.
    fn variant_name(&self, asn1: &str) -> String;
    /// A named value (an OID constant).
    fn const_name(&self, asn1: &str) -> String;
    /// An output module, from an ASN.1 module name.
    fn module_name(&self, asn1: &str) -> String;
    /// Names no generated type may take: the runtime's and the library's.
    fn reserved_types(&self) -> &[&str] {
        &[]
    }
}

/// The words of an ASN.1 name: split at `-`, `_` and case changes, digits kept with the word before them
/// (`PKIX1Explicit-2009` → `PKIX1`, `Explicit`, `2009`; `uTF8String` → `u`, `TF8`, `String`).
pub fn words(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_ascii_alphanumeric() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        let prev = i.checked_sub(1).and_then(|p| chars.get(p)).copied();
        let next = chars.get(i.saturating_add(1)).copied();
        let boundary = match prev {
            Some(p) if c.is_ascii_uppercase() => {
                p.is_ascii_lowercase()
                    || (p.is_ascii_digit() && !current.chars().all(|x| x.is_ascii_digit()))
                    || (p.is_ascii_uppercase() && next.is_some_and(|n| n.is_ascii_lowercase()))
            }
            Some(p) if c.is_ascii_lowercase() => p.is_ascii_digit() && current.len() > 1,
            _ => false,
        };
        if boundary && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        current.push(c);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn capitalize(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// `UpperCamel`, keeping a word's capitals (`TBSCertificate` stays, `id-ce-keyUsage` → `IdCeKeyUsage`).
pub fn upper_camel(name: &str) -> String {
    words(name)
        .iter()
        .map(|w| {
            if w.chars().any(|c| c.is_ascii_lowercase()) {
                let mut chars = w.chars();
                chars.next().map_or_else(String::new, |f| {
                    f.to_ascii_uppercase().to_string() + chars.as_str()
                })
            } else {
                capitalize(w)
            }
        })
        .collect()
}

/// `snake_case`.
pub fn snake(name: &str) -> String {
    words(name)
        .iter()
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_")
}

/// `SCREAMING_SNAKE_CASE`.
pub fn screaming(name: &str) -> String {
    words(name)
        .iter()
        .map(|w| w.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join("_")
}

/// Hands out names unique within one scope: a second `Name` becomes `Name2`, then `Name3`.
#[derive(Debug, Default)]
pub struct Scope {
    taken: HashSet<String>,
}

impl Scope {
    pub fn new(reserved: &[&str]) -> Self {
        Self {
            taken: reserved.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    pub fn claim(&mut self, name: String) -> String {
        if self.taken.insert(name.clone()) {
            return name;
        }
        for n in 2u32.. {
            let candidate = format!("{name}{n}");
            if self.taken.insert(candidate.clone()) {
                return candidate;
            }
        }
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_at_case_and_punctuation() {
        assert_eq!(words("PKIX1Explicit-2009"), ["PKIX1", "Explicit", "2009"]);
        assert_eq!(words("id-ce-keyUsage"), ["id", "ce", "key", "Usage"]);
        assert_eq!(words("TBSCertificate"), ["TBS", "Certificate"]);
        assert_eq!(words("uTF8String"), ["u", "TF8", "String"]);
        assert_eq!(words("sha256WithRSA"), ["sha256", "With", "RSA"]);
        assert_eq!(words("x121-dcc-code"), ["x121", "dcc", "code"]);
    }

    #[test]
    fn case_conversions() {
        assert_eq!(upper_camel("id-ce-keyUsage"), "IdCeKeyUsage");
        assert_eq!(upper_camel("TBSCertificate"), "TbsCertificate");
        assert_eq!(snake("subjectPublicKeyInfo"), "subject_public_key_info");
        assert_eq!(snake("PKIX1Explicit-2009"), "pkix1_explicit_2009");
        assert_eq!(screaming("id-ce-keyUsage"), "ID_CE_KEY_USAGE");
    }

    #[test]
    fn scopes_number_collisions() {
        let mut scope = Scope::new(&["Any"]);
        assert_eq!(scope.claim("Any".into()), "Any2");
        assert_eq!(scope.claim("Name".into()), "Name");
        assert_eq!(scope.claim("Name".into()), "Name2");
    }
}
