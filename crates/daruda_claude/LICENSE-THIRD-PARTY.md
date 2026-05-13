# Third-Party Licenses

This crate ports code from the projects below. Each entry lists the
upstream source, license, and the daruda files that incorporate the
ported code.

## c9watch

- Upstream: <https://github.com/minchenlee/c9watch>
- License: MIT
- Copyright (c) 2026 Minchen Lee

The MIT license text is reproduced at the bottom of this file.

The following daruda files are derived from c9watch sources:

| daruda file | c9watch source | lines |
|-------------|---------------|-------|
| `src/jsonl/parser.rs` | `src-tauri/src/session/parser.rs` | 34-240 (SessionEntry, MessageContent, UserMessage) |
| `src/jsonl/tail.rs` | `src-tauri/src/session/parser.rs` | 255-303 (read_last_n_lines reverse-read) |
| `src/jsonl/fsm.rs` | `src-tauri/src/session/status.rs` | 38-238 (determine_status + helpers) |
| `src/jsonl/permissions.rs` | `src-tauri/src/session/permissions.rs` | 1-208 (allow patterns + auto-approve whitelist) |

Modifications:
- Adapted to daruda's `SessionStatus` enum (Working / NeedsAttention /
  Idle / Connecting). c9watch's `WaitingForInput` maps to `Idle`.
- Removed sysinfo-based process discovery (`detector.rs`) — daruda owns
  its own PTY and matches sessions via cwd/transcript_path from hook
  payloads.

---

## MIT License (c9watch)

```
MIT License

Copyright (c) 2026 Minchen Lee

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
