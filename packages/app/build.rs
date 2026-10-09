//! Generates `crate::surface::strings`: one module per top-level section of
//! `locales/en.yml`, one function per key. `en.yml` is the only list of
//! strings, so a key typo is a compile error and an unused key a dead-code
//! warning. See `src/surface/strings/mod.rs` for the shape of what comes out.

use std::path::Path;

const EN: &str = "locales/en.yml";
/// Read by `rust_i18n::i18n!`, which tracks no file it embeds — without this
/// a translation-only edit would leave the old strings compiled in.
const KO: &str = "locales/ko.yml";
const CUSTOM_DIR: &str = "src/surface/strings/custom";

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS");
    if target_os == "windows" {
        println!("cargo:rerun-if-changed=resources/windows/icon.rc");
        println!("cargo:rerun-if-changed=../../assets/icon.ico");
        embed_resource::compile_for(
            "resources/windows/icon.rc",
            ["daruda"],
            embed_resource::NONE,
        )
        .manifest_required()
        .expect("embed Windows application icon");
    }
    println!("cargo:rerun-if-changed={EN}");
    println!("cargo:rerun-if-changed={KO}");
    println!("cargo:rerun-if-changed={CUSTOM_DIR}");
    let en = std::fs::read_to_string(EN).expect("read locales/en.yml");
    let out =
        strings_gen::generate(&en, Path::new(CUSTOM_DIR)).unwrap_or_else(|e| panic!("{EN}: {e}"));
    let dest = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("strings.rs");
    std::fs::write(dest, out).expect("write generated strings");
}
