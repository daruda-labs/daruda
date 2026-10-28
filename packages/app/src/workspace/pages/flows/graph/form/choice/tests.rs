use super::*;

#[test]
fn unknown_values_survive_and_known_values_are_not_duplicated() {
    let options = vec![
        SelectOption::new("", "Inherited"),
        SelectOption::simple("known"),
    ];
    let items = with_saved(&options, "retired-model");
    assert!(items.iter().any(|item| item.value == "retired-model"));
    assert_eq!(
        with_saved(&options, "known")
            .iter()
            .filter(|item| item.value == "known")
            .count(),
        1
    );
    assert_eq!(
        with_saved(&options, "")
            .iter()
            .filter(|item| item.value.is_empty())
            .count(),
        1
    );
}
