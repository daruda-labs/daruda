use super::*;

#[test]
fn unknown_values_survive_and_known_values_are_not_duplicated() {
    let options = vec![
        SelectOption::new("", "Inherited"),
        SelectOption::simple("known"),
    ];
    let items = with_saved(&options, "retired-model", "retired-model");
    assert!(items.iter().any(|item| item.value == "retired-model"));
    assert_eq!(
        with_saved(&options, "known", "known")
            .iter()
            .filter(|item| item.value == "known")
            .count(),
        1
    );
    assert_eq!(
        with_saved(&options, "", "")
            .iter()
            .filter(|item| item.value.is_empty())
            .count(),
        1
    );
}

#[test]
fn only_the_file_value_is_labelled_saved() {
    let options = vec![SelectOption::new("", "Inherited")];
    let label = |value: &str| {
        with_saved(&options, value, "from-file")
            .into_iter()
            .find(|item| item.value == value)
            .map(|item| item.label.to_string())
    };
    assert_eq!(
        label("from-file"),
        Some(s::flow::form_saved_choice("from-file"))
    );
    assert_eq!(label("typed"), Some(s::flow::form_custom_value("typed")));
}
