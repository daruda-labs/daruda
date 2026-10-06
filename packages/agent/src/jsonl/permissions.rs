//! `~/.claude/settings.json` permissions parser used by the JSONL FSM.
//!
//! Ported from c9watch (`src-tauri/src/session/permissions.rs:1-208`,
//! MIT — see `LICENSE-THIRD-PARTY.md`). The FSM uses this to decide
//! whether a pending `tool_use` is auto-approved (→ `Working`) or
//! needs the user (→ `NeedsAttention`).

use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ClaudeSettings {
    pub permissions: Option<Permissions>,
}

#[derive(Debug, Deserialize)]
pub struct Permissions {
    pub allow: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct PermissionChecker {
    allowed_patterns: Vec<AllowPattern>,
}

#[derive(Debug, Clone)]
enum AllowPattern {
    /// `Bash(prefix:*)` or `Bash(exact)`.
    Bash { prefix: String, wildcard: bool },
    /// Bare tool name like `Read` — full tool allow.
    Tool { name: String },
    /// `mcp__server__tool` — MCP tool.
    Mcp { name: String },
    /// `Skill(name)`. Parsed but not currently consulted by the
    /// auto-approve check — Skills go through their own allow list
    /// inside Claude Code. Kept here so the user's `permissions.allow`
    /// list isn't dropped during the parse round-trip.
    Skill {
        #[allow(dead_code)]
        name: String,
    },
}

impl PermissionChecker {
    /// Load from `~/.claude/settings.json`. Returns an empty checker
    /// (allows nothing beyond the always-on whitelist) if the file is
    /// missing or malformed.
    pub fn from_settings_file() -> Self {
        let Some(home) = dirs::home_dir() else {
            return Self::default();
        };
        Self::from_file(&home.join(".claude").join("settings.json"))
    }

    pub fn from_file(path: &Path) -> Self {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };
        let settings: ClaudeSettings = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(_) => return Self::default(),
        };
        let allowed = settings
            .permissions
            .and_then(|p| p.allow)
            .unwrap_or_default();
        let patterns = allowed
            .iter()
            .filter_map(|s| Self::parse_pattern(s))
            .collect();
        Self {
            allowed_patterns: patterns,
        }
    }

    fn parse_pattern(pattern: &str) -> Option<AllowPattern> {
        if let Some(inner) = pattern
            .strip_prefix("Bash(")
            .and_then(|s| s.strip_suffix(')'))
        {
            if let Some(prefix) = inner.strip_suffix(":*") {
                return Some(AllowPattern::Bash {
                    prefix: prefix.to_string(),
                    wildcard: true,
                });
            }
            return Some(AllowPattern::Bash {
                prefix: inner.to_string(),
                wildcard: false,
            });
        }
        if pattern.starts_with("mcp__") {
            return Some(AllowPattern::Mcp {
                name: pattern.to_string(),
            });
        }
        if let Some(inner) = pattern
            .strip_prefix("Skill(")
            .and_then(|s| s.strip_suffix(')'))
        {
            return Some(AllowPattern::Skill {
                name: inner.to_string(),
            });
        }
        if !pattern.contains('(') && !pattern.contains("__") {
            return Some(AllowPattern::Tool {
                name: pattern.to_string(),
            });
        }
        None
    }

    /// True if a pending `tool_use` does **not** need a permission
    /// dialog — either by being in the always-allowed read-only set
    /// or by matching an explicit `permissions.allow[]` entry.
    pub fn is_auto_approved(&self, tool_name: &str, tool_input: &serde_json::Value) -> bool {
        // Hardcoded read-only / no-side-effect set.
        match tool_name {
            "Read" | "Glob" | "Grep" | "WebFetch" | "WebSearch" | "Task" | "TaskList"
            | "TaskGet" | "TaskCreate" | "TaskUpdate" | "AskUserQuestion" => return true,
            _ => {}
        }

        if tool_name == "Bash" {
            let command = tool_input
                .get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            return self.is_bash_allowed(command);
        }

        if tool_name == "Write" || tool_name == "Edit" || tool_name == "NotebookEdit" {
            return self.is_tool_allowed(tool_name);
        }

        if tool_name.starts_with("mcp__") {
            return self.is_mcp_allowed(tool_name);
        }

        false
    }

    fn is_bash_allowed(&self, command: &str) -> bool {
        let trimmed = command.trim();
        for pattern in &self.allowed_patterns {
            if let AllowPattern::Bash { prefix, wildcard } = pattern {
                if *wildcard {
                    if trimmed.starts_with(prefix) {
                        return true;
                    }
                } else if trimmed == prefix {
                    return true;
                }
            }
        }
        false
    }

    fn is_tool_allowed(&self, tool_name: &str) -> bool {
        self.allowed_patterns
            .iter()
            .any(|p| matches!(p, AllowPattern::Tool { name } if name == tool_name))
    }

    fn is_mcp_allowed(&self, tool_name: &str) -> bool {
        self.allowed_patterns
            .iter()
            .any(|p| matches!(p, AllowPattern::Mcp { name } if name == tool_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bash_wildcard() {
        let p = PermissionChecker::parse_pattern("Bash(git add:*)").unwrap();
        assert!(matches!(p, AllowPattern::Bash { prefix, wildcard: true } if prefix == "git add"));
    }

    #[test]
    fn parse_bash_exact() {
        let p = PermissionChecker::parse_pattern("Bash(npm ci)").unwrap();
        assert!(matches!(p, AllowPattern::Bash { prefix, wildcard: false } if prefix == "npm ci"));
    }

    #[test]
    fn parse_mcp() {
        let p = PermissionChecker::parse_pattern("mcp__atlassian__getJiraIssue").unwrap();
        assert!(matches!(p, AllowPattern::Mcp { name } if name == "mcp__atlassian__getJiraIssue"));
    }

    #[test]
    fn parse_skill() {
        let p = PermissionChecker::parse_pattern("Skill(my-skill)").unwrap();
        assert!(matches!(p, AllowPattern::Skill { name } if name == "my-skill"));
    }

    #[test]
    fn parse_tool() {
        let p = PermissionChecker::parse_pattern("Read").unwrap();
        assert!(matches!(p, AllowPattern::Tool { name } if name == "Read"));
    }

    #[test]
    fn always_allowed_read_only_tools() {
        let c = PermissionChecker::default();
        for t in ["Read", "Glob", "Grep", "WebFetch", "WebSearch"] {
            assert!(c.is_auto_approved(t, &serde_json::json!({})), "{t}");
        }
    }

    #[test]
    fn bash_wildcard_match() {
        let c = PermissionChecker {
            allowed_patterns: vec![AllowPattern::Bash {
                prefix: "git add".to_string(),
                wildcard: true,
            }],
        };
        assert!(c.is_auto_approved("Bash", &serde_json::json!({"command": "git add ."})));
        assert!(c.is_auto_approved("Bash", &serde_json::json!({"command": "git add -p"})));
        assert!(!c.is_auto_approved("Bash", &serde_json::json!({"command": "git push"})));
    }

    #[test]
    fn bash_exact_match() {
        let c = PermissionChecker {
            allowed_patterns: vec![AllowPattern::Bash {
                prefix: "npm ci".to_string(),
                wildcard: false,
            }],
        };
        assert!(c.is_auto_approved("Bash", &serde_json::json!({"command": "npm ci"})));
        assert!(!c.is_auto_approved("Bash", &serde_json::json!({"command": "npm ci --offline"})));
    }

    #[test]
    fn write_edit_require_explicit_allow() {
        let c_empty = PermissionChecker::default();
        assert!(!c_empty.is_auto_approved("Write", &serde_json::json!({})));
        let c = PermissionChecker {
            allowed_patterns: vec![AllowPattern::Tool {
                name: "Write".to_string(),
            }],
        };
        assert!(c.is_auto_approved("Write", &serde_json::json!({})));
    }

    #[test]
    fn mcp_explicit_match() {
        let c = PermissionChecker {
            allowed_patterns: vec![AllowPattern::Mcp {
                name: "mcp__memory__set".to_string(),
            }],
        };
        assert!(c.is_auto_approved("mcp__memory__set", &serde_json::json!({})));
        assert!(!c.is_auto_approved("mcp__memory__delete", &serde_json::json!({})));
    }

    #[test]
    fn unknown_tool_defaults_to_needs_permission() {
        let c = PermissionChecker::default();
        assert!(!c.is_auto_approved("MysteryTool", &serde_json::json!({})));
    }

    #[test]
    fn from_settings_real_file_does_not_panic() {
        let _ = PermissionChecker::from_settings_file();
    }
}
