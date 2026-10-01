# Shell IPC and CSP

PR #24 (the desktop UI) adopts this path. Its `src-tauri` crate is merged into `apps/desktop/src-tauri`. The commands, the socket ops, and the CSP string below are the surface to build on.

## Content Security Policy

The HTTP responses and `apps/desktop/src-tauri/tauri.conf.json` use the same policy. It has no `*` source.

```
default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost http://127.0.0.1:8787 http://localhost:8787; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'
```

`connect-src` allows the Tauri IPC scheme and the loopback daemon. It does not allow another origin.

## Tauri commands

| Command | Windows | What it sends |
|---|---|---|
| `sign_decision` | `card` | `op=decide` |
| `undo_decision` | `card` | `op=undo` |
| `set_secret` | `main`, `settings` | `op=secret` |

`window.label()` is used only inside the shell process, to choose which per-launch secret file to read: `<data>.window-card`, `<data>.window-main`, or `<data>.window-settings` (mode 0600). The JSON body does not contain a `window` field. The daemon maps `window_secret` back to the label.

## Socket

Unix: `<data>.shell.sock`, mode 0600. The daemon reads `SO_PEERCRED` and refuses a peer whose uid is not the daemon's euid. A failed `getsockopt` fails closed.

Windows: a named pipe `\\.\pipe\dasdevbot-<32 hex of blake3(absolute data path)>` (`shell_pipe_name`). There is no TCP listener.
- The pipe's DACL is protected and grants `GENERIC_ALL` only to the daemon's user SID (`D:P(A;;GA;;;<sid>)`), and it has `PIPE_REJECT_REMOTE_CLIENTS`.
- The first instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a process that grabbed the name first makes the daemon fail to start.
- For each connection the daemon calls `GetNamedPipeClientProcessId`, opens that process's token, and compares its user SID with its own (`EqualSid`). Any failure means a foreign peer, which gets `unauthorized`.
- The client opens the pipe with `SECURITY_IDENTIFICATION`, so the server can't impersonate it.
- `write_private` applies the same owner-only protected DACL to `<data>.token`, `<data>.window-*`, the audit key and the tip sidecar.

Known limits on Windows:
- The client doesn't check who the pipe server is. If the daemon is down, a same-user squatter could answer.
- A pid can be reused between `GetNamedPipeClientProcessId` and `OpenProcess`. That fails closed, or checks the reusing process, and the ACL already restricts who can connect.
- Elevation and integrity level aren't compared.
- Only the same-user pipe path has run, and only under Wine on Linux (test `the_named_pipe_serves_the_same_user`). The squatter refusal and the ACLs need a real Windows host.

One JSON line in, one JSON line out: `{"ok":true,"result":...}` or `{"ok":false,"error":...}`.

Every line carries `token`, the bearer from `<data>.token`. The compare is constant-time (`subtle`). A length mismatch is a rejection.

## Ops

`decide` and `undo` fields: `approval_id`, `decision` (decide only), `reason` (optional), `window_secret`, and for an external card `client_signature` plus `client_nonce`.

`prepare` issues the nonce an external signature must cover. Fields: `approval_id`, `decision`, `reason`, `purpose` (`decision` or `undo`), `window_secret`. The result is `{signature_required, tier, nonce, message, prompt, hello_enrolled}`. Internal cards return `{signature_required:false, tier:"internal"}` and do not get a nonce. The `prompt` string is `{decision} {action} on {target}`.

`hello-enroll` asks Windows to open or create the non-exportable RSA-2048 key `dasdevbot-approval` (`KeyCredentialManager`). It stores only `(blob_type = X509SubjectPublicKeyInfo, SPKI)` in `hello_public_key`, writes `hello.enrolled`, and pins the fingerprint in the audit tip sidecar. A different key is refused until a `hello-reset`. The private key never leaves the platform. Other targets return `external tier is denied without Windows Hello`.

`secret` fields: `name`, `value`, `window_secret`. The daemon audits `secret.set` before the keyring write.

`hello-reset` fields: `reason` (non-empty), `window_secret` (must map to `settings`). The daemon asks the user to verify "reset the Windows Hello approval key", writes `hello.reset`, and then clears the enrolled key and its pin. A new key can enroll only after this.

## External versus internal

**Phase 1 hard-denies external cards.** `decide`, `undo` and `prepare` refuse them for every signature, because `EXTERNAL_TIER_ENABLED` is `false`. The flow below is the design that is re-enabled after H1, H2 and `docs/hello-hardware-test.md` pass.

Internal tiers stay on the Phase 1 Ed25519 `approval-key` seed. The daemon signs and verifies that seed, so the signature proves only same-uid. That is accepted for Phase 1 and revisited in Phase 2. External cards never use it.

On Windows, `sign_decision` / `undo_decision` send `decide` or `undo` first. If the daemon answers `external tier requires a Windows Hello signature`, the command calls `prepare`, enrolls when `hello_enrolled` is false, signs `message` with `sign_approval_message` (Hello consent uses `prompt`, then `RequestSignAsync`), and submits `client_signature` and `client_nonce`. The daemon checks the RSA PKCS#1 v1.5 / SHA-256 signature with CNG against the pinned public key and consumes the nonce. The daemon doesn't show its own Hello prompt.

On Linux and every other target the stub denies the external tier. The seed cannot approve it.

HTTP `POST /v1/approvals/{id}/decision` and `/undo` stay 403 `{"error":"approval decisions are Tauri IPC only"}`.
