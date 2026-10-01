#!/usr/bin/env bash
# tokens.css is the locked look v1.1 file. It must stay byte-identical.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
file="$root/apps/desktop/src/lib/styles/tokens.css"
expected="b78f108c1c09604ba70ca1dc7f00620d12acad0e6826eb1bc5f827e004ef0427"
actual="$(sha256sum "$file" | awk '{print $1}')"
if [[ "$actual" != "$expected" ]]; then
  echo "tokens.css sha256 mismatch" >&2
  echo "expected $expected" >&2
  echo "actual   $actual" >&2
  exit 1
fi
echo "tokens.css sha256 $actual"
