//! Fixed native library layouts for populated, empty and mixed-scope captures.

use crate::agent::{
    configuration::{
        Provider,
        library::{Group, Inventory, Target},
    },
    mcp::{McpScope, McpServer, McpTransport},
    skills::{Skill, SkillScope, SkillsSnapshot},
};
use crate::workspace::Workspace;
use daruda_store::project::RightDockView;
use gpui::{App, Entity, Window};

const DOCK_WIDTH: f32 = 300.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LibraryShot {
    Skills,
    SkillsEmpty,
    SkillsMixed,
    SkillsProjectEmpty,
    SkillsPersonalEmpty,
    SkillsPluginEmpty,
    SkillsSearch,
    Tools,
    ToolsEmpty,
}

pub(in crate::workspace) fn drive(
    shot: LibraryShot,
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    workspace.update(cx, |ws, cx| {
        ws.right_views.pause_library_for_shot();
        ws.right_views.library.manual = Some(Provider::Claude);
        ws.right_views.library.manual_pane = Some(ws.active_runtime().focused_pane_id);
        let mut view = ws.library_context();
        let Some(mut config) = view.config.clone() else {
            return;
        };
        // A missing project is still represented in the fixture without
        // creating files or touching a user's agent configuration.
        config.cwd = Some("/workspace/project".into());
        let mut inventory = Inventory::default();
        let skills = !matches!(shot, LibraryShot::Tools | LibraryShot::ToolsEmpty);
        if skills {
            inventory.skills = SkillsSnapshot {
                project_root: config.cwd.clone(),
                ..Default::default()
            };
            if shot != LibraryShot::SkillsEmpty {
                if !matches!(
                    shot,
                    LibraryShot::SkillsMixed | LibraryShot::SkillsProjectEmpty
                ) {
                    inventory.skills.project = vec![
                        skill(
                            "create-page",
                            "Create a frontend page with the project conventions.",
                            SkillScope::Project,
                        ),
                        skill(
                            "review-coderabbit-with-a-long-unbroken-name",
                            "Review pull request feedback without truncating action targets.",
                            SkillScope::Project,
                        ),
                        skill(
                            "screen-review",
                            "Review screenshots · 화면 간격과 정렬을 확인합니다.",
                            SkillScope::Project,
                        ),
                        skill(
                            "typecheck",
                            "Run TypeScript verification for the project.",
                            SkillScope::Project,
                        ),
                    ];
                }
                if !matches!(
                    shot,
                    LibraryShot::SkillsMixed | LibraryShot::SkillsPersonalEmpty
                ) {
                    inventory.skills.personal = vec![skill(
                        "daily-review",
                        "Review your changes before ending a work session.",
                        SkillScope::Personal,
                    )];
                }
                if shot != LibraryShot::SkillsPluginEmpty {
                    inventory.skills.plugin = vec![skill(
                        "superpowers:brainstorming",
                        "Explore requirements before implementation.",
                        SkillScope::Plugin,
                    )];
                }
                ws.right_views
                    .skill_plugin_expanded
                    .insert("superpowers@sample".into());
            }
        } else {
            for scope in [McpScope::Project, McpScope::User, McpScope::Local] {
                let Some(target) = Target::new(&config, scope, None) else {
                    continue;
                };
                let servers = if shot == LibraryShot::ToolsEmpty || scope == McpScope::Local {
                    vec![]
                } else {
                    vec![server(
                        scope,
                        if scope == McpScope::Project {
                            "playwright-with-a-long-unbroken-name"
                        } else {
                            "node_repl"
                        },
                    )]
                };
                inventory.groups.push(Group { target, servers });
            }
        }
        view.inventory = inventory;
        view.loading = false;
        view.error = None;
        ws.right_views.library.view = view;
        ws.right_views.skill_search_input.update(cx, |input, cx| {
            input.set_value(
                if shot == LibraryShot::SkillsSearch {
                    "no-matching-skill"
                } else {
                    ""
                },
                window,
                cx,
            )
        });
        ws.reveal_right_dock_view(
            if skills {
                RightDockView::Skills
            } else {
                RightDockView::Tools
            },
            cx,
        );
        ws.docks.right.update(cx, |dock, cx| {
            dock.size = DOCK_WIDTH;
            cx.notify();
        });
        cx.notify();
    });
}

fn skill(name: &str, description: &str, scope: SkillScope) -> Skill {
    let mut frontmatter = crate::agent::skills::frontmatter::SkillFrontmatter::empty();
    frontmatter.description = Some(description.into());
    Skill {
        name: name.into(),
        dir: std::path::Path::new("/workspace/project/.agents/skills").join(name),
        scope,
        frontmatter,
        body_preview: String::new(),
        aux_file_count: 0,
        modified_at: std::time::SystemTime::UNIX_EPOCH,
        plugin_id: (scope == SkillScope::Plugin).then(|| "superpowers@sample".into()),
        plugin_availability: None,
    }
}

fn server(scope: McpScope, name: &str) -> McpServer {
    McpServer {
        name: name.into(),
        scope,
        transport: McpTransport::Stdio,
        command: Some("example-server".into()),
        args: vec![],
        url: None,
        env: Default::default(),
        headers: Default::default(),
        disabled: scope == McpScope::User,
        extra: Default::default(),
    }
}

#[cfg(test)]
mod tests {}
