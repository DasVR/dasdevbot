#!/usr/bin/env bash
# Fail if NIL token names or ember / dark-glass visuals remain in the client.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/apps/desktop/src"

pattern='--nil-|--brand-ember|brand-ember|--sev-|--wash-ember|wash-ember|--glass-scrim|glass-scrim|dark-glass|dark glass|--lift-|--grain-opacity|Plus Jakarta|color-scheme:[[:space:]]*dark'

mapfile -d '' files < <(find . -type f \( -name '*.svelte' -o -name '*.css' -o -name '*.ts' -o -name '*.html' \) ! -path './lib/styles/tokens.css' -print0)

if ((${#files[@]} == 0)); then
  echo "no client sources" >&2
  exit 1
fi

if grep -n -i -E -e "$pattern" "${files[@]}"; then
  echo "NIL visual leftovers found" >&2
  exit 1
fi

if grep -n -i -E -e '(^|[^A-Za-z])ember([^A-Za-z]|$)' "${files[@]}"; then
  echo "ember leftover found" >&2
  exit 1
fi

echo "ban-nil-visuals: clean"
