# PR #25: live Ubuntu check (real claude 2.1.285, run as the dasdevbot user)

Security Director sign-off needs every item below to pass.

1. Version and hash.
   - `claude --version` prints 2.1.285.
   - Its SHA-256 matches the recorded value for /usr/local/bin/claude, or for cli.js plus /usr/bin/node on an npm install.
   - The resolved paths are root-owned, and their parent dirs aren't writable by dasdevbot.
2. 2.1.285 accepts every flag in the argv: `--tools ""`, `--disallowedTools "*"`, `--strict-mcp-config --mcp-config '{"mcpServers":{}}'`, `--max-turns 1`, `--setting-sources project,local`, `--settings '{"disableAllHooks":true}'`, `--system-prompt-file`, `--disable-slash-commands`, `--no-session-persistence`.
3. Hooks and project files don't fire.
   - The ignored test `local_claude_does_not_run_a_hostile_home_hook` passes.
   - A hook in /var/lib/dasdevbot/.claude/settings.json doesn't fire.
   - A CLAUDE.md or .claude/settings.json placed in /tmp, or in any ancestor of the cwd, isn't loaded.
4. The ~/.claude/CLAUDE.md under claude_home isn't loaded, or Arriq explicitly accepts that it is.
5. With a prompt that asks for a tool, the stream shows no tool_use. If one appears, provider.tool_use_blocked{job_id, cli_version} is written and no process in the group survives (`ps -g`).
6. A captured real rate_limit_event parses: the field names and status values match. Record whether utilization can exceed 1.0.
7. The service starts under the hardened unit, with ProtectHome and PrivateTmp. The login works, and /var/lib/dasdevbot is mode 0700 and owned by dasdevbot.
8. There's no auto-update: DISABLE_AUTOUPDATER and DISABLE_UPDATES are honored, and the binary hash is unchanged after 24 h.
9. `smoke-model --provider claude-cli --claude-home /var/lib/dasdevbot` returns ok, and the prompt text never appears in `journalctl -u dasdevbotd`.
