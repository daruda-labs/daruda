# Security Policy

## Supported versions

Only the latest release receives security fixes. There are no backport guarantees for older versions while the project is in the v0.x series.

| Version | Supported |
|---------|-----------|
| Latest release | ✓ |
| Older releases | — |

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Report security issues by email to:

**admin@daruda.ai**

Include as much of the following as possible:

- A description of the vulnerability and its potential impact
- Steps to reproduce or a proof-of-concept
- Affected version(s) and platform
- Any suggested fix, if you have one

You will receive an acknowledgement within **3 business days**. If the issue is confirmed, a fix will be prioritized and a patched release will be made available before public disclosure.

We follow a **90-day coordinated disclosure** policy. If you have a different timeline requirement, please mention it in your report.

## Scope

daruda is a local macOS terminal application. The following areas are particularly relevant from a security standpoint:

| Area | Notes |
|------|-------|
| **PTY / process execution** | daruda spawns shell processes via `portable-pty`. Malicious terminal escape sequences that could trigger unintended process execution are in scope. |
| **OSC / escape sequence handling** | Sequences that write to the clipboard (OSC 52), trigger system notifications (OSC 9 / OSC 777 / OSC 1337), or manipulate application state are in scope. |
| **Keychain access** | daruda reads the Claude Code OAuth token from macOS Keychain via the `security` CLI. Any path that leaks or misuses this token is in scope. |
| **File system access** | daruda reads `~/.claude/`, `~/.daruda/`, and project directories. Path traversal or unintended file access is in scope. |
| **Config / hook injection** | daruda installs hooks into `~/.claude/settings.json`. Attacks that abuse this installation path are in scope. |
| **Network** | daruda makes HTTPS requests to `api.anthropic.com` to fetch token usage. TLS verification failures or credential leakage are in scope. |

The following are **out of scope**:

- Vulnerabilities requiring physical access to the machine
- Social engineering attacks against the maintainer
- Issues in third-party dependencies not directly exploitable through daruda (report these to the upstream project)
- Missing security headers or features unrelated to the app's attack surface
