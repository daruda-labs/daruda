use super::super::fields::field_value;
use crate::{Config, SettingsPatch};

macro_rules! check_snapshot_boundaries {
    ($($variant:ident: $($member:ident).+, $path:literal, $ownership:ident, $serialize:expr;)*) => {
        #[test]
        fn ordinary_patches_ignore_unrelated_snapshot_changes() {
            let original = Config::default();
            $(
                let patch = SettingsPatch::$variant(field_value!($ownership, &original.$($member).+));
                let mut other = original.clone();
                other.shell.restore_output = !original.shell.restore_output;
                assert!(!patch.field_changed_between(&original, &other), "{}", $path);
                patch.apply_to(&mut other);
                assert_ne!(other.shell.restore_output, original.shell.restore_output, "{}", $path);
            )*
        }
    };
}

super::super::fields::with_settings_fields!(check_snapshot_boundaries);
