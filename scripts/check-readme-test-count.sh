#!/usr/bin/env bash
# Fail when the README's test count no longer matches the tree.
#
# The README's capability table states the test count by hand
# ("N unit and integration tests (M ignored)"), and it has drifted before
# (it was corrected in #15 and drifted again within days). This script compares
# that claim against the tree: every `#[test]` attribute is one test — the count
# matches `cargo test`'s exactly on this tree — and every `#[ignore]` attribute
# marks an ignored one. Run in CI; fix the Tests row in README.md when it fails.
set -euo pipefail
cd "$(dirname "$0")/.."

tree_total=$(grep -rEho '^[[:space:]]*#\[test\]' runtime/hikmah-kernel/src runtime/hikmah-kernel/tests | wc -l)
tree_ignored=$(grep -rho '#\[ignore' runtime/hikmah-kernel/src runtime/hikmah-kernel/tests | wc -l)

readme_claim=$(grep -oE '[0-9]+ unit and integration tests \([0-9]+ ignored' README.md | head -n 1)
if [ -z "$readme_claim" ]; then
  echo "README no longer states a test count the way this script parses;"
  echo "either update scripts/check-readme-test-count.sh or drop the number."
  exit 1
fi
readme_total=$(echo "$readme_claim" | grep -oE '^[0-9]+')
readme_ignored=$(echo "$readme_claim" | grep -oE '\([0-9]+' | grep -oE '[0-9]+')

if [ "$tree_total" != "$readme_total" ] || [ "$tree_ignored" != "$readme_ignored" ]; then
  echo "README test count drifted:"
  echo "  tree:   $tree_total tests ($tree_ignored ignored)"
  echo "  README: $readme_total tests ($readme_ignored ignored)"
  echo "Update the Tests row in README.md (or drop the number)."
  exit 1
fi
echo "README test count OK: $tree_total tests ($tree_ignored ignored)"
