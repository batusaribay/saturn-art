#!/usr/bin/env bash
# Stop using saturn-art as Omarchy's screensaver; restore the stock one.
set -euo pipefail

shim_dir="$HOME/.local/share/saturn-art/bin"
shim="$shim_dir/omarchy-launch-screensaver"
rcs=("$HOME/.bashrc" "$HOME/.bash_profile")
begin='# >>> saturn-art screensaver >>>'
end='# <<< saturn-art screensaver <<<'

rm -f "$shim" "$shim_dir/omarchy-screensaver"
rmdir "$shim_dir" 2>/dev/null || true
rmdir "$(dirname "$shim_dir")" 2>/dev/null || true

for rc in "${rcs[@]}"; do
  [[ -f "$rc" ]] || continue
  grep -qF "$begin" "$rc" || continue
  tmp=$(mktemp)
  awk -v b="$begin" -v e="$end" '
    $0 == b { skip = 1; next }
    $0 == e { skip = 0; next }
    !skip
  ' "$rc" >"$tmp"
  cat "$tmp" >"$rc"
  rm -f "$tmp"
done

echo "The Omarchy screensaver is back to the stock one."
echo "Existing terminals keep the old PATH; the next login shell uses the stock command."
