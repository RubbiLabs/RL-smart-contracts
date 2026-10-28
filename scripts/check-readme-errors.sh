#!/usr/bin/env bash
#
# Keep the error-code tables in README.md in sync with the contracts.
#
# For each contract, the table between the markers
#   <!-- errors:<contract>:start -->  and  <!-- errors:<contract>:end -->
# in README.md must be exactly the table generated from the `Error` enum in
# contracts/<contract>/src/errors.rs (code, variant name, rustdoc meaning).
#
# Usage:
#   scripts/check-readme-errors.sh          # exit 1 and show a diff on drift
#   scripts/check-readme-errors.sh --write  # rewrite the README tables
#
# Portable to bash 3.2 (macOS default); needs awk and diff.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
README="$REPO_ROOT/README.md"
CONTRACTS="vault subscriptions payroll"
WRITE=0

case "${1:-}" in
  "") ;;
  --write) WRITE=1 ;;
  *) printf 'usage: %s [--write]\n' "$0" >&2; exit 2 ;;
esac

# Markdown table generated from errors.rs: `| code | `Name` | meaning |`.
# Doc-comment lines directly above a variant are joined into the meaning.
generate_table() {
  awk '
    BEGIN { in_enum = 0; doc = ""; print "| Code | Name | Meaning |"; print "|---|---|---|" }
    /^pub enum Error \{/ { in_enum = 1; next }
    in_enum && /^\}/ { exit }
    !in_enum { next }
    /^[[:space:]]*\/\/\// {
      line = $0
      sub(/^[[:space:]]*\/\/\/[[:space:]]?/, "", line)
      doc = (doc == "" ? line : doc " " line)
      next
    }
    /^[[:space:]]*[A-Za-z_]+[[:space:]]*=[[:space:]]*[0-9]+,?/ {
      name = $0; sub(/^[[:space:]]*/, "", name); sub(/[[:space:]]*=.*$/, "", name)
      code = $0; sub(/^[^=]*=[[:space:]]*/, "", code); sub(/[^0-9].*$/, "", code)
      printf "| %s | `%s` | %s |\n", code, name, doc
      doc = ""
    }
  ' "$1"
}

# The lines strictly between the two markers of a contract.
readme_table() {
  awk -v s="<!-- errors:$1:start -->" -v e="<!-- errors:$1:end -->" '
    $0 == e { on = 0 }
    on { print }
    $0 == s { on = 1 }
  ' "$README"
}

# README with the block of one contract replaced by the content of file $2.
replace_block() {
  awk -v s="<!-- errors:$1:start -->" -v e="<!-- errors:$1:end -->" -v f="$2" '
    $0 == s { print; while ((getline line < f) > 0) print line; close(f); skipping = 1; next }
    $0 == e { skipping = 0 }
    !skipping { print }
  ' "$README"
}

status=0
tmp_expected="$(mktemp)"
tmp_actual="$(mktemp)"
tmp_readme="$(mktemp)"
trap 'rm -f "$tmp_expected" "$tmp_actual" "$tmp_readme"' EXIT

for contract in $CONTRACTS; do
  errors_rs="$REPO_ROOT/contracts/$contract/src/errors.rs"
  [ -f "$errors_rs" ] || { printf 'missing %s\n' "$errors_rs" >&2; exit 1; }
  generate_table "$errors_rs" > "$tmp_expected"

  if ! grep -q "<!-- errors:$contract:start -->" "$README" \
     || ! grep -q "<!-- errors:$contract:end -->" "$README"; then
    printf 'README.md has no <!-- errors:%s:start/end --> markers\n' "$contract" >&2
    exit 1
  fi

  if [ "$WRITE" -eq 1 ]; then
    replace_block "$contract" "$tmp_expected" > "$tmp_readme"
    cat "$tmp_readme" > "$README"
    printf 'updated README error table: %s\n' "$contract"
    continue
  fi

  readme_table "$contract" > "$tmp_actual"
  if diff -u --label "README.md ($contract)" --label "contracts/$contract/src/errors.rs" \
       "$tmp_actual" "$tmp_expected"; then
    printf 'ok: %s error table matches errors.rs\n' "$contract"
  else
    printf 'DRIFT: README.md %s error table differs from errors.rs (run %s --write)\n' \
      "$contract" "scripts/check-readme-errors.sh" >&2
    status=1
  fi
done

exit "$status"
