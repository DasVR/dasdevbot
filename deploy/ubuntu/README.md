# Ubuntu deploy

The daemon runs as a dedicated `dasdevbot` user. That user's home holds the Claude login and nothing copied from an operator account.

## Service user

```sh
sudo useradd --system --create-home --home-dir /var/lib/dasdevbot --shell /bin/bash dasdevbot
sudo install -d -o dasdevbot -g dasdevbot /var/lib/dasdevbot
```

Install the Claude Code CLI on a root-owned path, for example `/usr/local/bin/claude`. A user-owned npm `cli.js` is refused unless its interpreter can be hashed as well, and the daemon warns when the resolved binary is not root-owned.

## One-time Claude login

Log in once as the service user. This writes the login under `/var/lib/dasdevbot`, which is the only home the CLI is given.

```sh
sudo -iu dasdevbot claude
```

Do not copy `~/.claude` from a personal account into that home.

## Daemon

`dasdevbotd.service` runs as `dasdevbot` and passes `--claude-home /var/lib/dasdevbot`. The process sets the CLI's `HOME` to that path. It does not pass the operator's `HOME`.

```sh
sudo install -m 644 deploy/ubuntu/dasdevbotd.service /etc/systemd/system/dasdevbotd.service
sudo systemctl daemon-reload
sudo systemctl enable --now dasdevbotd
```
