//! Generates `crate::strings` from `locales/en.yml` — see `strings_gen`.

use std::path::Path;

const EN: &str = "locales/en.yml";
/// Read by `rust_i18n::i18n!`, which tracks no file it embeds.
const KO: &str = "locales/ko.yml";
/// A string that needs logic, named like its key, replaces the generated one.
const CUSTOM_DIR: &str = "src/strings/custom";

fn main() {
    println!("cargo:rerun-if-changed={EN}");
    println!("cargo:rerun-if-changed={KO}");
    println!("cargo:rerun-if-changed={CUSTOM_DIR}");
    let en = std::fs::read_to_string(EN).expect("read locales/en.yml");
    let out =
        strings_gen::generate(&en, Path::new(CUSTOM_DIR)).unwrap_or_else(|e| panic!("{EN}: {e}"));
    let dest = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("strings.rs");
    std::fs::write(dest, out).expect("write generated strings");
}
