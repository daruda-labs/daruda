//! Protocol content decoding and bounded output blocks.

use super::*;

/// Max bytes of tool-output text carried into the render model. Larger text is
/// truncated at a char boundary so expanding a tool card can't feed megabytes
/// to the markdown renderer (the expand-freeze bug). Not user-tunable yet.
pub(super) const MAX_TOOL_OUTPUT_TEXT_BYTES: usize = 64 * 1024;

/// Max bytes of a tool-output image's base64 `data` carried into the render
/// model as an `Image` block. Same trust-boundary rationale as
/// [`MAX_TOOL_OUTPUT_TEXT_BYTES`]: nothing in the protocol bounds what an
/// adapter sends, and unlike text, an oversized image can't be truncated in
/// place (a truncated base64 string is either invalid or decodes to a corrupt
/// image) — so a payload over this cap is remapped to a `Media` descriptor
/// (via [`media_block`]) instead, which is cheap to carry, rather than an
/// `Image` block, which is retained, cloned, decoded, and re-hashed every
/// frame. 8 MiB of base64 (~6 MB decoded) is generous for a normal
/// screenshot/PNG; only pathological payloads fall back.
pub(super) const MAX_TOOL_OUTPUT_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// What one pass over a tool call's `content` yielded.
pub(super) struct SplitContent {
    pub(super) diffs: Vec<DiffView>,
    pub(super) output: Vec<ToolOutputBlock>,
    /// An embedded terminal block was present. It renders nothing by itself, so
    /// the caller has to recover the bytes from another channel.
    pub(super) saw_terminal: bool,
}

/// Partition tool-call content into diffs and typed output blocks.
///
/// An embedded terminal block carries no content of its own — it is a handle to
/// a terminal this client never created (daruda implements no `terminal/*`
/// method) — so it only sets [`SplitContent::saw_terminal`]; recovering its
/// bytes is [`fold_output`]'s job, since they arrive on a different
/// notification than the handle. Any future content kind [`output_block_of`]
/// doesn't recognize is dropped silently.
pub(super) fn split_content(content: &[ToolCallContent]) -> SplitContent {
    let mut diffs = Vec::new();
    let mut output = Vec::new();
    let mut saw_terminal = false;
    for block in content {
        match block {
            ToolCallContent::Diff(diff) => diffs.push(DiffView {
                path: diff.path.clone(),
                old_text: diff.old_text.clone(),
                new_text: diff.new_text.clone(),
            }),
            ToolCallContent::Content(c) => {
                if let Some(out) = output_block_of(&c.content) {
                    output.push(out);
                }
            }
            ToolCallContent::Terminal(_) => saw_terminal = true,
            // Any future content kind is not rendered.
            _ => {}
        }
    }
    SplitContent {
        diffs,
        output,
        saw_terminal,
    }
}

/// Fold one event's output channels into a tool call's body, in priority order:
/// typed `content` blocks, then the adapter's `_meta` output sideband, then
/// `raw_output`. Only the first channel that carries something applies, and a
/// channel that carries nothing leaves the body as it stands.
///
/// The "leaves it as it stands" half is what makes claude-agent-acp's shell
/// lifecycle work: the captured bytes and the completion (`terminal_exit` +
/// an unfenced `rawOutput`) arrive as two separate `tool_call_update`s, so
/// overwriting from the second would both blank the recovered [`ToolOutputBlock::RawText`]
/// and push raw shell bytes through the markdown renderer.
/// The part of a `content` field [`fold_output`] folds: what it rendered to,
/// and whether a terminal handle was among it.
pub(super) struct ContentBody {
    pub(super) blocks: Vec<ToolOutputBlock>,
    pub(super) terminal_handle: bool,
}

pub(super) fn fold_output(
    output: &mut Vec<ToolOutputBlock>,
    content: Option<ContentBody>,
    sideband: Option<String>,
    raw_output: &Option<serde_json::Value>,
) {
    // `content` is replace semantics (the schema calls the field "Replace the
    // content collection"), so a present-but-unrenderable one — an empty array,
    // or diffs only — must clear a body an earlier update filled. The single
    // exception is a bare terminal handle: its bytes ride a content-less
    // notification of their own, so replacing on it would blank them.
    if let Some(body) = content
        && !(body.blocks.is_empty() && body.terminal_handle)
    {
        *output = body.blocks;
    }
    if output.is_empty()
        && let Some(blocks) = sideband
            .map(bounded_raw_text_blocks)
            .filter(|blocks| !blocks.is_empty())
    {
        *output = blocks;
    }
    push_raw_output_fallback(output, raw_output);
}

/// Cap `s` at a UTF-8 char boundary when it exceeds
/// `MAX_TOOL_OUTPUT_TEXT_BYTES`, returning the kept text plus the original byte
/// length when it was cut (so the renderer can show a marker). Shared by both
/// text block kinds so one cap governs everything the model carries.
pub(super) fn bounded(s: String) -> (String, Option<usize>) {
    let cap = MAX_TOOL_OUTPUT_TEXT_BYTES;
    if s.len() <= cap {
        return (s, None);
    }
    let original_len = s.len();
    let boundary = s.floor_char_boundary(cap);
    let mut text = s;
    text.truncate(boundary);
    (text, Some(original_len))
}

/// Build a bounded markdown `Text` block.
pub(super) fn bounded_text(s: String) -> ToolOutputBlock {
    let (text, truncated_from) = bounded(s);
    ToolOutputBlock::Text {
        text,
        truncated_from,
    }
}

/// Build a bounded `RawText` block — verbatim shell output, never markdown.
pub(super) fn bounded_raw_text(s: String) -> ToolOutputBlock {
    let (text, truncated_from) = bounded(s);
    ToolOutputBlock::RawText {
        text,
        truncated_from,
    }
}

/// A bounded `RawText` block wrapped in a `Vec`, or an empty `Vec` when the
/// command printed nothing, so a silent command adds no empty block.
pub(super) fn bounded_raw_text_blocks(text: String) -> Vec<ToolOutputBlock> {
    if text.trim().is_empty() {
        Vec::new()
    } else {
        vec![bounded_raw_text(text)]
    }
}

/// Map a content block to a renderable output block. `None` for empty text and
/// for any future content kind the protocol adds that this build doesn't know
/// about (`ContentBlock` is `#[non_exhaustive]`).
pub(super) fn output_block_of(block: &ContentBlock) -> Option<ToolOutputBlock> {
    match block {
        ContentBlock::Text(t) if !t.text.is_empty() => Some(bounded_text(t.text.clone())),
        ContentBlock::Image(img) => Some(if img.data.len() > MAX_TOOL_OUTPUT_IMAGE_BYTES {
            media_block(img.mime_type.clone(), &img.data)
        } else {
            ToolOutputBlock::Image {
                data: img.data.clone(),
                mime: img.mime_type.clone(),
            }
        }),
        ContentBlock::Audio(a) => Some(media_block(a.mime_type.clone(), &a.data)),
        ContentBlock::ResourceLink(rl) => Some(ToolOutputBlock::ResourceLink {
            uri: rl.uri.clone(),
            // Prefer the human title; the `name` field is always present.
            name: rl.title.clone().unwrap_or_else(|| rl.name.clone()),
            mime: rl.mime_type.clone(),
        }),
        ContentBlock::Resource(er) => match &er.resource {
            EmbeddedResourceResource::TextResourceContents(t) if !t.text.trim().is_empty() => {
                Some(bounded_text(t.text.clone()))
            }
            EmbeddedResourceResource::TextResourceContents(_) => None,
            EmbeddedResourceResource::BlobResourceContents(b) => Some(media_block(
                b.mime_type.clone().unwrap_or_default(),
                &b.blob,
            )),
            // `EmbeddedResourceResource` is `#[non_exhaustive]`.
            #[allow(unreachable_patterns)]
            _ => None,
        },
        _ => None,
    }
}

/// Estimated decoded byte size of a base64 string (ignoring padding/whitespace),
/// good enough for a human-readable descriptor. `daruda_acp` carries no base64
/// dependency, so this is arithmetic only — the real decode (and image
/// rasterization) happens at the app render boundary.
pub(super) fn est_decoded_len(b64: &str) -> usize {
    b64.len() / 4 * 3
}

/// Build a `Media` descriptor block — the shared shape for every non-rendered
/// binary payload (audio, embedded blob, and an image over
/// [`MAX_TOOL_OUTPUT_IMAGE_BYTES`]), so every call site computes `byte_len` the
/// same way and orders the fields the same way.
pub(super) fn media_block(mime: String, data: &str) -> ToolOutputBlock {
    ToolOutputBlock::Media {
        mime,
        byte_len: est_decoded_len(data),
    }
}

/// Append fallback output blocks derived from a tool call's `raw_output`, but
/// only when no higher-priority channel produced renderable output (see
/// [`fold_output`]). Adapters that report results through an embedded terminal
/// with no output sideband, or *only* in `raw_output` — codex-acp streams a
/// shell command's output through a terminal and repeats it in `raw_output`, and
/// its MCP calls carry results solely there — would otherwise render an empty
/// card. Claude-style adapters embed the same content as `content` blocks, so
/// their `output` is already non-empty and this is a no-op (no duplication).
pub(super) fn push_raw_output_fallback(
    output: &mut Vec<ToolOutputBlock>,
    raw_output: &Option<serde_json::Value>,
) {
    if !output.is_empty() {
        return;
    }
    if let Some(raw) = raw_output {
        output.extend(raw_output_blocks(raw));
    }
}

/// Render a `raw_output` value as output blocks. codex-acp's command execution
/// stores the human-facing output as a `formatted_output` string, surfaced as
/// [`ToolOutputBlock::RawText`] (highest priority — codex-acp relies on it):
/// those are shell bytes, not markdown, so they must render verbatim. Some
/// adapters use the same shape but call the field `output`; treat that as the
/// same human-facing stream rather than showing the JSON wrapper. A bare string
/// is also raw output. A JSON **array** is Anthropic's raw content-block shape
/// (e.g. `[{"type":"image",...}, {"type":"text",...}]`) — each element is
/// parsed via [`raw_content_block`], in order, and recognized elements (image /
/// audio / text / embedded resource) are kept while unrecognized ones are
/// skipped; if *none* of the array's elements are recognized the whole array
/// falls back to pretty JSON below (so a genuinely unrelated array isn't
/// silently dropped). A bare JSON object that is itself one recognized content
/// block parses to that one block. Anything else (a structured object such as an
/// MCP `{ result, error }`, or an array/object with nothing recognizable)
/// renders as pretty JSON, still as raw text so the app shows it in the bounded
/// editor embed instead of the markdown path. Returns an empty `Vec` when there
/// is no visible text.
pub(super) fn raw_output_blocks(raw: &serde_json::Value) -> Vec<ToolOutputBlock> {
    if let Some(s) = raw
        .get("formatted_output")
        .and_then(serde_json::Value::as_str)
    {
        // codex's command-execution shape (`{ formatted_output, exit_code }` —
        // the same key `DefaultAdapter::command_exit` requires before it badges
        // an exit), so these are shell bytes: a `#` or `---` the command printed
        // is literal, not a heading or a horizontal rule.
        return bounded_raw_text_blocks(s.to_string());
    }
    if let Some(s) = raw.get("output").and_then(serde_json::Value::as_str) {
        return bounded_raw_text_blocks(s.to_string());
    }
    match raw {
        serde_json::Value::String(s) => return bounded_raw_text_blocks(s.clone()),
        serde_json::Value::Array(elements) => {
            let recognized: Vec<ToolOutputBlock> =
                elements.iter().filter_map(raw_content_block).collect();
            if !recognized.is_empty() {
                return recognized;
            }
            // No element recognized — fall through to the pretty-JSON fallback
            // below rather than silently dropping the whole array.
        }
        _ => {
            if let Some(block) = raw_content_block(raw) {
                return vec![block];
            }
        }
    }
    match serde_json::to_string_pretty(raw) {
        Ok(text) => bounded_raw_text_blocks(text),
        Err(_) => Vec::new(),
    }
}

/// Parse one raw JSON value as a single content block — either one element of
/// a `rawOutput` array, or a bare `rawOutput` object that is itself a content
/// block. Recognizes Anthropic's raw shape: `type` discriminates
/// `"text"` / `"image"` / `"audio"` / `"resource"`, with image/audio payload
/// nested under `source.data` / `source.media_type` (Anthropic's own shape) or
/// falling back to a flat `data` / `media_type` / `mime_type` (defensive, in
/// case another adapter sends the flatter ACP-like shape instead). Returns
/// `None` for a value with no recognized `type`, or a recognized `type` missing
/// its required payload field.
pub(super) fn raw_content_block(el: &serde_json::Value) -> Option<ToolOutputBlock> {
    let ty = el.get("type").and_then(serde_json::Value::as_str)?;
    match ty {
        "text" => {
            let text = el.get("text").and_then(serde_json::Value::as_str)?;
            (!text.trim().is_empty()).then(|| bounded_text(text.to_string()))
        }
        "image" => {
            let data = raw_media_data(el)?;
            let mime = raw_media_mime(el);
            Some(if data.len() > MAX_TOOL_OUTPUT_IMAGE_BYTES {
                media_block(mime, &data)
            } else {
                ToolOutputBlock::Image { data, mime }
            })
        }
        "audio" => {
            let data = raw_media_data(el)?;
            Some(media_block(raw_media_mime(el), &data))
        }
        "resource" => {
            let resource = el.get("resource")?;
            if let Some(blob) = resource.get("blob").and_then(serde_json::Value::as_str) {
                let mime = resource
                    .get("mime_type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Some(media_block(mime, blob))
            } else {
                let text = resource.get("text").and_then(serde_json::Value::as_str)?;
                (!text.trim().is_empty()).then(|| bounded_text(text.to_string()))
            }
        }
        _ => None,
    }
}

/// The base64 payload of a raw image/audio content-block element: Anthropic's
/// nested `source.data`, falling back to a flat `data`.
pub(super) fn raw_media_data(el: &serde_json::Value) -> Option<String> {
    el.get("source")
        .and_then(|s| s.get("data"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| el.get("data").and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

/// The MIME type of a raw image/audio content-block element: Anthropic's
/// nested `source.media_type`, falling back to a flat `media_type` or
/// `mime_type`. Empty string when none is present — the source omitted it.
pub(super) fn raw_media_mime(el: &serde_json::Value) -> String {
    el.get("source")
        .and_then(|s| s.get("media_type"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| el.get("media_type").and_then(serde_json::Value::as_str))
        .or_else(|| el.get("mime_type").and_then(serde_json::Value::as_str))
        .unwrap_or_default()
        .to_string()
}
