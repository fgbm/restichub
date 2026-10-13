#!/usr/bin/env bash
# Fails on comments that restate the code, refer to review history, or narrate steps.
# The rules are in AGENTS.md, section Code rules.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# The directories to scan. The self-test points this at the fixture directory.
read -r -a dirs <<<"${COMMENT_CHECK_DIRS:-crates}"

if ! command -v rg >/dev/null 2>&1; then
  echo "check-comments: ripgrep (rg) is not installed" >&2
  exit 2
fi

findings=0

# rg exits 1 when it finds nothing.
scan() {
  local pattern="$1"
  shift
  rg -n --no-heading --with-filename -g '*.rs' -g '!**/generated/**' -g '!**/*.pb.rs' \
    -e "$pattern" "$@" || true
}

# Prints each finding and counts it. A trailing `// comment-check: allow` skips the line.
# A non-zero maxwords keeps only a comment of at most that many words.
emit() {
  local maxwords="$1"
  local line rest text words
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    case "$line" in
      *'comment-check: allow'*) continue ;;
    esac
    if [ "$maxwords" -gt 0 ]; then
      rest="${line#*:}"
      text="${rest#*:}"
      words=$(wc -w <<<"$text")
      [ "$words" -le "$maxwords" ] || continue
    fi
    printf '%s\n' "$line"
    findings=$((findings + 1))
  done
}

# A short restatement of the code, such as "The user." or "Creates a tenant.".
restatement='^\s*//[/!]?\s*(The|A|An|Returns?|Creates?|Gets?|Sets?|Checks?)\b[^.]{0,40}\.$'
# A reference to a review finding or an issue.
review_id='^\s*//.*\b[NDMHL][0-9]+\b'
review_text='^\s*//.*(per (the )?review|review finding|issue #[0-9]+)'
# A comment that narrates a step.
narration='^\s*//\s*(Now|Then|First|Next|Finally|Step [0-9])\b'

emit 0 < <(scan "$restatement" "${dirs[@]}")
emit 0 < <(scan "$review_id" "${dirs[@]}")
emit 0 < <(scan "$review_text" "${dirs[@]}")
emit 0 < <(scan "$narration" "${dirs[@]}")

if [ "$findings" -gt 0 ]; then
  echo "check-comments: $findings finding(s)" >&2
  exit 1
fi

echo "check-comments: no findings in ${dirs[*]}"
