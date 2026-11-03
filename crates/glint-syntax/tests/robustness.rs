mod common;
use common::*;
use proptest::prelude::*;

const SEED: &str = include_str!("fixtures/rfc5280/PKIX1Explicit88.asn");

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn arbitrary_text_never_panics_and_stays_lossless(src in ".{0,400}") {
        let _ = parse(&src);
        let _ = parse_fragment(&src);
    }

    #[test]
    fn asn1_flavoured_text_never_panics(src in r"[A-Za-z0-9 \n{}()\[\]:=,.|^!&@<>'\x22-]{0,300}") {
        let _ = parse(&src);
    }

    #[test]
    fn truncating_a_real_module_never_panics(cut in 0..SEED.len()) {
        if let Some(prefix) = SEED.get(..cut) {
            let _ = parse(prefix);
        }
    }

    #[test]
    fn deleting_a_span_from_a_real_module_never_panics(start in 0..SEED.len(), len in 1usize..200) {
        let end = start.saturating_add(len).min(SEED.len());
        if SEED.is_char_boundary(start) && SEED.is_char_boundary(end) {
            let mut mutated = String::from(&SEED[..start]);
            mutated.push_str(&SEED[end..]);
            let _ = parse(&mutated);
        }
    }
}

#[test]
fn one_megabyte_of_input_parses_in_bounded_time() {
    let mut text = String::with_capacity(1_100_000);
    while text.len() < 1_000_000 {
        text.push_str(SEED);
    }
    let started = std::time::Instant::now();
    let parse = parse(&text);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "parsing took {:?}",
        started.elapsed()
    );
    assert!(!parse.errors().is_empty());
}

#[test]
fn one_megabyte_of_garbage_parses_in_bounded_time() {
    let text = "{ ( [ SEQUENCE a ::= ".repeat(50_000);
    let started = std::time::Instant::now();
    let _ = parse(&text);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "parsing took {:?}",
        started.elapsed()
    );
}
