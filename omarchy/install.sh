#!/usr/bin/env bash
# Make saturn-art Omarchy's screensaver.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(dirname "$here")

shim_dir="$HOME/.local/share/saturn-art/bin"
shim="$shim_dir/omarchy-launch-screensaver"
rc="$HOME/.bash_profile"
old_rc="$HOME/.bashrc"
begin='# >>> saturn-art screensaver >>>'
end='# <<< saturn-art screensaver <<<'

if ! command -v saturn-art >/dev/null 2>&1; then
  echo "saturn-art is not on PATH; run 'make install' first." >&2
  exit 1
fi

mkdir -p "$shim_dir"
install -m 755 "$here/omarchy-launch-screensaver" "$shim"
# Remove the old inner-command shim from earlier versions; it never took
# effect because Omarchy pins /usr/share/omarchy/bin ahead of the user PATH.
rm -f "$shim_dir/omarchy-screensaver"

# Omarchy's launcher resolves `omarchy-screensaver` with a login shell, and
# /usr/share/omarchy/bin comes before ~/.local/bin.  Put our shim directory
# first for login shells so the shim wins.  It contains only this one file.
# Drop an older block from .bashrc if present (it lands before the mise env,
# which rewrites PATH, so it would not take effect).
strip_block() {
  local f=$1
  [[ -f $f ]] || return 0
  grep -qF "$begin" "$f" || return 0
  local tmp
  tmp=$(mktemp)
  awk -v b="$begin" -v e="$end" '$0 == b { skip = 1; next } $0 == e { skip = 0; next } !skip' "$f" >"$tmp"
  cat "$tmp" >"$f"
  rm -f "$tmp"
}
strip_block "$old_rc"

touch "$rc"
if ! grep -qF "$begin" "$rc"; then
  {
    echo ""
    echo "$begin"
    echo "export PATH=\"\$HOME/.local/share/saturn-art/bin:\$PATH\""
    echo "$end"
  } >>"$rc"
fi

if ! bash -lc 'command -v omarchy-launch-screensaver' | grep -qx "$shim"; then
  echo "warning: a login shell does not resolve $shim first" >&2
  echo "         check the PATH block in $rc" >&2
fi

echo "saturn-art is now the Omarchy screensaver."
echo "  try it:     bash -lc 'omarchy-launch-screensaver force'"
echo "  turn off:   $repo/omarchy/uninstall.sh  (or: make screensaver-off)"
echo "  disable the idle screensaver: omarchy toggle screensaver"
echo "  idle delay: \"idle\": { \"screensaver\": <seconds> } in ~/.config/omarchy/shell.json"
