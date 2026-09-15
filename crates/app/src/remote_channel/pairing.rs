use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(600);
const MAX_ATTEMPTS: u32 = 5;

pub struct Pairing {
    code: String,
    created: Instant,
    attempts: u32,
}

impl Pairing {
    pub fn new() -> Self {
        Self {
            code: uuid::Uuid::new_v4().simple().to_string()[..6].to_uppercase(),
            created: Instant::now(),
            attempts: 0,
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn matches(&mut self, text: &str) -> bool {
        if self.created.elapsed() >= TTL || self.attempts >= MAX_ATTEMPTS {
            return false;
        }
        let Some(code) = pair_code(text) else {
            return false;
        };
        self.attempts += 1;
        self.code.eq_ignore_ascii_case(code.trim())
    }
}

/// The code a pairing message carries, or `None` when it is not one. Callers
/// use it to recognise an attempt that must never reach an agent as a prompt.
pub fn pair_code(text: &str) -> Option<&str> {
    text.strip_prefix("!pair ")
        .or_else(|| text.strip_prefix("/pair "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_expires_and_limits_guesses() {
        let mut pairing = Pairing::new();
        let right = format!("!pair {}", pairing.code());
        for _ in 0..MAX_ATTEMPTS {
            assert!(!pairing.matches("!pair wrong"));
        }
        assert!(!pairing.matches(&right));
        let mut pairing = Pairing::new();
        pairing.created -= TTL;
        assert!(!pairing.matches(&format!("!pair {}", pairing.code())));
    }

    #[test]
    fn a_pairing_attempt_is_recognisable_whether_or_not_it_matches() {
        assert_eq!(pair_code("!pair ABC123"), Some("ABC123"));
        assert_eq!(pair_code("/pair abc"), Some("abc"));
        assert_eq!(pair_code("ship it"), None);
        assert_eq!(pair_code("!pairing"), None);
    }

    #[test]
    fn slack_text_prefix_and_case_insensitive_code_work() {
        let mut pairing = Pairing::new();
        assert!(pairing.matches(&format!("!pair {}", pairing.code().to_lowercase())));
    }
}
