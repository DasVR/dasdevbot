# PR #25: live Ubuntu check (real claude 2.1.285, run as the dasdevbot user)

Security Director sign-off needs every item below to pass.

1. Version and hash.
   - `claude --version` prints 2.1.285.
   - Its SHA-256 matches the recorded value for /usr/local/bin/claude, or for cli.js plus /usr/bin/node on an npm install.
   - The resolved paths are root-owned, and their parent dirs aren't writable by dasdevbot.
2. 2.1.285 accepts every flag in the argv: `--tools ""`, `--disallowedTools "*"`, `--strict-mcp-config --mcp-config '{"mcpServers":{}}'`, `--max-turns 1`, `--setting-sources project,local`, `--settings '{"disableAllHooks":true,"permissions":{"disableAutoMode":"disable"}}'`, `--permission-mode dontAsk`, `--system-prompt-file`, `--disable-slash-commands`, `--no-session-persistence`, `--safe-mode`, `--restricted`. The login under `CLAUDE_CONFIG_DIR=/var/lib/dasdevbot/claude-config` works with `--restricted` and `--safe-mode`.
3. Hooks and project files don't fire. Run this item offline (for example under `unshare -rn`), so no call reaches Claude.
   - The ignored test `local_claude_does_not_run_a_hostile_home_hook` passes.
   - A hook in /var/lib/dasdevbot/.claude/settings.json, /var/lib/dasdevbot/claude-config/settings.json, /var/lib/dasdevbot/claude-cwd/.claude/settings.json or /var/lib/dasdevbot/claude-cwd/.claude/settings.local.json doesn't fire, and neither does an `apiKeyHelper` in any of them.
   - A CLAUDE.md or .claude/settings.json placed in /tmp, or in any ancestor of the cwd, isn't loaded.
4. The ~/.claude/CLAUDE.md under claude_home isn't loaded, or Arriq explicitly accepts that it is.
5. With a prompt that asks for a tool, the stream shows no tool_use. If one appears, provider.tool_use_blocked{job_id, cli_version} is written and no process in the group survives (`ps -g`).
6. A captured real rate_limit_event parses: the field names and status values match, and the log line doesn't show `status=unknown` (an unknown status pauses the job). Record whether utilization can exceed 1.0. An unknown status with no `resetsAt` re-polls every 60 s; the cap and backoff are tracked in #31.
7. The service starts under the hardened unit, with ProtectHome and PrivateTmp. The login works, and /var/lib/dasdevbot is mode 0700 and owned by dasdevbot.
8. There's no auto-update: DISABLE_AUTOUPDATER and DISABLE_UPDATES are honored, and the binary hash is unchanged after 24 h.
9. `smoke-model --provider claude-cli --claude-home /var/lib/dasdevbot` returns ok, and the prompt text never appears in `journalctl -u dasdevbotd`.
10. memfd_create and exec via /proc/self/fd work under SystemCallFilter=@system-service, ProtectSystem=strict and PrivateDevices.
11. The native claude 2.1.285 binary runs from the memfd, even though it may re-read itself through /proc/self/exe.
12. A CLAUDE.md in {claude_home} or {claude_home}/claude-cwd isn't loaded, or Arriq accepts that it is. Either way, only dasdevbot and root can write there.
13. Login under the dedicated config dir works with the full isolation argv.
    - `sudo -u dasdevbot -H env CLAUDE_CONFIG_DIR=/var/lib/dasdevbot/claude-config claude` login succeeds, and the credentials land under /var/lib/dasdevbot/claude-config (mode 0700, owner dasdevbot), not /var/lib/dasdevbot/.claude.
    - With that login, a call with `--restricted` and `--safe-mode` (plus the rest of the argv) returns a non-error `result`. `--restricted` ignores settings files; confirm it does not also hide the stored login, so the call is not an `is_error` "Not logged in".
    - `smoke-model --provider claude-cli --claude-home /var/lib/dasdevbot --claude-sha256 <digest>` returns ok under the hardened unit.
14. Record the `permissionMode`, `tools` and `mcp_servers` values from the init line.
    - Capture the `{"type":"system","subtype":"init",...}` line of one stream-json run with the full daemon argv (locally, not from the daemon log, which must not hold the raw stream).
    - Confirm `permissionMode` is `"dontAsk"`, `tools` is `[]` and `mcp_servers` is `[]`, and that the call returns a non-error `result`.
    - The daemon passes `--permission-mode dontAsk` and `permissions.disableAutoMode`, and kills the CLI and refuses the call unless init reports `dontAsk` with empty `tools` and `mcp_servers`.
15. Inherited env doesn't pass through: start the daemon with `CLAUDE_CONFIG_DIR=/tmp/x`, `ANTHROPIC_API_KEY=sk-test`, `ANTHROPIC_BASE_URL=http://127.0.0.1:9`, and `CLAUDE_CODE_USE_BEDROCK=1` in its environment. The child still uses /var/lib/dasdevbot/claude-config and the Claude Pro login (check `/proc/<child>/environ` holds only USER, LANG, TMPDIR, HOME, CLAUDE_CONFIG_DIR, PATH, DISABLE_AUTOUPDATER, DISABLE_UPDATES).
16. Refusals at startup: a symlinked or 0755 /var/lib/dasdevbot/claude-config, or one owned by another uid, makes the provider fail closed with no CLI call.
17. Live-run gate: the shipped unit keeps the all-zero `--claude-sha256` placeholder and no Ollama key is in the keyring until items 1-16 pass, so `serve` fails closed for both providers until then.

## Device user unit

`deploy/ubuntu/dasdevbotd-device.service` is a user unit, not a template.

- There is no `User=` line. `User=%i` is only valid in a template unit (`dasdevbotd-device@.service`). This file is not one.
- `WantedBy=default.target`. A user unit is pulled in by the user manager's `default.target`, not `multi-user.target`.
- `ProtectHome` is omitted. On the user manager it hides the home directory, and `ReadWritePaths=%h` does not pierce that, so a database under `%h` cannot be opened. `StateDirectory=dasdevbot` creates the directory under the user state dir, and the unit passes `--data %S/dasdevbot/device.sqlite`.
- Install with `systemctl --user enable --now dasdevbotd-device.service`.
