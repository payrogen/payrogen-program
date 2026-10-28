#!/usr/bin/env bash
#
# sync.sh — refresh this public audit repo from the private PAYROGEN monorepo.
#
# Copies ONLY the Anchor program + workspace files from the monorepo into this repo,
# runs a secret scan as a safety gate, and shows a git diff. It does NOT commit or push —
# review the diff, then commit/push yourself. This keeps the publishable surface an
# explicit allowlist (fail-closed): nothing reaches GitHub unless copied here first.
#
# Usage:  ./sync.sh
set -euo pipefail

# --- Paths (edit SRC if your monorepo moves) ---
SRC="/Users/mac/Documents/INSTAFOODY/APP/PAYROGEN"
AUDIT="$(cd "$(dirname "$0")" && pwd)"

echo "Source monorepo : $SRC"
echo "Audit repo      : $AUDIT"

if [ ! -d "$SRC/programs/payrogen" ]; then
  echo "ERROR: $SRC/programs/payrogen not found. Fix SRC in sync.sh." >&2
  exit 1
fi

# --- Copy ONLY the program + workspace files (allowlist) ---
rm -rf "$AUDIT/programs"
cp -R "$SRC/programs" "$AUDIT/programs"
cp "$SRC/Anchor.toml" "$SRC/Cargo.toml" "$SRC/Cargo.lock" "$AUDIT/"

# --- Strip noise ---
find "$AUDIT" -name '.DS_Store' -delete
rm -rf "$AUDIT/programs/payrogen/target" "$AUDIT/target" "$AUDIT/.anchor" 2>/dev/null || true

# --- Safety gate: scan for anything sensitive before you commit ---
echo "Running secret scan..."
HITS="$(grep -rniE \
  'BEGIN .*PRIVATE|api[_-]?key|mnemonic|secret_key|SECRET=|PASSWORD=|HELIUS|CIRCLE_API|AES256|HMAC_SIGNING' \
  "$AUDIT/programs" "$AUDIT/Anchor.toml" 2>/dev/null | grep -v 'Cargo.lock' || true)"
if [ -n "$HITS" ]; then
  echo "!!! POTENTIAL SECRET DETECTED — aborting. Review before committing:" >&2
  echo "$HITS" >&2
  exit 2
fi

# --- Also fail if any keypair-looking json or .env slipped in ---
if find "$AUDIT" -path "$AUDIT/.git" -prune -o \( -name '*.json' ! -name 'Cargo.lock' -o -name '.env*' \) -print | grep -q .; then
  echo "!!! Unexpected .json/.env file present — review before committing:" >&2
  find "$AUDIT" -path "$AUDIT/.git" -prune -o \( -name '*.json' ! -name 'Cargo.lock' -o -name '.env*' \) -print >&2
  exit 3
fi

echo "Secret scan clean."
echo
echo "Changes staged for review (git status):"
cd "$AUDIT"
git add -A
git status --short

echo
echo "Review the diff above, then publish with:"
echo "  cd \"$AUDIT\" && git commit -m \"sync program from monorepo\" && git push"
