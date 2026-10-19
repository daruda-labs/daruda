use super::*;
use crate::runs::seed::{self, SeedEnd, SeedRun};
use gpui::TestAppContext;

#[gpui::test]
fn malformed_missing_alias_and_valid_siblings_are_independent(cx: &mut TestAppContext) {
    let query = cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        let run = SeedRun::chat("/work");
        let task = seed::task(
            seed::spec(Default::default(), "valid"),
            Some((&run, SeedEnd::Done)),
            cx,
        );
        prepare(
            Some(vec![
                "flow:../bad".into(),
                "task:missing".into(),
                format!("task:{task}"),
            ]),
            cx,
        )
    });
    let result = query.read();
    let content = &result["structuredContent"];
    assert_eq!(
        content["errors"],
        serde_json::json!([{"code":"unknown_handle","handle":"flow:../bad"}])
    );
    assert_eq!(content["runs"][0]["state"]["state"], "gone");
    assert_eq!(content["runs"][1]["state"]["ended_as"], "completed");
    assert!(content["runs"][1]["handle"].as_str().unwrap().contains('/'));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(result["content"][0]["text"].as_str().unwrap())
            .unwrap(),
        *content
    );
    assert_eq!(result["isError"], false);
}

#[gpui::test]
fn status_scope_is_only_registered_starts_and_queries_have_no_reporting_effect(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        let run = SeedRun::chat("/work");
        let task = seed::task(
            seed::spec(Default::default(), "task"),
            Some((&run, SeedEnd::Done)),
            cx,
        );
        assert_eq!(
            prepare(None, cx).read()["structuredContent"]["runs"],
            serde_json::json!([])
        );
        RunsHost::register_run(domain::RunKey::Task(run.exec(&task)), cx);
        let first = prepare(None, cx).read();
        assert_eq!(first, prepare(None, cx).read());
        assert_eq!(
            first["structuredContent"]["runs"].as_array().unwrap().len(),
            1
        );
    });
}
