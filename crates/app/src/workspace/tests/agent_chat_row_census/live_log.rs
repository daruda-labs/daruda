//! Optional diagnostic that replays a captured ACP log through the real mapper
//! and row projection. It does nothing unless `DARUDA_CENSUS_LOG` is set.
//!
//! ```text
//! DARUDA_CENSUS_LOG=~/.daruda/logs/debug/acp-wire-codex-acp.log \
//!   DARUDA_CENSUS_AGENT=codex-acp \
//!   cargo test -p daruda --bin daruda workspace::tests::agent_chat_row_census \
//!   -- --nocapture
//! ```

use daruda_acp::{ChatItem, SessionUpdate, adapter::adapter_for, mapping};

use super::{Lens, per_turn_as_last, per_turn_settled, turn_bounds};
use crate::workspace::main_area::agent_chat_pane::display_filter::DisplayFilter;
use crate::workspace::main_area::agent_chat_pane::fold_mode::FoldPreset;
use crate::workspace::main_area::agent_chat_pane::rows::tail::TailWindow;

const TAIL_WINDOWS: [TailWindow; 2] = [TailWindow::All, TailWindow::Last(5)];

const FILTERS: [(&str, &[&str]); 3] = [
    ("none", &[]),
    ("tools only", &["tools"]),
    ("edits only", &["tools", "tool_edit"]),
];

const SAMPLED_STEPS: usize = 3;

/// The `agent_info.name` from the capture's `initialize` response, if it has
/// one. `None` for a capture whose handshake was not recorded.
fn reported_program(log: &str) -> Option<String> {
    log.lines().find_map(|line| {
        let brace = line.find('{')?;
        let v = serde_json::from_str::<serde_json::Value>(&line[brace..]).ok()?;
        v.get("result")?
            .get("agentInfo")?
            .get("name")?
            .as_str()
            .map(str::to_owned)
    })
}

/// Replay prompts and updates into the item list a live pane would hold.
///
/// The strategy is selected the way production selects it — from the program
/// the log's own `initialize` reports — so a capture exercises the same dialect
/// a live pane would read it with, not just the catalog-id fallback.
fn replay(log: &str, agent_id: &str) -> Vec<ChatItem> {
    let adapter = adapter_for(reported_program(log).as_deref(), agent_id);
    let mut items: Vec<ChatItem> = Vec::new();
    for line in log.lines() {
        let Some(brace) = line.find('{') else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line[brace..]) else {
            continue;
        };
        match v.get("method").and_then(|m| m.as_str()) {
            Some("session/prompt") => {
                mapping::finalize_streaming(&mut items);
                items.push(ChatItem::UserText("<prompt>".into()));
            }
            Some("session/update") => {
                let Some(update) = v.pointer("/params/update") else {
                    continue;
                };
                if let Ok(su) = serde_json::from_value::<SessionUpdate>(update.clone()) {
                    mapping::apply_update_with(&mut items, &su, adapter.as_ref());
                }
            }
            _ => {}
        }
    }
    mapping::finalize_streaming(&mut items);
    items
}

fn item_label(it: &ChatItem) -> &'static str {
    match it {
        ChatItem::UserText(_) => "UserText",
        ChatItem::AssistantText { .. } => "AssistantText",
        ChatItem::Thinking { .. } => "Thinking",
        ChatItem::ToolCall(_) => "ToolCall",
        ChatItem::Permission(_) => "Permission",
        _ => "Other",
    }
}

fn skeleton(items: &[ChatItem]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let label = item_label(&items[i]);
        let mut n = 0;
        while i < items.len() && item_label(&items[i]) == label {
            n += 1;
            i += 1;
        }
        out.push(format!("{label}x{n}"));
    }
    out.join(" ")
}

#[test]
fn census() {
    let Some(path) = std::env::var_os("DARUDA_CENSUS_LOG") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    let agent_id = std::env::var("DARUDA_CENSUS_AGENT").unwrap_or_else(|_| "claude".into());
    let log = std::fs::read_to_string(&path).expect("census log readable");
    let items = replay(&log, &agent_id);

    println!("\n=== {} (adapter: {agent_id}) ===", path.display());
    let mut icounts: std::collections::BTreeMap<&str, usize> = Default::default();
    for it in &items {
        *icounts.entry(item_label(it)).or_default() += 1;
    }
    println!("items: {} total  {icounts:?}", items.len());

    let (mut with_name, mut with_diff, mut with_exit, mut n_tools) = (0, 0, 0, 0);
    for it in &items {
        if let ChatItem::ToolCall(tc) = it {
            n_tools += 1;
            with_name += usize::from(tc.tool_name.is_some());
            with_diff += usize::from(!tc.diffs.is_empty());
            with_exit += usize::from(tc.exit.is_some());
        }
    }
    println!("tools: {n_tools} | tool_name {with_name} | diffs {with_diff} | exit {with_exit}");

    println!("-- turn skeletons --");
    let bounds = turn_bounds(&items);
    for w in bounds.windows(2) {
        println!(
            "  items {}..{}: {}",
            w[0],
            w[1],
            skeleton(&items[w[0]..w[1]])
        );
    }

    let auto = Lens::preset(FoldPreset::Auto);
    println!("-- visible rows per turn, as the newest turn --");
    for tail in TAIL_WINDOWS {
        println!(
            "  tail {tail:?}: {:?}",
            per_turn_as_last(&items, auto.tail(tail))
        );
    }
    for preset in FoldPreset::ALL {
        println!(
            "  mode {preset:?}: {:?}",
            per_turn_as_last(&items, Lens::preset(preset))
        );
    }
    for (name, tokens) in FILTERS {
        let lens = auto.filter(DisplayFilter::from_tokens(tokens.iter().copied()));
        println!("  filter {name}: {:?}", per_turn_as_last(&items, lens));
    }
    println!(
        "-- visible rows per turn, settled (only the last turn is newest) --\n  {:?}",
        per_turn_settled(&items, auto)
    );

    print_step_samples(&items);
}

fn print_step_samples(items: &[ChatItem]) {
    let first_line = |t: &str| -> String {
        t.lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .chars()
            .take(72)
            .collect()
    };
    println!("-- what a Step header could say --");
    let mut shown = 0;
    let mut k = 0;
    while k < items.len() && shown < SAMPLED_STEPS {
        if !matches!(items[k], ChatItem::ToolCall(_)) {
            k += 1;
            continue;
        }
        let start = k;
        while k < items.len() && matches!(items[k], ChatItem::ToolCall(_)) {
            k += 1;
        }
        let (mut think, mut assist) = (String::new(), String::new());
        for j in (0..start).rev().take(4) {
            match &items[j] {
                ChatItem::Thinking { text, .. } if think.is_empty() => think = first_line(text),
                ChatItem::AssistantText { text, .. } if assist.is_empty() => {
                    assist = first_line(text);
                }
                ChatItem::ToolCall(_) => break,
                _ => {}
            }
        }
        let titles: Vec<&str> = items[start..k]
            .iter()
            .filter_map(|it| match it {
                ChatItem::ToolCall(tc) => Some(tc.title.as_str()),
                _ => None,
            })
            .take(4)
            .collect();
        println!("  STEP run@{start} len={}", k - start);
        println!("    thinking: {think:?}");
        println!("    assistant: {assist:?}");
        println!("    tool titles: {titles:?}");
        shown += 1;
    }
}
