#!/usr/bin/env bash
# tokens.css is the locked look v1.1 file. It must stay byte-identical.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
file="$root/apps/desktop/src/lib/styles/tokens.css"
expected="88c892553ba05e341c63441ab6551a02a925c88a2a70a0b540005dbf7a4b7f8f"
actual="$(sha256sum "$file" | awk '{print $1}')"
if [[ "$actual" != "$expected" ]]; then
  echo "tokens.css sha256 mismatch" >&2
  echo "expected $expected" >&2
  echo "actual   $actual" >&2
  exit 1
fi
echo "tokens.css sha256 $actual"
