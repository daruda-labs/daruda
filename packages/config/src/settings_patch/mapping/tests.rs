use super::super::fields::field_value;
use crate::{Config, SettingsPatch};

trait ChangedSample {
    fn changed_sample(&self) -> Self;
}

impl ChangedSample for crate::StatusIconTheme {
    fn changed_sample(&self) -> Self {
        match self {
            Self::Dots => Self::Icons,
            Self::Icons | Self::Custom(_) => Self::Dots,
        }
    }
}

impl ChangedSample for bool {
    fn changed_sample(&self) -> Self {
        !self
    }
}

impl ChangedSample for String {
    fn changed_sample(&self) -> Self {
        format!("{self}-changed")
    }
}

macro_rules! numeric_samples {
    ($($kind:ty),*) => {
        $(impl ChangedSample for $kind {
            fn changed_sample(&self) -> Self {
                *self + 1 as $kind
            }
        })*
    };
}

numeric_samples!(f32, f64, u8, u32, u64, usize);

macro_rules! check_snapshot_boundaries {
    ($($variant:ident: $($member:ident).+, $path:literal, $ownership:ident, $serialize:expr;)*) => {
        #[test]
        fn ordinary_patches_ignore_unrelated_snapshot_changes() {
            let original = Config::default();
            $(
                let changed = original.$($member).+.changed_sample();
                assert_ne!(changed, original.$($member).+, "nondefault sample: {}", $path);
                let patch = SettingsPatch::$variant(field_value!($ownership, &changed));
                let mut other = original.clone();
                other.shell.restore_output = !original.shell.restore_output;
                assert!(!patch.field_changed_between(&original, &other), "{}", $path);
                patch.apply_to(&mut other);
                assert_eq!(other.$($member).+, changed, "addressed write: {}", $path);
                assert!(patch.field_changed_between(&original, &other), "changed field: {}", $path);
                assert_ne!(other.shell.restore_output, original.shell.restore_output, "{}", $path);
            )*
        }
    };
}

super::super::fields::with_settings_fields!(check_snapshot_boundaries);
