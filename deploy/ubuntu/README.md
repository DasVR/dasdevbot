# Ubuntu deploy

The daemon runs as a dedicated `dasdevbot` user. That user's home holds the Claude login and nothing copied from an operator account. The home is `/var/lib/dasdevbot`, so the unit can set `ProtectHome=true` and still pass `--claude-home /var/lib/dasdevbot`.

## Service user

```sh
sudo useradd --system --create-home --home-dir /var/lib/dasdevbot --shell /usr/sbin/nologin dasdevbot
sudo install -d -m 0700 -o dasdevbot -g dasdevbot /var/lib/dasdevbot
```

The shell is `nologin`. `sudo -u dasdevbot -H` runs a command as that user and sets `HOME` to the passwd home, so the login does not need an interactive shell. The daemon allows this passwd home only because the uid is below 1000 and `pw_shell` is `nologin` or `false`. A login shell such as bash is refused.

## Native Claude Code binary

Install the official native build, not npm. The native installer produces one ELF. An npm `cli.js` is a script and the daemon refuses it.

The current install docs (https://code.claude.com/docs/en/setup) accept a version on the native installer. `minimumVersion` is only a floor. Pin the exact build, then copy that ELF to a root-owned path. The installer keeps its launcher at `~/.local/bin/claude`, a symlink into `~/.local/share/claude/versions/`. Do not point the daemon at that user-owned path.

```sh
curl -fsSL https://claude.ai/install.sh | bash -s 2.1.285
sudo install -m 0755 -o root -g root \
  "$(readlink -f "$HOME/.local/bin/claude")" /usr/local/bin/claude
```

`/opt/claude/claude` is the same kind of root-owned path. The daemon warns when the resolved binary or a parent directory is not root-owned, or when a parent directory is group or world writable.

Native installs auto-update unless that is turned off. Official docs: `DISABLE_AUTOUPDATER=1` stops only the background check, and `claude update` / `claude install` still run. `DISABLE_UPDATES=1` blocks those paths as well. The daemon sets both in the CLI's environment. Do not treat `minimumVersion` as a pin.

Compute the digest the service requires and substitute it in `dasdevbotd.service`:

```sh
sha256sum /usr/local/bin/claude
```

`--claude-home` must be mode `0700` and owned by the daemon uid. A normal user's home and `$HOME` are refused.

## One-time Claude login

Log in once as the service user, with the same `CLAUDE_CONFIG_DIR` the daemon gives the CLI. This writes the login under `/var/lib/dasdevbot/claude-config`, which is the only config dir the CLI is given.

```sh
sudo -u dasdevbot -H install -d -m 0700 /var/lib/dasdevbot/claude-config
sudo -u dasdevbot -H env CLAUDE_CONFIG_DIR=/var/lib/dasdevbot/claude-config claude
```

Do not copy `~/.claude` from a personal account into that home.

## Daemon

`dasdevbotd.service` runs as `dasdevbot` and passes `--claude-home /var/lib/dasdevbot`. The process sets the CLI's `HOME` to that path. It does not pass the operator's `HOME`. Each call uses a mode-0700 directory under `/var/lib/dasdevbot/claude-cwd` and writes the system prompt outside that directory.

The child also gets `CLAUDE_CONFIG_DIR=/var/lib/dasdevbot/claude-config`. An inherited `CLAUDE_CONFIG_DIR` is never passed through. The daemon creates that directory mode 0700 if it is missing, and refuses it if it is a symlink, is not owned by the daemon uid, or is not mode 0700.

Every call passes `--restricted`, `--safe-mode`, `--setting-sources project,local` and `--settings '{"disableAllHooks":true}'`, and the daemon refuses to run an argv that lacks any of them. With Claude Code 2.1.285, run offline under `unshare -rn` with a hook and an `apiKeyHelper` command planted in the config dir and in the cwd's `.claude/settings.json`:

- `--setting-sources project,local` alone still ran the project hooks.
- `disableAllHooks` and `--safe-mode` each stopped the hooks but not the project `apiKeyHelper`.
- `--restricted` stopped both. It ignores user, project and local settings files.

Managed settings (`/etc/claude-code/managed-settings.json`) still apply. They are root-owned.

`--setting-sources` stays `project,local`. The Claude Code CLI documents that flag as a comma-separated list of `user`, `project`, and `local` only. There is no empty or `none` value, and an empty string was reported broken from CLI 2.1.59 onward. The pin is 2.1.285, so `user` is omitted and `project,local` remains. The private cwd keeps project and local files from being read out of `/tmp`.

`serve` on the server role requires `--claude-sha256`. A digest that does not match the native ELF fails closed. The digest recorded at startup is never replaced.

```sh
sudo install -m 644 deploy/ubuntu/dasdevbotd.service /etc/systemd/system/dasdevbotd.service
sudo systemctl daemon-reload
sudo systemctl enable --now dasdevbotd
```

The unit sets `ProtectSystem=strict`, `ProtectHome=true`, `ReadWritePaths=/var/lib/dasdevbot`, `PrivateTmp=true`, and `UMask=0077`.
