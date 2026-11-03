#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
use std::io::Read;

fn main() {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).unwrap();
    let parse = glint_syntax::parse_source(&text);
    println!("{:#?}", parse.syntax());
    for e in parse.errors() {
        println!("{} {:?} {}", e.code, e.range, e.message);
    }
}
