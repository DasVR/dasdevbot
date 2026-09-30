# Ubuntu deploy

The daemon runs as a dedicated `dasdevbot` user. That user's home holds the Claude login and nothing copied from an operator account. The home is `/var/lib/dasdevbot`, so the unit can set `ProtectHome=true` and still pass `--claude-home /var/lib/dasdevbot`.

## Service user

```sh
sudo useradd --system --create-home --home-dir /var/lib/dasdevbot --shell /usr/sbin/nologin dasdevbot
sudo install -d -m 0700 -o dasdevbot -g dasdevbot /var/lib/dasdevbot
```

The shell is `nologin`. `sudo -u dasdevbot -H` runs a command as that user and sets `HOME` to the passwd home, so the login does not need an interactive shell.

Install the Claude Code CLI on a root-owned path, for example `/usr/local/bin/claude`. A user-owned npm `cli.js` is refused unless its interpreter can be hashed as well, and the daemon warns when the resolved binary or a parent directory is not root-owned, or when a parent directory is group or world writable.

`--claude-home` must be mode `0700` and owned by the daemon uid. A normal user's home and `$HOME` are refused. A system account (uid below 1000) may use its own passwd home, which is this directory.

## One-time Claude login

Log in once as the service user. This writes the login under `/var/lib/dasdevbot`, which is the only home the CLI is given.

```sh
sudo -u dasdevbot -H claude
```

Do not copy `~/.claude` from a personal account into that home.

## Daemon

`dasdevbotd.service` runs as `dasdevbot` and passes `--claude-home /var/lib/dasdevbot`. The process sets the CLI's `HOME` to that path. It does not pass the operator's `HOME`. Each call uses a mode-0700 directory under `/var/lib/dasdevbot/claude-cwd` and writes the system prompt outside that directory.

`--setting-sources` stays `project,local`. The Claude Code CLI documents that flag as a comma-separated list of `user`, `project`, and `local` only. There is no empty or `none` value, and an empty string was reported broken from CLI 2.1.59 onward. The pin is 2.1.285, so `user` is omitted and `project,local` remains. The private cwd keeps project and local files from being read out of `/tmp`.

Record the binary's SHA-256 and pass it with `--claude-sha256` once the live install is known. A later change to those bytes fails closed. The digest recorded at startup is never replaced.

```sh
sudo install -m 644 deploy/ubuntu/dasdevbotd.service /etc/systemd/system/dasdevbotd.service
sudo systemctl daemon-reload
sudo systemctl enable --now dasdevbotd
```

The unit sets `ProtectSystem=strict`, `ProtectHome=true`, `ReadWritePaths=/var/lib/dasdevbot`, `PrivateTmp=true`, and `UMask=0077`.
