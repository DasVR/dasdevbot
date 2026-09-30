#!/usr/bin/env bash
# tokens.css is the locked look v1.1 file. It must stay byte-identical.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
file="$root/apps/desktop/src/lib/styles/tokens.css"
expected="232e2b6e68c9c4fb3a9d46ee3353a3d412ed2b9dad327a1a4f8b7078b34ea8cd"
actual="$(sha256sum "$file" | awk '{print $1}')"
if [[ "$actual" != "$expected" ]]; then
  echo "tokens.css sha256 mismatch" >&2
  echo "expected $expected" >&2
  echo "actual   $actual" >&2
  exit 1
fi
echo "tokens.css sha256 $actual"
