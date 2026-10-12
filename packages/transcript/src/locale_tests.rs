const EN: &str = include_str!("../locales/en.yml");
const KO: &str = include_str!("../locales/ko.yml");

#[test]
fn locale_en_ko_key_parity() {
    strings_gen::check_key_parity(EN, KO).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn locale_en_ko_placeholder_parity() {
    strings_gen::check_placeholder_parity(EN, KO).unwrap_or_else(|e| panic!("{e}"));
}
