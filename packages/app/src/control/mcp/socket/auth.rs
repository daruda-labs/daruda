//! Who is allowed to speak on the socket.
//!
//! A token belongs to a session, not to the process: a new orchestrator mints
//! its own and the previous one stops authenticating, so a shim left over from
//! a discarded session cannot keep driving the app.

/// The one non-MCP frame on the wire.
///
/// Answering without a reason on failure is deliberate: a caller guessing
/// tokens learns nothing from the reply, and there is only ever one legitimate
/// client, which already knows its own token.
pub(crate) fn handshake_reply(frame: &str, expected: &str, runtime_id: &str) -> (bool, String) {
    let offered = serde_json::from_str::<serde_json::Value>(frame)
        .ok()
        .and_then(|v| v.get("token").and_then(|t| t.as_str()).map(str::to_owned));
    // Constant-time-ish: compare full byte strings rather than returning early
    // on the first differing byte. Not a real defence — a local attacker has
    // better options — but it costs nothing.
    let ok = offered.is_some_and(|t| {
        t.len() == expected.len()
            && t.bytes()
                .zip(expected.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    });
    let reply = if ok {
        serde_json::json!({ "ok": true, "runtime_id": runtime_id }).to_string()
    } else {
        serde_json::json!({ "ok": false, "error": "unauthorized" }).to_string()
    };
    // The decision travels beside the reply rather than being recovered from
    // it: an authorization outcome should not be re-derived by substring.
    (ok, reply)
}

/// A fresh per-run identity and its session token.
///
/// Both random per run: the id so a restarted daruda is distinguishable from
/// the one a shim connected to, the token so a shim from a previous run cannot
/// authenticate against this one.
pub(crate) fn new_runtime_id() -> String {
    uuid::Uuid::new_v4().as_simple().to_string()
}

pub(crate) fn new_token() -> String {
    uuid::Uuid::new_v4().as_simple().to_string()
}

/// The token the socket accepts right now, and which orchestrator session it
/// belongs to.
///
/// Shared with the accept loop rather than copied into it — and behind a lock
/// because that loop runs on the background executor while the rotation
/// happens on the foreground. The token is bound to the *session*, not the
/// process: creating an orchestrator session
/// rotates it, so a shim from a discarded session can neither authenticate
/// again nor keep issuing calls on a connection it authorized earlier.
///
/// The epoch is what makes the second half work — a rotation cannot reach into
/// an established connection, so each one remembers the epoch it authorized
/// under and stops being served once that epoch is retired.
#[derive(Clone)]
pub(crate) struct TokenGate(std::sync::Arc<std::sync::Mutex<Gate>>);

struct Gate {
    token: String,
    epoch: u64,
}

impl TokenGate {
    /// A gate holding a token no one has been given.
    ///
    /// Random rather than empty: an empty expected token would match a peer
    /// that offers an empty one, so the pre-session state would authorize
    /// anybody. [`Self::rotate`] is the only way to obtain a usable token.
    pub(crate) fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(Gate {
            token: new_token(),
            epoch: 0,
        })))
    }

    /// Issue a fresh token for a new session and retire the previous one.
    pub(crate) fn rotate(&self) -> String {
        let token = new_token();
        let mut gate = self.lock();
        gate.token = token.clone();
        gate.epoch += 1;
        token
    }

    /// A gate already holding `token`, so a test can know what authenticates
    /// without reaching into the guarded state.
    #[cfg(test)]
    pub(super) fn holding(token: &str) -> Self {
        let gate = Self::new();
        gate.rotate();
        gate.lock().token = token.to_owned();
        gate
    }

    pub(super) fn current(&self) -> (String, u64) {
        let gate = self.lock();
        (gate.token.clone(), gate.epoch)
    }

    pub(super) fn epoch(&self) -> u64 {
        self.lock().epoch
    }

    /// A poisoned mutex means a panic while rotating, which cannot leave the
    /// token half-written — the fields are replaced together under the lock.
    fn lock(&self) -> std::sync::MutexGuard<'_, Gate> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}
