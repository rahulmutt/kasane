#!/usr/bin/env bash
# Reproduces the length-4/length-5 structural regression figures in
# ../README.md's "Post-implementation verification" section: how many
# INEXPR->CORRUPT regressions commit 05bb516 (delimiter choice before splice,
# no condition 4) introduced against `main`, and that commit 0909b3a
# (condition 4: decline `_` where the saved child would fuse across classes)
# closes every one of them.
#
# Evidence only -- not a test, never run in CI. Uses `git worktree` against
# three fixed revisions and leaves the working tree untouched; each worktree
# is removed before this script exits (even on failure, via the trap).
#
# Usage: structural-len4-5-sweep.sh [output-dir]
#   output-dir defaults to a fresh mktemp -d. Each revision's per-shape
#   classification lands in <output-dir>/<label>/len{4,5}.tsv.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(git -C "$HERE" rev-parse --show-toplevel)"
OUT="${1:-$(mktemp -d)}"
mkdir -p "$OUT"
echo "output directory: $OUT"

BASE=d4fc510   # main, pre-item
NOFIX=05bb516  # delimiter choice before splice, condition 4 not yet landed
FIX=0909b3a    # + condition 4

worktrees=()
cleanup() {
  for wt in "${worktrees[@]:-}"; do
    [ -n "$wt" ] && git -C "$REPO" worktree remove --force "$wt" 2>/dev/null || true
  done
}
trap cleanup EXIT

run_revision() {
  local label="$1" sha="$2"
  local wt="$OUT/wt-$label"
  echo
  echo "== $label ($sha) =="
  git -C "$REPO" worktree add --quiet --detach "$wt" "$sha"
  worktrees+=("$wt")
  cp "$HERE/zz_structural_len4_5.rs" "$wt/crates/kasane-writer/tests/zz_structural_len4_5.rs"
  mkdir -p "$OUT/$label"
  ( cd "$wt" && KASANE_STRUCT_OUT="$OUT/$label" \
      cargo test --release -p kasane-writer --test zz_structural_len4_5 -- --nocapture )
  git -C "$REPO" worktree remove --force "$wt"
  worktrees=("${worktrees[@]/$wt/}")
}

run_revision base   "$BASE"
run_revision nofix  "$NOFIX"
run_revision fix    "$FIX"

echo
echo "== per-shape transitions against main (line-for-line: same enumeration order every revision) =="
for len in 4 5; do
  echo "-- length-$len --"
  for target in nofix fix; do
    regressions="$(paste "$OUT/base/len$len.tsv" "$OUT/$target/len$len.tsv" \
      | awk -F'\t' '$1=="INEXPR" && $3=="CORRUPT" {n++} END{print n+0}')"
    improvements="$(paste "$OUT/base/len$len.tsv" "$OUT/$target/len$len.tsv" \
      | awk -F'\t' '$1!="CLEAN" && $3=="CLEAN" {n++} END{print n+0}')"
    echo "  base->$target   INEXPR->CORRUPT regressions: $regressions   ->CLEAN improvements: $improvements"
  done
  fix_vs_nofix="$(paste "$OUT/nofix/len$len.tsv" "$OUT/fix/len$len.tsv" \
    | awk -F'\t' '$1!=$3 {n++} END{print n+0}')"
  echo "  nofix->fix      shapes that move (any direction): $fix_vs_nofix"
done
