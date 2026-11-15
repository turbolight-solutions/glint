#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

//! Decoding arbitrary bytes never panics, and every byte belongs to a TLV (Codegen and Decoder Design, Testing).

use glint_decode::{NoOracle, Tlv, decode};
use glint_ir::{
    Collection, Component, IrSet, Kind, Origin, Presence, Record, Type, TypeDef, TypeId,
};
use proptest::prelude::*;

/// `T ::= SEQUENCE { a INTEGER, b OCTET STRING OPTIONAL, c SEQUENCE OF T }`, recursive.
fn schema() -> IrSet {
    let component = |name: &str, kind: Kind, presence: Presence| Component {
        name: name.to_owned(),
        presence,
        region: Default::default(),
        group: None,
        version: None,
        ty: Type::of(kind),
    };
    IrSet {
        version: glint_ir::IR_VERSION,
        root: TypeId(0),
        types: vec![TypeDef {
            name: "T".to_owned(),
            origin: Origin {
                module: "M".to_owned(),
                module_oid: None,
                document: None,
                asn1_name: "T".to_owned(),
                actuals: Vec::new(),
            },
            ty: Type::of(Kind::Sequence(Record {
                components: vec![
                    component("a", Kind::Integer { named: Vec::new() }, Presence::Required),
                    component("b", Kind::OctetString, Presence::Optional),
                    component(
                        "c",
                        Kind::SequenceOf(Collection {
                            element_name: None,
                            element: Box::new(Type::of(Kind::Reference { target: TypeId(0) })),
                        }),
                        Presence::Required,
                    ),
                ],
                extensible: false,
            })),
        }],
        object_sets: Vec::new(),
        oids: Vec::new(),
        notes: Vec::new(),
    }
}

/// The children of `tlv` tile its contents, and so on down.
fn check_tiling(tlv: &Tlv, bytes: &[u8]) -> Result<(), String> {
    if tlv.end > bytes.len() || tlv.offset + tlv.header_len > tlv.end {
        return Err(format!("TLV at {} runs past its end", tlv.offset));
    }
    if tlv.children.is_empty() {
        return Ok(());
    }
    let mut pos = tlv.offset + tlv.header_len;
    for child in &tlv.children {
        if child.offset != pos {
            return Err(format!(
                "child of {} at {}, expected {pos}",
                tlv.offset, child.offset
            ));
        }
        check_tiling(child, bytes)?;
        pos = child.end;
    }
    // the end-of-contents octets of an indefinite length follow the last child
    if pos != tlv.end && pos + 2 != tlv.end {
        return Err(format!(
            "children of {} end at {pos}, not {}",
            tlv.offset, tlv.end
        ));
    }
    Ok(())
}

fn check(bytes: &[u8], with_schema: bool) -> Result<(), String> {
    let set = schema();
    let decoded = decode(bytes, with_schema.then_some((&set, set.root)), &NoOracle);
    let mut pos = 0;
    for tlv in &decoded.tlvs {
        if tlv.offset != pos {
            return Err(format!("top-level TLV at {}, expected {pos}", tlv.offset));
        }
        check_tiling(tlv, bytes)?;
        pos = tlv.end;
    }
    if pos != bytes.len() {
        return Err(format!("TLVs end at {pos}, input at {}", bytes.len()));
    }
    Ok(())
}

/// Bytes that look like BER often enough to reach deep paths: small tags, short lengths, nesting.
fn berish() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop_oneof![
            Just(0x30u8),
            Just(0x31),
            Just(0x02),
            Just(0x04),
            Just(0x24),
            Just(0x80),
            Just(0x00),
            Just(0xa0),
            any::<u8>(),
        ],
        0..64,
    )
}

proptest! {
    #[test]
    fn random_bytes_are_tiled_without_a_schema(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        check(&bytes, false).map_err(TestCaseError::fail)?;
    }

    #[test]
    fn berish_bytes_are_tiled_with_and_without_a_schema(bytes in berish()) {
        check(&bytes, false).map_err(TestCaseError::fail)?;
        check(&bytes, true).map_err(TestCaseError::fail)?;
    }
}

#[test]
fn deep_nesting_stops() {
    let mut bytes = Vec::new();
    for _ in 0..200 {
        bytes.extend([0x30, 0x80]);
    }
    check(&bytes, false).unwrap();
    check(&bytes, true).unwrap();
}
