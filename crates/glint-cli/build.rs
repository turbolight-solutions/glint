//! Embeds a corpus bundle into the binary when one is available at build time.
//!
//! The bundle comes from `$GLINT_EMBED_CORPUS`, or `corpus/embedded.bundle` at the workspace root.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=GLINT_EMBED_CORPUS");
    println!("cargo:rustc-check-cfg=cfg(embedded_corpus)");
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let default = manifest.join("../../corpus/embedded.bundle");
    println!("cargo:rerun-if-changed={}", default.display());
    let chosen = std::env::var_os("GLINT_EMBED_CORPUS")
        .map(PathBuf::from)
        .or_else(|| default.exists().then_some(default));
    let Some(path) = chosen.and_then(|p| p.canonicalize().ok()) else {
        return;
    };
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rustc-cfg=embedded_corpus");
    println!(
        "cargo:rustc-env=GLINT_EMBEDDED_CORPUS_PATH={}",
        path.display()
    );
}
