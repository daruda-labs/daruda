use super::PromptMarkKind;
use crate::ansi::{self, OSC7_FILE_SCHEME};
use crate::vt_codes::{
    AttentionKind, FtcsCommand, NotificationRequest, OSC_CLIPBOARD, OSC_CWD, OSC_DEFAULT_BG,
    OSC_DEFAULT_FG, OSC_FTCS, OSC_ITERM2, OSC_NOTIFICATION, OSC_NOTIFY_RXVT,
    OSC_TITLE_ICON_AND_WINDOW, OSC_TITLE_WINDOW,
};
use crate::vt_limits::{OSC133_PAYLOAD_CAP, XTGETTCAP_BODY_CAP};

// ============================================================================
// DSR scanner (CSI 5n / CSI 6n)
// ============================================================================

#[derive(Clone, Copy, Debug)]
pub(super) enum TerminalQuery {
    DeviceStatus,
    CursorPosition,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum DsrScanState {
    #[default]
    Idle,
    Esc,
    Csi,
    CsiQ,
    Csi5,
    CsiQ5,
    Csi6,
    CsiQ6,
}

impl DsrScanState {
    pub(super) fn advance(&mut self, b: u8) -> Option<TerminalQuery> {
        use DsrScanState::*;

        let matched = match (*self, b) {
            (Csi5, b'n') | (CsiQ5, b'n') => Some(TerminalQuery::DeviceStatus),
            (Csi6, b'n') | (CsiQ6, b'n') => Some(TerminalQuery::CursorPosition),
            _ => None,
        };

        *self = match (*self, b) {
            (_, 0x1b) => Esc,
            (Esc, b'[') => Csi,
            (Csi, b'?') => CsiQ,
            (Csi, b'5') => Csi5,
            (CsiQ, b'5') => CsiQ5,
            (Csi, b'6') => Csi6,
            (CsiQ, b'6') => CsiQ6,
            (Csi5, b'n') => Idle,
            (CsiQ5, b'n') => Idle,
            (Csi6, b'n') => Idle,
            (CsiQ6, b'n') => Idle,
            _ => Idle,
        };

        matched
    }
}

// ============================================================================
// OSC color query scanner (OSC 10 ? / OSC 11 ?)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OscQuery {
    ForegroundColor,
    BackgroundColor,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum OscQueryScanState {
    #[default]
    Idle,
    Esc,
    Osc,
    Ps {
        value: u32,
    },
    AfterSemicolon {
        ps: u32,
    },
    Query {
        ps: u32,
    },
    StEscape {
        ps: u32,
    },
}

impl OscQueryScanState {
    pub(super) fn advance(&mut self, b: u8) -> Option<OscQuery> {
        use OscQueryScanState::*;

        let color_for_ps = |ps: u32| match ps {
            v if v == OSC_DEFAULT_FG => Some(OscQuery::ForegroundColor),
            v if v == OSC_DEFAULT_BG => Some(OscQuery::BackgroundColor),
            _ => None,
        };
        let matched = match (*self, b) {
            (Query { ps }, ansi::BEL) => color_for_ps(ps),
            (StEscape { ps }, b'\\') => color_for_ps(ps),
            _ => None,
        };

        *self = match (*self, b) {
            (Query { ps }, 0x1b) => StEscape { ps },
            (_, 0x1b) => Esc,
            (Esc, b']') => Osc,
            (Esc, _) => Idle,
            (Osc, d) if d.is_ascii_digit() => Ps {
                value: (d - b'0') as u32,
            },
            (Ps { value }, d) if d.is_ascii_digit() => Ps {
                value: value.saturating_mul(10).saturating_add((d - b'0') as u32),
            },
            (Ps { value }, b';') => value_to_after_semicolon_state(value),
            (Osc, _) | (Ps { .. }, _) => Idle,
            (AfterSemicolon { ps }, b'?') => Query { ps },
            (AfterSemicolon { .. }, _) => Idle,
            (Query { .. }, 0x07) => Idle,
            (Query { .. }, _) => Idle,
            (StEscape { .. }, b'\\') => Idle,
            (StEscape { .. }, _) => Idle,
            _ => Idle,
        };

        matched
    }
}

// ============================================================================
// OSC 133 scanner (FinalTerm / FTCS shell integration marks)
// ============================================================================

/// Stateful byte scanner that detects OSC 133 (FinalTerm / FTCS)
/// terminators across chunk boundaries. On terminator, returns the
/// parsed mark kind and optional exit code. Runs in lockstep with
/// `terminal.feed()` so the caller can capture cursor position *after*
/// the segment ending at the terminator — giving each mark the correct
/// `abs_y` (cursor position translated against current overflow).
#[derive(Clone, Debug, Default)]
pub(super) struct Osc133Scanner {
    state: Osc133State,
    payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Osc133State {
    #[default]
    Idle,
    Esc,
    OscBracket,
    Ps(u32),
    Skip,
    Payload,
    PayloadEsc,
}

impl Osc133Scanner {
    pub(super) fn advance(&mut self, b: u8) -> Option<(PromptMarkKind, Option<i32>)> {
        use Osc133State::*;
        match self.state {
            Idle => {
                if b == ansi::ESC {
                    self.state = Esc;
                }
                None
            }
            Esc => {
                self.state = if b == b']' { OscBracket } else { Idle };
                None
            }
            OscBracket => {
                if b.is_ascii_digit() {
                    self.state = Ps((b - b'0') as u32);
                } else {
                    self.state = Idle;
                }
                None
            }
            Ps(v) => {
                if b.is_ascii_digit() {
                    self.state = Ps(v.saturating_mul(10).saturating_add((b - b'0') as u32));
                    None
                } else if b == b';' {
                    if v == OSC_FTCS {
                        self.payload.clear();
                        self.state = Payload;
                    } else {
                        self.state = Skip;
                    }
                    None
                } else if b == ansi::BEL || b == ansi::ESC {
                    self.state = if b == ansi::ESC { PayloadEsc } else { Idle };
                    None
                } else {
                    self.state = Idle;
                    None
                }
            }
            Skip => {
                match b {
                    ansi::BEL => self.state = Idle,
                    ansi::ESC => self.state = PayloadEsc,
                    _ => {}
                }
                None
            }
            Payload => match b {
                ansi::BEL => {
                    let parsed = parse_osc133_payload(&self.payload);
                    self.payload.clear();
                    self.state = Idle;
                    parsed
                }
                ansi::ESC => {
                    self.state = PayloadEsc;
                    None
                }
                _ => {
                    if self.payload.len() < OSC133_PAYLOAD_CAP {
                        self.payload.push(b);
                    }
                    None
                }
            },
            PayloadEsc => {
                let parsed = if b == ansi::ST_FINAL && !self.payload.is_empty() {
                    parse_osc133_payload(&self.payload)
                } else {
                    None
                };
                self.payload.clear();
                self.state = Idle;
                parsed
            }
        }
    }
}

// ============================================================================
// Capability query scanner (Primary/Secondary/Tertiary DA, XTVERSION,
// DECRQM, Kitty keyboard)
// ============================================================================

#[derive(Clone, Copy, Debug, Default)]
enum CapState {
    #[default]
    Idle,
    Esc,
    Csi,
    CsiGt,
    CsiEq,
    CsiNum(u32),
    CsiGtNum(u32),
    CsiQ,
    CsiQNum(u32),
    CsiQNumDollar(u32),
}

#[derive(Clone, Copy, Debug)]
pub(super) enum CapabilityQuery {
    PrimaryDa,
    SecondaryDa,
    TertiaryDa,
    XtVersion,
    Decrqm(u32),
    KittyKeyboard,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct CapabilityScanner {
    state: CapState,
}

impl CapabilityScanner {
    pub(super) fn advance(&mut self, b: u8) -> Option<CapabilityQuery> {
        use CapState::*;
        let cur = self.state;

        let result = match (cur, b) {
            // Primary DA: only ESC[c and ESC[0c (Ps=0 is the only defined value).
            (Csi, b'c') | (CsiNum(0), b'c') => Some(CapabilityQuery::PrimaryDa),
            // Secondary DA: only ESC[>c and ESC[>0c.
            (CsiGt, b'c') | (CsiGtNum(0), b'c') => Some(CapabilityQuery::SecondaryDa),
            (CsiGt, b'q') | (CsiGtNum(0), b'q') => Some(CapabilityQuery::XtVersion),
            (CsiEq, b'c') => Some(CapabilityQuery::TertiaryDa),
            (CsiQNumDollar(ps), b'p') => Some(CapabilityQuery::Decrqm(ps)),
            (CsiQ, b'u') => Some(CapabilityQuery::KittyKeyboard),
            _ => None,
        };

        self.state = match (cur, b) {
            (_, 0x1b) => Esc,
            (Esc, b'[') => Csi,
            (Esc, _) => Idle,
            (Csi, b'>') => CsiGt,
            (Csi, b'=') => CsiEq,
            (Csi, b'?') => CsiQ,
            (Csi, d) if d.is_ascii_digit() => CsiNum((d - b'0') as u32),
            (CsiNum(n), d) if d.is_ascii_digit() => {
                CsiNum(n.saturating_mul(10).saturating_add((d - b'0') as u32))
            }
            (CsiGt, d) if d.is_ascii_digit() => CsiGtNum((d - b'0') as u32),
            (CsiGtNum(n), d) if d.is_ascii_digit() => {
                CsiGtNum(n.saturating_mul(10).saturating_add((d - b'0') as u32))
            }
            (CsiQ, d) if d.is_ascii_digit() => CsiQNum((d - b'0') as u32),
            (CsiQNum(n), d) if d.is_ascii_digit() => {
                CsiQNum(n.saturating_mul(10).saturating_add((d - b'0') as u32))
            }
            (CsiQNum(ps), b'$') => CsiQNumDollar(ps),
            _ => Idle,
        };

        result
    }
}

// ============================================================================
// XTGETTCAP scanner  (DCS + q <hex-names> ST)
// ============================================================================

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum XtGetTcapState {
    #[default]
    Idle,
    Esc,
    Dcs,
    DcsPlus,
    DcsPlusQ,
    DcsBody,
    DcsBodyEsc,
}

#[derive(Clone, Debug, Default)]
pub(super) struct XtGetTcapScanner {
    state: XtGetTcapState,
    body: Vec<u8>,
}

impl XtGetTcapScanner {
    pub(super) fn advance(&mut self, b: u8) -> Option<Vec<u8>> {
        use XtGetTcapState::*;
        let (next, result) = match self.state {
            Idle => (if b == ansi::ESC { Esc } else { Idle }, None),
            Esc => (
                match b {
                    b'P' => Dcs,
                    0x1b => Esc,
                    _ => Idle,
                },
                None,
            ),
            Dcs => (
                match b {
                    b'+' => DcsPlus,
                    0x1b => Esc,
                    _ => Idle,
                },
                None,
            ),
            DcsPlus => (
                match b {
                    b'q' => {
                        self.body.clear();
                        DcsPlusQ
                    }
                    0x1b => {
                        self.body.clear();
                        Esc
                    }
                    _ => Idle,
                },
                None,
            ),
            DcsPlusQ | DcsBody => match b {
                0x1b => (DcsBodyEsc, None),
                c if (0x20..0x7f).contains(&c) => {
                    if self.body.len() < XTGETTCAP_BODY_CAP {
                        self.body.push(c);
                    }
                    (DcsBody, None)
                }
                _ => {
                    self.body.clear();
                    (Idle, None)
                }
            },
            DcsBodyEsc => {
                if b == ansi::ST_FINAL {
                    let body = std::mem::take(&mut self.body);
                    (Idle, Some(body))
                } else {
                    self.body.clear();
                    (if b == ansi::ESC { Esc } else { Idle }, None)
                }
            }
        };
        self.state = next;
        result
    }
}

// ============================================================================
// XTGETTCAP response builder
// ============================================================================

/// Build a combined XTGETTCAP response for a semicolon-separated list of
/// hex-encoded capability names. Returns an empty vec if the body is empty
/// or malformed.
pub(super) fn build_xtgettcap_response(body: &[u8]) -> Vec<u8> {
    let body_str = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for cap_hex in body_str.split(';') {
        if cap_hex.is_empty() {
            continue;
        }
        let cap_name = decode_hex_cap_name(cap_hex);
        let value: Option<&str> = match cap_name.as_str() {
            "TN" | "name" => Some("daruda"),
            "RGB" => Some("1"),
            // Terminfo parameterized string for extended underline styles
            // (undercurl, dotted, dashed). %p1%d is replaced by the style
            // index (3=curl, 4=dot, 5=dash) via tparm(). Neovim / LSP
            // diagnostics use this capability to activate undercurl.
            "Smulx" => Some("\x1b[4:%p1%dm"),
            // 256-color palette support.
            "256color" => Some("1"),
            _ => None,
        };
        let resp = match value {
            Some(v) => ansi::xtgettcap_found(cap_hex, v),
            None => ansi::xtgettcap_not_found(cap_hex),
        };
        out.extend_from_slice(resp.as_bytes());
    }
    out
}

fn decode_hex_cap_name(hex: &str) -> String {
    let bytes = hex.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return hex.to_string();
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks(2) {
        let hi = (chunk[0] as char).to_digit(16);
        let lo = (chunk[1] as char).to_digit(16);
        match (hi, lo) {
            (Some(h), Some(l)) => out.push(((h << 4) | l) as u8),
            _ => return hex.to_string(),
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| hex.to_string())
}

fn value_to_after_semicolon_state(ps: u32) -> OscQueryScanState {
    if ps == OSC_DEFAULT_FG || ps == OSC_DEFAULT_BG {
        OscQueryScanState::AfterSemicolon { ps }
    } else {
        OscQueryScanState::Idle
    }
}

// ============================================================================
// OSC payload parsers
// ============================================================================

/// Parse an OSC 133 payload (the portion after `133;`) into a semantic
/// prompt event. Returns None for unknown subcommands or malformed
/// input. See FinalTerm FTCS spec / iTerm2 `VT100Terminal.m:4520-4616`.
pub(crate) fn parse_osc133_payload(payload: &[u8]) -> Option<(PromptMarkKind, Option<i32>)> {
    let s = std::str::from_utf8(payload).ok()?;
    // Support both `133;A` (caller strips the `133;`) and `A`/`D;1` forms.
    let rest = s.strip_prefix("133;").unwrap_or(s);
    let (head, tail) = match rest.find(';') {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    let head_byte = head.as_bytes().first().copied()?;
    if head.len() != 1 {
        return None;
    }
    let kind = match FtcsCommand::from_letter(head_byte)? {
        FtcsCommand::PromptStart => PromptMarkKind::PromptStart,
        FtcsCommand::CommandStart => PromptMarkKind::CommandStart,
        FtcsCommand::CommandExecuted => PromptMarkKind::CommandExecuted,
        FtcsCommand::CommandFinished => PromptMarkKind::CommandFinished,
        FtcsCommand::SemanticTextStart => PromptMarkKind::SemanticTextStart,
        FtcsCommand::SemanticTextEnd => PromptMarkKind::SemanticTextEnd,
    };
    let exit_code = match (kind, tail) {
        (PromptMarkKind::CommandFinished, Some(rest)) => {
            // Exit code is the first `;`-separated field after D.
            let head = rest.split(';').next()?;
            head.parse::<i32>().ok()
        }
        _ => None,
    };
    Some((kind, exit_code))
}

/// Aggregated outputs of a single OSC scan pass. Each slot retains
/// the last-winning value seen during the pass; the caller commits
/// non-`None` slots back onto the session after the loop.
#[derive(Default)]
pub(super) struct OscDispatch {
    pub(super) title: Option<String>,
    pub(super) clipboard: Option<String>,
    pub(super) cwd: Option<String>,
    pub(super) attention: Option<AttentionKind>,
    pub(super) notification: Option<NotificationRequest>,
    /// `OSC 1337 ; ClearScrollback` arrived during this scan. The
    /// caller drains this into a flag so the actual scrollback wipe
    /// (`ESC [ 3 J` re-fed to ghostty_vt) happens *after* the chunk's
    /// own bytes are processed — a clear that races ahead of the
    /// shell's pre-clear output would erase that output too.
    pub(super) clear_scrollback: bool,
}

/// Dispatch a parsed OSC payload into the matching slot of `out`.
pub(super) fn commit_osc_payload(ps: u32, payload: &[u8], out: &mut OscDispatch, track_cwd: bool) {
    match ps {
        v if v == OSC_TITLE_ICON_AND_WINDOW || v == OSC_TITLE_WINDOW => {
            out.title = Some(String::from_utf8_lossy(payload).into_owned());
        }
        v if v == OSC_CWD && track_cwd => {
            if let Some(path) = parse_osc7_path(payload) {
                out.cwd = Some(path);
            }
        }
        v if v == OSC_CLIPBOARD => {
            if let Some(c) = decode_osc_52(payload) {
                out.clipboard = Some(c);
            }
        }
        v if v == OSC_ITERM2 => {
            dispatch_osc_iterm2(payload, out);
        }
        v if v == OSC_NOTIFICATION && !payload.is_empty() => {
            out.notification = Some(NotificationRequest::Osc9 {
                body: String::from_utf8_lossy(payload).into_owned(),
            });
        }
        v if v == OSC_NOTIFY_RXVT => {
            if let Some(req) = parse_osc_777(payload) {
                out.notification = Some(req);
            }
        }
        _ => {}
    }
}

/// Parse `notify ; <title> ; <body>` — the only OSC 777 subcommand
/// daruda surfaces. Other subcommands (`preexec`, …) fall through.
fn parse_osc_777(payload: &[u8]) -> Option<NotificationRequest> {
    let s = std::str::from_utf8(payload).ok()?;
    let mut parts = s.splitn(3, ';');
    let sub = parts.next()?;
    if !sub.eq_ignore_ascii_case("notify") {
        return None;
    }
    let title = parts.next()?.to_owned();
    let body = parts.next().unwrap_or("").to_owned();
    Some(NotificationRequest::Osc777 { title, body })
}

/// Dispatch an OSC 1337 payload across the three sub-commands daruda
/// surfaces today: `RequestAttention=<value>`, `ClearScrollback`
/// (bare key), and `Copy=<selection>:<base64>`. Anything else
/// (file transfer, variable reporting, …) falls through silently
/// — iTerm2's proprietary set is large and we add support as
/// individual features land.
fn dispatch_osc_iterm2(payload: &[u8], out: &mut OscDispatch) {
    let Ok(s) = std::str::from_utf8(payload) else {
        return;
    };
    match s.split_once('=') {
        // Bare key — `ClearScrollback` is the only supported one.
        None => {
            if s.trim().eq_ignore_ascii_case("ClearScrollback") {
                out.clear_scrollback = true;
            }
        }
        Some((key, value)) => {
            let key_trim = key.trim();
            if key_trim.eq_ignore_ascii_case("RequestAttention") {
                if let Some(kind) = AttentionKind::from_value(value) {
                    out.attention = Some(kind);
                }
            } else if key_trim.eq_ignore_ascii_case("Copy")
                && let Some(text) = decode_iterm2_copy_value(value)
            {
                out.clipboard = Some(text);
            }
        }
    }
}

/// Parse the `<selection>:<base64>` form of `OSC 1337 ; Copy=…`.
/// `<selection>` is the same prefix as OSC 52 (e.g. `c` for the
/// system clipboard) and is currently honoured as advisory — we
/// always write to the system clipboard regardless. Empty or
/// missing `<selection>` is allowed (some shells emit
/// `Copy=:base64`).
fn decode_iterm2_copy_value(value: &str) -> Option<String> {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    let (_selection, b64) = value.split_once(':')?;
    let bytes = STANDARD.decode(b64.as_bytes()).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Parse an OSC 7 payload of form `file://hostname/path` into the local
/// path. Returns None for malformed input. Percent-decoding is applied to
/// the path so encoded spaces (`%20`) are normalized.
pub(super) fn parse_osc7_path(payload: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(payload).ok()?;
    let after_scheme = s.strip_prefix(OSC7_FILE_SCHEME)?;
    // Drop the hostname segment (between scheme and the path's leading `/`).
    let path_start = after_scheme.find('/')?;
    let raw = &after_scheme[path_start..];
    Some(percent_decode(raw))
}

pub(super) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn decode_osc_52(payload: &[u8]) -> Option<String> {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;

    let mut split = payload.splitn(2, |b| *b == b';');
    let selection = split.next()?;
    let data = split.next()?;

    if !selection.contains(&b'c') {
        return None;
    }
    if data.is_empty() {
        return None;
    }

    let decoded = STANDARD.decode(data).ok()?;
    Some(String::from_utf8_lossy(&decoded).into_owned())
}
