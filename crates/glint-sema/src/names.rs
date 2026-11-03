//! The identifiers that X.660 Annex A allows as an object identifier `NameForm` (X.680 32.7, 32.8).

use glint_corpus::Oid;

// X.660 A.2.2, A.2.4
const ROOT_NAMES: &[(&str, u128)] = &[
    ("itu-t", 0),
    ("ccitt", 0),
    ("iso", 1),
    ("joint-iso-itu-t", 2),
    ("joint-iso-ccitt", 2),
];

// X.660 A.3.2: only the first five secondary identifiers may be used as a NameForm
const ITU_T_NAMES: &[(&str, u128)] = &[
    ("recommendation", 0),
    ("question", 1),
    ("administration", 2),
    ("network-operator", 3),
    ("identified-organization", 4),
];

// X.660 A.4.2
const ISO_NAMES: &[(&str, u128)] = &[
    ("standard", 0),
    ("registration-authority", 1),
    ("member-body", 2),
    ("identified-organization", 3),
];

/// The numeric value of `name` used as a bare `NameForm` directly under `parent`, if X.660 registers it there.
pub fn registered_arc(parent: &[u128], name: &str) -> Option<u128> {
    let table: &[(&str, u128)] = match parent {
        [] => ROOT_NAMES,
        [0] => ITU_T_NAMES,
        [1] => ISO_NAMES,
        [0, 0] => return recommendation_series(name),
        _ => return None,
    };
    table
        .iter()
        .find_map(|&(candidate, arc)| (candidate == name).then_some(arc))
}

// X.660 A.3.3.1: secondary identifiers a to z are the arcs 1 to 26
fn recommendation_series(name: &str) -> Option<u128> {
    let mut chars = name.chars();
    let letter = chars.next()?;
    if chars.next().is_some() || !letter.is_ascii_lowercase() {
        return None;
    }
    Some(u128::from(
        u32::from(letter)
            .saturating_sub(u32::from('a'))
            .saturating_add(1),
    ))
}

/// Whether `name` is an X.660 identifier at any position (used to tell a `NameForm` from a value reference).
pub fn is_registered_anywhere(name: &str) -> bool {
    ROOT_NAMES
        .iter()
        .chain(ITU_T_NAMES)
        .chain(ISO_NAMES)
        .any(|&(candidate, _)| candidate == name)
        || recommendation_series(name).is_some()
}

/// Why an object identifier value is not valid by itself (X.660 via X.680 32.11).
pub fn validate_arcs(oid: &Oid) -> Option<&'static str> {
    let arcs = oid.arcs();
    if arcs.len() < 2 {
        return Some("an object identifier needs at least two arcs");
    }
    match arcs {
        [first, ..] if *first > 2 => Some("the first arc must be 0, 1 or 2"),
        [first, second, ..] if *first < 2 && *second >= 40 => {
            Some("the second arc must be below 40 under arcs 0 and 1")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_names() {
        assert_eq!(registered_arc(&[], "iso"), Some(1));
        assert_eq!(registered_arc(&[], "joint-iso-ccitt"), Some(2));
        assert_eq!(registered_arc(&[], "dod"), None);
    }

    #[test]
    fn identified_organization_depends_on_the_parent() {
        assert_eq!(registered_arc(&[1], "identified-organization"), Some(3));
        assert_eq!(registered_arc(&[0], "identified-organization"), Some(4));
        assert_eq!(registered_arc(&[2], "identified-organization"), None);
    }

    #[test]
    fn recommendation_letters() {
        assert_eq!(registered_arc(&[0, 0], "a"), Some(1));
        assert_eq!(registered_arc(&[0, 0], "z"), Some(26));
        assert_eq!(registered_arc(&[0, 0], "ab"), None);
    }
}
