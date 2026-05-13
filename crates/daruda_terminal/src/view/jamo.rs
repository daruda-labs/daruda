// Korean Jamo client-side recomposition
//
// When the macOS Korean IME fails to compose (e.g. Mach Port init delay after
// input method switch), individual Compatibility Jamo (U+3131–U+3163) are
// committed one by one.  This module detects consecutive jamo and recomposes
// them into proper Hangul syllables using backspace + rewrite to PTY.

pub(super) const HANGUL_S_BASE: u32 = 0xAC00;

/// Map a Compatibility Jamo consonant to its Choseong (leading) index (0–18).
pub(super) fn choseong_index(ch: char) -> Option<u32> {
    match ch {
        'ㄱ' => Some(0),
        'ㄲ' => Some(1),
        'ㄴ' => Some(2),
        'ㄷ' => Some(3),
        'ㄸ' => Some(4),
        'ㄹ' => Some(5),
        'ㅁ' => Some(6),
        'ㅂ' => Some(7),
        'ㅃ' => Some(8),
        'ㅅ' => Some(9),
        'ㅆ' => Some(10),
        'ㅇ' => Some(11),
        'ㅈ' => Some(12),
        'ㅉ' => Some(13),
        'ㅊ' => Some(14),
        'ㅋ' => Some(15),
        'ㅌ' => Some(16),
        'ㅍ' => Some(17),
        'ㅎ' => Some(18),
        _ => None,
    }
}

/// Map a Compatibility Jamo vowel to its Jungseong (medial) index (0–20).
pub(super) fn jungseong_index(ch: char) -> Option<u32> {
    let code = ch as u32;
    if (0x314F..=0x3163).contains(&code) {
        Some(code - 0x314F)
    } else {
        None
    }
}

/// Map a Compatibility Jamo consonant to its Jongseong (trailing) index (1–27).
/// Returns None for consonants that cannot appear as trailing (ㄸ, ㅃ, ㅉ).
pub(super) fn jongseong_index(ch: char) -> Option<u32> {
    match ch {
        'ㄱ' => Some(1),
        'ㄲ' => Some(2),
        'ㄳ' => Some(3),
        'ㄴ' => Some(4),
        'ㄵ' => Some(5),
        'ㄶ' => Some(6),
        'ㄷ' => Some(7),
        'ㄹ' => Some(8),
        'ㄺ' => Some(9),
        'ㄻ' => Some(10),
        'ㄼ' => Some(11),
        'ㄽ' => Some(12),
        'ㄾ' => Some(13),
        'ㄿ' => Some(14),
        'ㅀ' => Some(15),
        'ㅁ' => Some(16),
        'ㅂ' => Some(17),
        'ㅄ' => Some(18),
        'ㅅ' => Some(19),
        'ㅆ' => Some(20),
        'ㅇ' => Some(21),
        'ㅈ' => Some(22),
        'ㅊ' => Some(23),
        'ㅋ' => Some(24),
        'ㅌ' => Some(25),
        'ㅍ' => Some(26),
        'ㅎ' => Some(27),
        _ => None,
    }
}

/// True if `ch` is a Hangul Compatibility Jamo (U+3131–U+3163).
pub(super) fn is_hangul_compat_jamo(ch: char) -> bool {
    ('\u{3131}'..='\u{3163}').contains(&ch)
}

/// True if `ch` is a Compatibility Jamo that can appear as a choseong
/// (leading consonant). Excludes pure-jongseong clusters (ㄳ, ㄵ, ...)
/// that have no choseong role.
pub(super) fn is_compat_choseong(ch: char) -> bool {
    choseong_index(ch).is_some()
}

/// True if `ch` is a Compatibility Jamo jungseong (medial vowel,
/// U+314F..U+3163).
pub(super) fn is_compat_jungseong(ch: char) -> bool {
    jungseong_index(ch).is_some()
}

/// True if `ch` is a Compatibility Jamo that can appear as a jongseong
/// (trailing consonant / cluster).
pub(super) fn is_compat_jongseong(ch: char) -> bool {
    jongseong_index(ch).is_some()
}

// `is_lv_syllable` and the old `try_jamo_compose` helpers were
// retired when the HangulComposer state machine replaced the DEL +
// rewrite recomposition path (Phase C2). The composer resolves
// choseong + jungseong + jongseong internally from its own state,
// so there is no longer a caller that needs to probe a precomposed
// syllable character or attempt a one-shot compose.
