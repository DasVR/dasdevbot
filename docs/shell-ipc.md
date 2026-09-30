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

Windows: `127.0.0.1` on an ephemeral port written to `<data>.shell.port`. There is no peer-cred check. The per-launch window secret and the bearer are the authentication.

One JSON line in, one JSON line out: `{"ok":true,"result":...}` or `{"ok":false,"error":...}`.

Every line carries `token`, the bearer from `<data>.token`. The compare is constant-time (`subtle`). A length mismatch is a rejection.

## Ops

`decide` and `undo` fields: `approval_id`, `decision` (decide only), `reason` (optional), `window_secret`, and for an external card `client_signature` plus `client_nonce`.

`prepare` issues the nonce an external signature must cover. Fields: `approval_id`, `decision`, `reason`, `purpose` (`decision` or `undo`), `window_secret`. The result is `{signature_required, tier, nonce, message, prompt, hello_enrolled}`. Internal cards return `{signature_required:false, tier:"internal"}` and do not get a nonce. The `prompt` string is `{decision} {action} on {target}`.

`hello-enroll` asks Windows to open or create the non-exportable key `dasdevbot-approval` (`KeyCredentialManager`) and stores only `(blob_type, public_key)` in `hello_public_key`. The private key never leaves the platform. Other targets return `external tier is denied without Windows Hello`.

`secret` fields: `name`, `value`, `window_secret`. The daemon audits `secret.set` before the keyring write.

## External versus internal

Internal tiers stay on the Phase 1 Ed25519 `approval-key` seed. The daemon signs and verifies that seed. External cards never use it.

On Windows, `sign_decision` / `undo_decision` send `decide` or `undo` first. If the daemon answers `external tier requires a Windows Hello signature`, the command calls `prepare`, enrolls when `hello_enrolled` is false, signs `message` with `sign_approval_message` (Hello consent uses `prompt`, then `RequestSignAsync`), and submits `client_signature` and `client_nonce`. The daemon checks the signature with CNG against the stored public key and consumes the nonce. A second Hello prompt can appear at commit.

On Linux and every other target the stub denies the external tier. The seed cannot approve it.

HTTP `POST /v1/approvals/{id}/decision` and `/undo` stay 403 `{"error":"approval decisions are Tauri IPC only"}`.
