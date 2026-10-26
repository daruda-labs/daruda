use super::check::allowed;
use super::{EventKind, LifecycleKind};

#[test]
fn import_and_create_never_act_on_an_existing_record() {
    for from in enum_iterator::all::<LifecycleKind>() {
        assert!(!allowed(from, EventKind::Imported));
        assert!(!allowed(from, EventKind::Created));
    }
}
