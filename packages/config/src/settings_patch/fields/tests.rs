use crate::SettingsFieldId;

macro_rules! check_field_paths {
    ($($variant:ident: $($member:ident).+, $path:literal, $ownership:ident, $serialize:expr;)*) => {
        #[test]
        fn ordinary_fields_address_distinct_keys() {
            let mut paths = std::collections::HashSet::new();
            $(
                let path = SettingsFieldId::$variant.path();
                assert!(paths.insert(path), "duplicate persistence boundary: {path}");
                assert!(path.rsplit_once('.').is_some(), "missing section: {path}");
            )*
        }
    };
}

super::with_settings_fields!(check_field_paths);
