//! Client-side Hangul composer.
//!
//! macOS IME occasionally commits individual Hangul Compatibility Jamo
//! (U+3131..U+3163) to our EntityInputHandler instead of precomposed
//! syllables (e.g. during the Mach Port init delay right after an
//! input-source switch). Emitting raw compat jamo to the PTY triggers
//! a Claude Code rendering bug — Claude advances its internal column
//! by a fixed +2 per committed char regardless of display width,
//! leaving the previous cursor-block cell untouched after every
//! narrow commit and accumulating visible `█` ghosts. See
//! `Projects/daruda/Tasks/Hangul-IME-Composer-Plan.md`.
//!
//! This composer sits in front of the PTY write path and combines
//! consecutive compat jamo into precomposed syllables
//! (U+AC00..U+D7A3, East Asian Wide) before anything reaches the PTY.
//! Non-Hangul input and fully-formed syllables pass through unchanged.
//! The composer holds at most one in-progress syllable; a non-jamo
//! input, a syllable boundary (choseong-overlap), or explicit
//! [`HangulComposer::flush`] / [`HangulComposer::reset`] releases it.
//!
//! Handles LV / LVT syllables: single-consonant choseong + jungseong +
//! optional single-consonant jongseong + jong-migration when a vowel
//! follows a jong. Does not compose compound jong / compound jung /
//! cluster splits.

use super::jamo::{
    HANGUL_S_BASE, choseong_index, is_compat_choseong, is_compat_jongseong, is_compat_jungseong,
    is_hangul_compat_jamo, jongseong_index, jungseong_index,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Empty,
    /// Choseong alone — waiting for a vowel.
    Cho(char),
    /// Choseong + jungseong = LV syllable.
    ChoJung(char, char),
    /// Choseong + jungseong + jongseong = LVT syllable.
    ChoJungJong(char, char, char),
}

pub(crate) struct HangulComposer {
    state: State,
}

impl HangulComposer {
    pub(crate) fn new() -> Self {
        Self {
            state: State::Empty,
        }
    }

    /// Feed one character. Returns zero or more strings to forward to
    /// the PTY in emit order. Non-Hangul input first flushes any
    /// in-progress syllable, then passes the character through.
    pub(crate) fn feed(&mut self, ch: char) -> Vec<String> {
        if !is_hangul_compat_jamo(ch) {
            let mut out = Vec::with_capacity(2);
            if let Some(s) = self.flush() {
                out.push(s);
            }
            out.push(ch.to_string());
            return out;
        }
        self.feed_jamo(ch)
    }

    fn feed_jamo(&mut self, ch: char) -> Vec<String> {
        match self.state {
            State::Empty => {
                if is_compat_choseong(ch) {
                    self.state = State::Cho(ch);
                    Vec::new()
                } else {
                    // Jungseong or pure-cluster jongseong with no
                    // choseong in play. Cannot anchor a syllable —
                    // emit as-is so the caller is not surprised by
                    // swallowed input.
                    vec![ch.to_string()]
                }
            }
            State::Cho(c) => {
                if is_compat_jungseong(ch) {
                    self.state = State::ChoJung(c, ch);
                    Vec::new()
                } else if is_compat_choseong(ch) {
                    // Consonant after consonant without an intervening
                    // vowel: flush the first as a standalone jamo,
                    // start a fresh Cho. Rare outside of deliberate
                    // testing, but matches Korean IME intuition.
                    self.state = State::Cho(ch);
                    vec![c.to_string()]
                } else {
                    // Compat jamo that's neither a cho nor a jung
                    // (pure-cluster jongseong). Flush the cho and
                    // emit the stray jamo.
                    self.state = State::Empty;
                    vec![c.to_string(), ch.to_string()]
                }
            }
            State::ChoJung(c, v) => {
                if is_compat_jongseong(ch) {
                    self.state = State::ChoJungJong(c, v, ch);
                    Vec::new()
                } else if is_compat_jungseong(ch) {
                    // Two consecutive vowels: flush LV, then emit the
                    // new vowel as a standalone jamo (cannot start a
                    // new syllable with a vowel).
                    let lv = compose_lv(c, v);
                    self.state = State::Empty;
                    vec![lv.to_string(), ch.to_string()]
                } else if is_compat_choseong(ch) {
                    // A choseong that isn't jongseong-capable (ㄸ / ㅃ
                    // / ㅉ). Flush LV, start a new Cho.
                    let lv = compose_lv(c, v);
                    self.state = State::Cho(ch);
                    vec![lv.to_string()]
                } else {
                    // Defensive path — compat jamo that classifies
                    // as none of cho / jung / jong. Not currently
                    // reachable given the U+3131..U+3163 table, but
                    // degrade gracefully.
                    let lv = compose_lv(c, v);
                    self.state = State::Empty;
                    vec![lv.to_string(), ch.to_string()]
                }
            }
            State::ChoJungJong(c, v, t) => {
                if is_compat_jungseong(ch) {
                    // Jong-migration: a vowel after the jong means
                    // the user was starting a new syllable with the
                    // jong as its cho. Flush LV (without the jong)
                    // and transition. Only safe when the jong is a
                    // single consonant that's also a valid cho — cluster
                    // jongs are not split (ㄳ → ㄱ+ㅅ).
                    if is_compat_choseong(t) {
                        let lv = compose_lv(c, v);
                        self.state = State::ChoJung(t, ch);
                        vec![lv.to_string()]
                    } else {
                        // Cluster jong: flush the full LVT, emit the
                        // vowel as a standalone jamo.
                        let lvt = compose_lvt(c, v, t);
                        self.state = State::Empty;
                        vec![lvt.to_string(), ch.to_string()]
                    }
                } else if is_compat_choseong(ch) {
                    // New syllable's cho. Flush LVT, start fresh Cho.
                    let lvt = compose_lvt(c, v, t);
                    self.state = State::Cho(ch);
                    vec![lvt.to_string()]
                } else {
                    // Another jongseong / cluster after an LVT. Compound
                    // jongs are not composed; flush what we have and emit
                    // the input as-is.
                    let lvt = compose_lvt(c, v, t);
                    self.state = State::Empty;
                    vec![lvt.to_string(), ch.to_string()]
                }
            }
        }
    }

    /// Emit the in-progress syllable as a precomposed string and
    /// return to [`State::Empty`]. Returns `None` when nothing is in
    /// flight. Callers invoke this when a control key, special
    /// sequence, or mode change (e.g. Cmd+F search overlay) would
    /// otherwise strand the partial composition in an unrelated
    /// context.
    pub(crate) fn flush(&mut self) -> Option<String> {
        let state = std::mem::replace(&mut self.state, State::Empty);
        match state {
            State::Empty => None,
            State::Cho(c) => Some(c.to_string()),
            State::ChoJung(c, v) => Some(compose_lv(c, v).to_string()),
            State::ChoJungJong(c, v, t) => Some(compose_lvt(c, v, t).to_string()),
        }
    }

    /// Drop the in-progress syllable without emitting. Used when the
    /// host context (e.g. the search overlay takes over input
    /// routing) invalidates the PTY target and flushing would leak a
    /// partial syllable into an unrelated buffer.
    pub(crate) fn reset(&mut self) {
        self.state = State::Empty;
    }

    /// True when the composer holds a lone choseong with no jungseong
    /// yet. Callers use this to decide whether to flush on an incoming
    /// consonant preedit: a bare Cho cannot combine with another
    /// consonant, so flushing it immediately is correct; a ChoJung
    /// state may still accept a jongseong, so it must not be flushed.
    pub(crate) fn is_cho_only(&self) -> bool {
        matches!(self.state, State::Cho(_))
    }

    /// True when the composer holds any in-progress syllable.
    pub(crate) fn is_composing(&self) -> bool {
        !matches!(self.state, State::Empty)
    }

    /// Returns what the preedit should show if `ch` were the next jamo,
    /// without mutating the composer. Returns `None` when `ch` cannot
    /// extend the current state (e.g. a new choseong arriving after
    /// ChoJungJong, or a consonant arriving in Empty state).
    pub(crate) fn peek_with(&self, ch: char) -> Option<String> {
        if !is_hangul_compat_jamo(ch) {
            return None;
        }
        match self.state {
            State::Empty => None,
            State::Cho(c) if is_compat_jungseong(ch) => Some(compose_lv(c, ch).to_string()),
            State::ChoJung(c, v) if is_compat_jongseong(ch) => {
                Some(compose_lvt(c, v, ch).to_string())
            }
            _ => None,
        }
    }

    #[cfg(test)]
    fn is_idle(&self) -> bool {
        matches!(self.state, State::Empty)
    }
}

/// Compose a single LV (choseong + jungseong) syllable. Panics only on
/// an upstream classification bug — every caller routes through the
/// compat-jamo classifiers before reaching here.
fn compose_lv(cho: char, jung: char) -> char {
    let cho_i = choseong_index(cho).expect("compose_lv: cho outside choseong table");
    let jung_i = jungseong_index(jung).expect("compose_lv: jung outside jungseong table");
    char::from_u32(HANGUL_S_BASE + (cho_i * 21 + jung_i) * 28)
        .expect("LV codepoint inside Hangul syllable block")
}

/// Compose a full LVT (cho + jung + jong) syllable.
fn compose_lvt(cho: char, jung: char, jong: char) -> char {
    let cho_i = choseong_index(cho).expect("compose_lvt: cho outside choseong table");
    let jung_i = jungseong_index(jung).expect("compose_lvt: jung outside jungseong table");
    let jong_i = jongseong_index(jong).expect("compose_lvt: jong outside jongseong table");
    char::from_u32(HANGUL_S_BASE + (cho_i * 21 + jung_i) * 28 + jong_i)
        .expect("LVT codepoint inside Hangul syllable block")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the composer with a sequence of input chars, collecting
    /// every emission plus the final `flush()` result. Models how
    /// `replace_text_in_range` will call into it in Phase C2.
    fn drive(input: &str) -> Vec<String> {
        let mut c = HangulComposer::new();
        let mut out: Vec<String> = Vec::new();
        for ch in input.chars() {
            out.extend(c.feed(ch));
        }
        if let Some(s) = c.flush() {
            out.push(s);
        }
        out
    }

    #[test]
    fn empty_cho_jung_jong_composes_syllable() {
        // ㅇ(U+3147) + ㅏ(U+314F) + ㄴ(U+3134) → "안" (U+C548)
        assert_eq!(drive("ㅇㅏㄴ"), vec!["안"]);
    }

    #[test]
    fn cho_only_flushes_on_nonjamo() {
        // ㄱ a → standalone ㄱ then a
        assert_eq!(drive("ㄱa"), vec!["ㄱ", "a"]);
    }

    #[test]
    fn lv_flushes_on_nonjamo() {
        // 가(ㄱㅏ) then ASCII 'x' → "가", "x"
        assert_eq!(drive("ㄱㅏx"), vec!["가", "x"]);
    }

    #[test]
    fn jung_migration_reuses_prev_jong() {
        // ㅇㅏㄴㅏ → 안 in flight, next ㅏ migrates ㄴ to new syllable:
        // flush "아" (LV without jong) + in-flight "나" (ChoJung) that
        // flush() releases at the end. Matches Korean IME behaviour.
        assert_eq!(drive("ㅇㅏㄴㅏ"), vec!["아", "나"]);
    }

    #[test]
    fn choseong_overlap_flushes() {
        // ㄱㄴ → ㄱ then in-flight ㄴ (Cho state) flushed at end.
        assert_eq!(drive("ㄱㄴ"), vec!["ㄱ", "ㄴ"]);
    }

    #[test]
    fn non_korean_passes_through() {
        // abc → emitted 1:1 with no state change.
        assert_eq!(drive("abc"), vec!["a", "b", "c"]);
    }

    #[test]
    fn ascii_mixed_with_hangul() {
        // ASCII around a composed syllable — each boundary forces a
        // flush, keeping emit order stable.
        assert_eq!(drive("aㅇㅏㄴb"), vec!["a", "안", "b"]);
    }

    #[test]
    fn precomposed_syllable_passes_through() {
        // A fully-formed syllable from macOS IME (not a compat jamo)
        // must pass through unchanged — the composer is jamo-only.
        assert_eq!(drive("안"), vec!["안"]);
    }

    #[test]
    fn multiple_syllables_compose_independently() {
        // ㅇㅏㄴㄴㅕㅇ → 안 + 녕. The ㄴ between syllables migrates
        // from the first syllable's jong to the second's cho when ㅕ
        // arrives.
        assert_eq!(drive("ㅇㅏㄴㄴㅕㅇ"), vec!["안", "녕"]);
    }

    #[test]
    fn reset_during_composition_drops_buffer() {
        // Reset drops partial composition without emitting. Protects
        // against leaking a syllable into an unrelated PTY when e.g. the
        // search overlay takes focus.
        let mut c = HangulComposer::new();
        c.feed('ㄱ');
        c.feed('ㅏ');
        c.reset();
        assert!(c.is_idle());
        assert!(c.flush().is_none());
    }

    #[test]
    fn flush_on_empty_returns_none() {
        let mut c = HangulComposer::new();
        assert!(c.flush().is_none());
        assert!(c.is_idle());
    }

    #[test]
    fn flush_on_cho_returns_lone_jamo() {
        let mut c = HangulComposer::new();
        c.feed('ㄱ');
        assert_eq!(c.flush(), Some("ㄱ".into()));
    }

    #[test]
    fn flush_on_lv_returns_syllable() {
        let mut c = HangulComposer::new();
        c.feed('ㄱ');
        c.feed('ㅏ');
        assert_eq!(c.flush(), Some("가".into()));
    }

    #[test]
    fn flush_on_lvt_returns_full_syllable() {
        let mut c = HangulComposer::new();
        c.feed('ㅎ');
        c.feed('ㅏ');
        c.feed('ㄴ');
        assert_eq!(c.flush(), Some("한".into()));
    }

    #[test]
    fn double_vowel_flushes_lv_and_emits_standalone() {
        // ㄱㅏㅏ: first ㅏ closes the LV to 가; second ㅏ cannot start
        // a new syllable, so it falls through as a standalone jamo.
        assert_eq!(drive("ㄱㅏㅏ"), vec!["가", "ㅏ"]);
    }

    #[test]
    fn cho_followed_by_non_jong_cluster() {
        // ㄱ + ㄳ (pure-cluster jongseong, not a choseong): flush ㄱ
        // and pass the cluster through as a standalone jamo.
        // ㄳ = U+3133
        assert_eq!(drive("ㄱㄳ"), vec!["ㄱ", "ㄳ"]);
    }

    #[test]
    fn lvt_followed_by_new_cho_flushes() {
        // 안 in flight, ㅁ starts a new syllable (it's a choseong,
        // not a jung/jong add) → flush 안, start Cho(ㅁ).
        assert_eq!(drive("ㅇㅏㄴㅁ"), vec!["안", "ㅁ"]);
    }

    #[test]
    fn lvt_with_cluster_jong_flushes_on_vowel() {
        // Manually seed a ChoJungJong where the jong is a cluster
        // (ㄳ is jong-valid but not a choseong). A following vowel
        // must NOT migrate a cluster onto the next syllable —
        // flush full LVT and emit the vowel standalone.
        let mut c = HangulComposer::new();
        c.feed('ㄱ');
        c.feed('ㅏ');
        // ㄳ (cluster jong, not a choseong)
        c.feed('ㄳ');
        let out = c.feed('ㅏ');
        // ㄱ + ㅏ + ㄳ = U+AC03 "갃"; the trailing vowel cannot migrate a
        // cluster jong onto a new syllable, so the full LVT flushes and
        // ㅏ lands as a standalone jamo.
        assert_eq!(out, vec!["갃", "ㅏ"]);
        assert!(c.flush().is_none());
    }

    #[test]
    fn standalone_vowel_at_start_emits_as_jamo() {
        // Empty + jung → no syllable to anchor, emit the jamo.
        assert_eq!(drive("ㅏ"), vec!["ㅏ"]);
    }

    #[test]
    fn no_emit_while_composing() {
        // While a syllable is still being assembled, feed returns
        // nothing. This is the property Claude relies on — no commit
        // byte reaches the PTY until the composer releases the final
        // (wide) syllable.
        let mut c = HangulComposer::new();
        assert!(c.feed('ㄱ').is_empty());
        assert!(c.feed('ㅏ').is_empty());
        assert!(c.feed('ㅇ').is_empty()); // jong add
        // Nothing has been emitted yet; three compat jamo in, zero
        // out, one wide syllable pending.
        assert_eq!(c.flush(), Some("강".into()));
    }

    #[test]
    fn feed_never_emits_del_or_bs() {
        // Invariant the composer exists to preserve: no output
        // string ever contains DEL (0x7f) or BS (0x08). The retired
        // DEL + rewrite recomposition path would have leaked these
        // into Claude Code's input stream on every composed syllable.
        // Exhaustively exercise the choseong × jungseong × optional
        // jongseong space so a future edit that reintroduces a
        // "send DEL before rewriting" shortcut is caught here.
        let choseong: [char; 19] = [
            'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ',
            'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
        ];
        let jungseong: [char; 21] = [
            'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ',
            'ㅝ', 'ㅞ', 'ㅟ', 'ㅠ', 'ㅡ', 'ㅢ', 'ㅣ',
        ];
        let jongseong_single: [char; 19] = [
            'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅊ', 'ㅋ', 'ㅌ',
            'ㅍ', 'ㅎ', 'ㄳ', 'ㄵ', 'ㅄ',
        ];

        let check = |s: &str| {
            assert!(
                !s.as_bytes().contains(&0x7f) && !s.as_bytes().contains(&0x08),
                "composer emitted DEL/BS byte in {s:?}"
            );
        };

        for &cho in &choseong {
            for &jung in &jungseong {
                for &jong in &jongseong_single {
                    let out = drive(&format!("{cho}{jung}{jong}"));
                    for s in out {
                        check(&s);
                    }
                }
            }
        }

        // Mixed sequences that exercise the jong-migration and
        // non-jamo-boundary paths — the most likely regression sites.
        for seq in [
            "ㅇㅏㄴㄴㅕㅇㅎㅏㅅㅔㅇㅛ", // 안녕하세요
            "ㄱㅏaㄴㅏb",
            "ㅎㅎㅎ",
            "ㅏㅣㅗ",
            "ㅇㅏㄴㅏ", // jong migration
            "ㅎㅏㄴㄱㅏ",
        ] {
            for s in drive(seq) {
                check(&s);
            }
        }
    }
}
