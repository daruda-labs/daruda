//! Login methods the agent advertises at `initialize`, and the shell command
//! each one runs.
//!
//! The agent lists these only when the client opts in (see
//! `session::client_capabilities`), and *which* it lists depends on where it
//! decided it is running. Locally it offers two — a subscription login and an
//! Anthropic Console login — and **they bill differently**: the first spends a
//! plan the user already pays for, while the second starts per-token billing
//! that only shows up on a later invoice. That difference is the reason this
//! is a typed model rather than a passthrough of the wire shape: a host that
//! compares id strings at the call site will eventually lose it; a host that
//! matches on [`LoginMethodKind`] cannot.
//!
//! An agent that detected it is running remotely replaces both with a single
//! interactive one, under an id of its own. A host that only knows the local
//! pair therefore reports the *only* way a remote user can sign in as a method
//! of unknown billing — which is why the remote id is recognized here too.
//!
//! The runnable command comes from the vendor-private
//! `_meta["terminal-auth"]` block, which carries the interpreter the agent is
//! actually running on plus the resolved adapter path. It is not in the ACP
//! spec, so a method without it is normal rather than an error — the host
//! falls back to deriving the command itself.
//!
//! Input is the SDK's typed `AuthMethod`, not the raw JSON: `schema::v1` reads
//! the agent's shape as-is, and taking it typed means an SDK upgrade that stops
//! matching is a compile error here rather than an empty list at runtime (the
//! `auth_methods` field skips what it cannot parse, silently).

use agent_client_protocol::schema::v1::AuthMethod;

/// How a login method bills, which decides how prominently a host may offer it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginMethodKind {
    /// Signs into a plan the user already pays for. The safe default.
    Subscription,
    /// Starts per-token API billing. A host must say so before running it.
    MeteredApi,
    /// Hands the user the agent's own interactive sign-in and lets them choose
    /// inside it. What an agent running on a remote host advertises — and the
    /// only method it advertises, so a host that refuses to offer it leaves
    /// that user no way in at all.
    ///
    /// Safe to offer without a billing warning, for the opposite reason to
    /// [`Self::Subscription`]: any billing choice is made by the user, in
    /// front of them, rather than by a host on their behalf.
    Interactive,
    /// A method this build does not recognize. Billing is unknown, so it is
    /// **not** safe to offer alongside the others by default — an agent that
    /// adds a metered method later would otherwise be surfaced as if it were
    /// free.
    Unrecognized(String),
}

impl LoginMethodKind {
    /// Classify by the agent's method id.
    #[must_use]
    pub fn from_id(id: &str) -> Self {
        match id {
            "claude-ai-login" => Self::Subscription,
            "console-login" => Self::MeteredApi,
            // Advertised only by an agent that detected it is running
            // remotely, in place of the pair above.
            "claude-login" => Self::Interactive,
            other => Self::Unrecognized(other.to_owned()),
        }
    }

    /// Whether a host may offer this without an explicit billing warning.
    /// False for anything metered or unknown — the caller must either add the
    /// warning or leave it out.
    #[must_use]
    pub fn is_safe_default(&self) -> bool {
        matches!(self, Self::Subscription | Self::Interactive)
    }
}

/// One advertised way to sign in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginMethod {
    /// The agent's method id, kept for diagnostics and round-tripping.
    pub id: String,
    /// The agent's own label. English, and not localizable — a host shows its
    /// own string and uses this only as a fallback.
    pub name: String,
    pub kind: LoginMethodKind,
    /// Interpreter + argv from `_meta["terminal-auth"]`, when the agent
    /// supplied it. `None` means the host must derive the command itself.
    pub command: Option<TerminalCommand>,
}

/// A directly executable command: a program and its arguments, already
/// resolved by the agent (no `npx` re-resolution, no guessing which Node.js).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl TerminalCommand {
    /// Render as a single shell line, quoting every word.
    ///
    /// Quoting is not cosmetic here: the agent's own paths routinely contain
    /// spaces (`~/Library/Application Support/…`), so joining on a space
    /// produces a line that runs the wrong program — or, with a hostile path,
    /// a different one entirely.
    #[must_use]
    pub fn to_shell_line(&self) -> String {
        let mut words = Vec::with_capacity(self.args.len() + 1);
        words.push(self.program.as_str());
        words.extend(self.args.iter().map(String::as_str));
        shell_words::join(words)
    }
}

/// Read the advertised methods off an `initialize` response.
///
/// The SDK has already dropped anything it could not read — `auth_methods` is
/// deserialized with `VecSkipError` — so this never rejects an entry: an id and
/// a name are guaranteed, and only the vendor-private command is optional.
///
/// The ACP method *type* is deliberately not filtered on. Which login a host
/// may offer is decided by [`LoginMethodKind`], read from the id; a method
/// arriving as some other type still bills the same way.
#[must_use]
pub fn parse_login_methods(auth_methods: &[AuthMethod]) -> Vec<LoginMethod> {
    auth_methods.iter().map(parse_one).collect()
}

fn parse_one(method: &AuthMethod) -> LoginMethod {
    let id = method.id().0.to_string();
    let kind = LoginMethodKind::from_id(&id);
    LoginMethod {
        id,
        name: method.name().to_owned(),
        kind,
        command: parse_terminal_command(method),
    }
}

/// `_meta["terminal-auth"] = { command, args }`. Absent on an agent that did
/// not see the companion `_meta` flag, so this returns `None` rather than
/// treating it as malformed.
fn parse_terminal_command(method: &AuthMethod) -> Option<TerminalCommand> {
    let block = method.meta()?.get(crate::session::TERMINAL_AUTH_META_KEY)?;
    let program = block.get("command")?.as_str()?.to_owned();
    let args = block
        .get("args")?
        .as_array()?
        .iter()
        .map(|a| a.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    Some(TerminalCommand { program, args })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{AuthMethod, InitializeResponse};

    /// Deserialize an `initialize` response exactly as the wire delivers it.
    fn advertised(auth_methods: serde_json::Value) -> Vec<AuthMethod> {
        let response: InitializeResponse = serde_json::from_value(serde_json::json!({
            "protocolVersion": 1,
            "agentCapabilities": {},
            "authMethods": auth_methods,
        }))
        .expect("an initialize response never fails to parse — every field degrades");
        response.auth_methods
    }

    /// The exact payload captured from the adapter once the client advertised
    /// `auth.terminal` + `_meta["terminal-auth"]`.
    fn captured() -> Vec<AuthMethod> {
        advertised(serde_json::json!([
            {
                "description": "Use Claude subscription ",
                "name": "Claude Subscription",
                "id": "claude-ai-login",
                "type": "terminal",
                "args": ["--cli", "auth", "login", "--claudeai"],
                "_meta": { "terminal-auth": {
                    "command": "/Users/woo/.nvm/versions/node/v22.19.0/bin/node",
                    "args": [
                        "/Users/woo/Library/Application Support/daruda/node/npx-cache/_npx/b555b4fead8494dc/node_modules/.bin/claude-agent-acp",
                        "--cli", "auth", "login", "--claudeai"
                    ],
                    "label": "Claude Login"
                }}
            },
            {
                "description": "Use Anthropic Console (API usage billing)",
                "name": "Anthropic Console",
                "id": "console-login",
                "type": "terminal",
                "args": ["--cli", "auth", "login", "--console"],
                "_meta": { "terminal-auth": {
                    "command": "/Users/woo/.nvm/versions/node/v22.19.0/bin/node",
                    "args": [
                        "/Users/woo/Library/Application Support/daruda/node/npx-cache/_npx/b555b4fead8494dc/node_modules/.bin/claude-agent-acp",
                        "--cli", "auth", "login", "--console"
                    ],
                    "label": "Anthropic Console Login"
                }}
            }
        ]))
    }

    /// The canary for the whole feature: the agent's `terminal` methods have to
    /// survive the SDK's own deserializer before this module ever sees them.
    ///
    /// `InitializeResponse::auth_methods` is `VecSkipError`, so a shape the SDK
    /// cannot read is dropped **silently** — the login methods would vanish with
    /// no error anywhere. The v2 schema module reads `methodId` where the agent
    /// sends `id`, and rejects `type: "terminal"` in its catch-all variant, so
    /// moving off `schema::v1` is exactly that silent loss.
    #[test]
    fn the_agents_terminal_methods_survive_the_sdks_own_parser() {
        let methods = captured();
        assert_eq!(
            methods.len(),
            2,
            "the SDK dropped an advertised method instead of parsing it"
        );
        assert!(
            methods.iter().all(|m| matches!(m, AuthMethod::Terminal(_))),
            "the agent advertises terminal-type logins: {methods:?}"
        );
    }

    #[test]
    fn the_captured_payload_yields_both_methods_with_their_billing() {
        let methods = parse_login_methods(&captured());
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0].kind, LoginMethodKind::Subscription);
        assert_eq!(methods[1].kind, LoginMethodKind::MeteredApi);
    }

    /// The distinction the whole type exists to protect: only the
    /// subscription login may be offered without a billing warning.
    #[test]
    fn only_the_subscription_login_is_a_safe_default() {
        let methods = parse_login_methods(&captured());
        assert!(methods[0].kind.is_safe_default());
        assert!(
            !methods[1].kind.is_safe_default(),
            "Console login starts per-token billing — never the silent default"
        );
    }

    /// An agent that adds a method later must not be surfaced as if it were
    /// free just because we don't recognize it.
    #[test]
    fn an_unrecognized_method_is_never_a_safe_default() {
        let kind = LoginMethodKind::from_id("some-future-login");
        assert_eq!(
            kind,
            LoginMethodKind::Unrecognized("some-future-login".to_owned())
        );
        assert!(!kind.is_safe_default());
    }

    /// The agent's own paths contain spaces. Joining on a space would run
    /// `/Users/woo/Library/Application` and pass the rest as arguments.
    #[test]
    fn the_shell_line_quotes_paths_that_contain_spaces() {
        let methods = parse_login_methods(&captured());
        let line = methods[0]
            .command
            .as_ref()
            .expect("the capture carries a terminal-auth block")
            .to_shell_line();
        assert!(
            line.contains("'/Users/woo/Library/Application Support/daruda/node/npx-cache/_npx/b555b4fead8494dc/node_modules/.bin/claude-agent-acp'"),
            "the spaced path must survive as one word: {line}"
        );
        assert!(line.ends_with("--cli auth login --claudeai"));
        // Round-trips back to the exact argv the agent handed over.
        let parsed = shell_words::split(&line).expect("the line re-splits");
        assert_eq!(parsed.len(), 6);
        assert_eq!(parsed[0], "/Users/woo/.nvm/versions/node/v22.19.0/bin/node");
    }

    /// Without the companion `_meta` flag only `args` arrives. That is the
    /// documented shape, not a malformed one — the entry stays, minus a
    /// runnable command, and the host derives its own.
    #[test]
    fn a_method_without_the_meta_block_is_kept_without_a_command() {
        let methods = parse_login_methods(&advertised(serde_json::json!([{
            "id": "claude-ai-login",
            "name": "Claude Subscription",
            "type": "terminal",
            "args": ["--cli", "auth", "login", "--claudeai"]
        }])));
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].kind, LoginMethodKind::Subscription);
        assert_eq!(methods[0].command, None);
    }

    /// The area is marked unstable upstream, so every reshape must degrade to
    /// "fewer methods", never to a panic or a half-built command.
    #[test]
    fn reshaped_entries_are_dropped_rather_than_half_read() {
        let methods = parse_login_methods(&advertised(serde_json::json!([
            {"name": "no id at all", "type": "terminal"},
            {"id": 7, "type": "terminal"},
            "a bare string",
            {"id": "x", "name": "X", "type": "terminal",
             "_meta": {"terminal-auth": {"command": "node"}}},
            {"id": "y", "name": "Y", "type": "terminal",
             "_meta": {"terminal-auth": {"command": "node", "args": [1, 2]}}}
        ])));
        // The two malformed `_meta` blocks still yield a usable entry — just
        // without a command; the three unreadable shapes are dropped by the
        // SDK's own `VecSkipError` before this module sees them.
        assert_eq!(methods.len(), 2);
        assert!(methods.iter().all(|m| m.command.is_none()));
    }

    /// An agent type this module does not model still has to survive as an
    /// entry — its id is what decides the billing, not its ACP method type.
    #[test]
    fn a_non_terminal_method_is_still_read() {
        let methods = parse_login_methods(&advertised(serde_json::json!([{
            "id": "console-login",
            "name": "Anthropic Console"
        }])));
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].kind, LoginMethodKind::MeteredApi);
        assert_eq!(methods[0].command, None);
    }

    /// What an agent running on a remote host advertises instead. It is a
    /// different method entirely — one entry, a different id, and no console
    /// option — so a build that only knows the local pair reports the only way
    /// in as a method of unknown billing.
    fn captured_remote() -> Vec<AuthMethod> {
        advertised(serde_json::json!([{
            "description": "Run `claude /login` in the terminal",
            "name": "Log in with Claude",
            "id": "claude-login",
            "type": "terminal",
            "args": ["--cli"],
            "_meta": { "terminal-auth": {
                "command": "/usr/bin/node",
                "args": ["/remote/claude-agent-acp", "--cli"],
                "label": "Claude Login"
            }}
        }]))
    }

    #[test]
    fn the_remote_login_is_recognized_rather_than_unknown() {
        let methods = parse_login_methods(&captured_remote());
        assert_eq!(methods.len(), 1);
        assert_eq!(
            methods[0].kind,
            LoginMethodKind::Interactive,
            "the remote flow is known, not a method this build has never seen"
        );
    }

    /// The user picks any billing choice inside the flow and sees it there, so
    /// there is nothing a host would be deciding on their behalf. Offering it
    /// has to stay possible — on a remote host it is the only way in.
    #[test]
    fn the_remote_login_may_be_offered_without_a_billing_warning() {
        let methods = parse_login_methods(&captured_remote());
        assert!(methods[0].kind.is_safe_default());
    }

    /// The command it hands over runs the CLI itself, with no flag selecting a
    /// billing path — the choice happens inside.
    #[test]
    fn the_remote_login_command_selects_no_billing_path() {
        let methods = parse_login_methods(&captured_remote());
        let line = methods[0]
            .command
            .as_ref()
            .expect("the remote method carries a terminal-auth block")
            .to_shell_line();
        assert!(!line.contains("--console"), "{line}");
        assert!(!line.contains("--claudeai"), "{line}");
    }

    #[test]
    fn no_advertised_methods_is_an_empty_list_not_an_error() {
        assert!(parse_login_methods(&[]).is_empty());
    }
}
