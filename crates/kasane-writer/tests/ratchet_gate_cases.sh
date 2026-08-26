#!/usr/bin/env bash
# The census gates' negative directions, run against real history.
#
# `mise run census-ratchet` on its own only ever exercises its gates where they
# pass, which is indistinguishable from gates that always pass. This script
# drives every one of them into failure on purpose and fails if any stays quiet:
#
#   direction 2 -- the length-3 queue gate. It admits `queue_added \
#     text_removed`: a shape may enter the structure queue only if the same
#     shape left census-known-corrupt.txt in the same commit. The case is the
#     one design spec §2b.4 of the abutment ledger recorded, where a shape
#     entered the queue with the text file unchanged.
#   direction 3 -- the length-4 union gate. Its two files carry no promotion
#     rule (there is no length-4 text file), so any growth is a regression.
#     Exercised by hand once on 2026-08-23 and filed under
#     docs/superpowers/evidence/2026-08-23-len4-structural-tier/; this is what
#     keeps re-proving it.
#   direction 4 -- the length-3 union gate. Injected into the PERMANENT file
#     rather than the queue, which is the whole of the case: `perm` is
#     report-only, so `union` is the only gate left that can speak. The same
#     shape in the queue would trip `queue+` first and prove nothing here.
#   directions 5 and 6 -- the two ceilings' no-gratuitous-raise check, at
#     length 3 and length 4. A raise with no shape newly claimed permanent is
#     a pre-authorised bless, which is the reviewable moment the ceiling
#     exists to create.
#   direction 7 -- the same check's POSITIVE direction: a raise that IS
#     justified must pass. Alone among these gates, `ceiling_check`'s
#     predicate has two terms (`raised` AND `nothing moved in`), so the
#     failure directions above cannot tell it from a check that rejects every
#     raise. Dropping `&& [ "$grew" -eq 0 ]` from the task leaves 5 and 6
#     green and only this direction red.
#   direction 8 -- the length-5 union gate. Its counts file carries no
#     promotion rule, the same as union4's: any growth on the gated number is
#     a regression. `queue5` and `perm5` are report-only, so `union5` is the
#     only one of the three that can speak.
#
# Every gate the ratchet table prints now has a direction here: queue+, union,
# union4, union5, both ceilings' no-gratuitous-raise check, and the ceiling's
# positive direction. What that does NOT cover: the `text` gate, whose file is
# empty at both ends, so there is no growth to inject that the union would not
# also catch first.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

q=crates/kasane-writer/tests/census-known-structure-corrupt.txt
q4=crates/kasane-writer/tests/census-len4-known-structure-corrupt.txt
perm=crates/kasane-writer/tests/census-inexpressible.txt
ceil=crates/kasane-writer/tests/census-permanent-count.txt
ceil4=crates/kasane-writer/tests/census-len4-permanent-count.txt
counts5=crates/kasane-writer/tests/census-len5-counts.txt

tmp="$(mktemp -d)"

# Restore every mutated census file -- they are tracked -- before cleanup. An
# EXIT trap that only removed $tmp would delete the backups before a signal
# handler ever got a chance to run a restore of its own, so a SIGTERM/Ctrl-C
# during a mutation or during a `mise run` call would leave the working tree
# with a modified allowlist and nothing left to recover it from. Putting the
# restore in the trap, ahead of the cleanup, means the ordering cannot invert
# on any exit path the shell still controls -- normal, early, or signaled.
# `SIGKILL` and a power cut run no trap at all; against those the guarantee is
# only that the backups are still on disk to restore from by hand.
#
# The restore is checked, not silenced. A `cp` that failed behind `2>/dev/null`
# left the census file mutated while the script went on to print "every
# direction behaved correctly" and exit 0 -- a script whose whole subject is a
# gate that must speak has no business swallowing its own. A failure on one
# file does not skip the others, and $tmp survives so every backup is still
# recoverable.
#
# Written once and called per file rather than copied per direction: two
# directions mutate two files, and a second copy of the reasoning above is a
# second copy to drift.
n_backups=0
back_up() { # path
  n_backups=$((n_backups + 1))
  printf '%s\n' "$1" > "$tmp/path.$n_backups"
  cp "$1" "$tmp/backup.$n_backups"
}
restore() {
  local i=1 p status=0
  while [ "$i" -le "$n_backups" ]; do
    p="$(cat "$tmp/path.$i")"
    if ! cp -f "$tmp/backup.$i" "$p"; then
      echo "FAIL: could not restore $p; it is still mutated." >&2
      echo "      Its original content is in $tmp/backup.$i, which is NOT removed." >&2
      status=1
    fi
    i=$((i + 1))
  done
  [ "$status" -eq 0 ] || exit 1
  rm -rf "$tmp"
}
trap restore EXIT

# Run the gate, keep its output, and hand back its exit status. Both streams:
# the table prints to stdout and the failure messages to stderr, and a
# direction needs to read both.
ratchet() { # out-file
  local rc=0
  mise run census-ratchet > "$1" 2>&1 || rc=$?
  cat "$1"
  return "$rc"
}

# Does one row of the gate's table say this?
#
# Matching the row rather than the exit status is what makes the directions
# below mean anything. `census-ratchet` fails as a whole for any of nine
# reasons, so "it exited non-zero" cannot tell the gate under test from an
# unrelated one that spoke first -- and direction 3, which mutates a length-4
# file, cannot be checked that way at all.
row_says() { # label-regex verdict-regex out-file
  grep -Eq "^$1[[:space:]].*$2" "$3"
}

# Does the output carry this line anywhere?
#
# `ceiling_check` speaks in a sentence, not a table row -- it prints
# `ceiling(lenN): ...` on success and `FAIL ceiling(lenN) raised ...` on
# failure, neither of which `row_says` can match. Directions 5-7 need the same
# specificity the table directions get, for the same reason: the ceiling is one
# of nine things that fail this task.
line_says() { # regex out-file
  grep -Eq "$1" "$2"
}

# The base revision `census-ratchet` itself resolved, read back off its own
# output rather than re-derived here. Directions 5-7 have to raise a ceiling
# ABOVE its value at the base, and a second copy of the merge-base logic is a
# second thing to drift -- worse, one that would drift silently, since a
# mis-resolved base still produces a plausible number.
base_rev() { # out-file
  local b
  b="$(awk '/^base: / { print $2; exit }' "$1")"
  if [ -z "$b" ]; then
    echo "FAIL: census-ratchet printed no 'base:' line; cannot resolve the" >&2
    echo "      revision its ceilings compare against." >&2
    exit 1
  fi
  printf '%s\n' "$b"
}

# Shapes outside census_support::alphabet() -- Text("z") is not in it -- so they
# can never collide with a blessed entry and read as no growth at all. The
# absence checks below are belt and braces on that.
probe3='[Text("z"), Text("z"), Text("z")]'
probe4='[Text("z"), Text("z"), Text("z"), Text("z")]'

echo "== direction 1: this branch's census files must PASS =="
ratchet "$tmp/out.1"
# The passing direction pins that union4 is *running*, not merely absent, and
# it is the only place that can: all three runs resolve the same base, so a
# union4 that skips here skips in direction 3 too, and direction 3 would then
# report a gate proven that never ran -- the exact failure mode this script
# exists to close. Diagnosing the skip separately from a missing row is worth
# the two lines: one is a stale branch, the other is a gate that was removed.
if row_says union4 'skipped \(no baseline\)' "$tmp/out.1"; then
  echo "FAIL: union4 skipped -- no length-4 baseline at this base, so direction 3" >&2
  echo "      cannot be exercised. Rebase onto a commit that carries the" >&2
  echo "      length-4 census files." >&2
  exit 1
fi
if ! row_says union4 '[[:space:]]ok$' "$tmp/out.1"; then
  echo "FAIL: no passing 'union4 ... ok' row; the length-4 union gate did not run." >&2
  exit 1
fi
# The same argument for the length-3 union, which direction 4 targets. It
# cannot skip -- `check union` is called with no skip-marker -- so unlike
# union4 there is one failure mode here rather than two: the row is absent
# because the gate was deleted or downgraded to `report`, and a deleted gate
# accepts everything in silence.
if ! row_says union '[[:space:]]ok$' "$tmp/out.1"; then
  echo "FAIL: no passing 'union ... ok' row; the length-3 union gate did not run." >&2
  exit 1
fi
# The same argument for union5, which direction 8 targets. Like union4 it CAN
# skip -- its counts file is absent at any base predating it -- so it has the
# same two failure modes: a stale branch, or a gate that was removed.
if row_says union5 'skipped \(no baseline\)' "$tmp/out.1"; then
  echo "FAIL: union5 skipped -- no length-5 counts at this base, so direction 8" >&2
  echo "      cannot be exercised. Rebase onto a commit that carries" >&2
  echo "      census-len5-counts.txt." >&2
  exit 1
fi
if ! row_says union5 '[[:space:]]ok$' "$tmp/out.1"; then
  echo "FAIL: no passing 'union5 ... ok' row; the length-5 union gate did not run." >&2
  exit 1
fi
# And for both ceilings, whose check returns 0 early when the ceiling file is
# absent at the base -- printing `absent at the base` and gating nothing.
# Directions 5-7 mutate a file that check would not read, so without this they
# would report three gates proven that never ran. This is the ceiling's form of
# the union4 skip above.
for len in len3 len4; do
  if line_says "^ceiling\($len\): absent at the base" "$tmp/out.1"; then
    echo "FAIL: ceiling($len) is absent at this base, so its directions cannot be" >&2
    echo "      exercised. Rebase onto a commit that carries the ceiling files." >&2
    exit 1
  fi
  if ! line_says "^ceiling\($len\): [0-9]+ -> [0-9]+ \([0-9]+ shape" "$tmp/out.1"; then
    echo "FAIL: no 'ceiling($len): N -> N' line; that ceiling check did not run." >&2
    exit 1
  fi
done
# Read once, here, from the run that has just been checked for coherence. Every
# direction invokes the same task in the same tree, so they all resolve the
# same base -- taking it from this output rather than re-deriving it is what
# guarantees the ceilings directions 5-7 raise are measured against the
# revision the gate itself will compare them to.
base="$(base_rev "$tmp/out.1")"
echo "base for the ceiling directions: $base"

echo
echo "== direction 2: a length-3 queue growth with no text removal must FAIL =="
back_up "$q"
printf '%s\n' '[Code("x"), Code("x"), Emph([Emph([Text("a")])])]' >> "$q"
LC_ALL=C sort -u -o "$q" "$q"
if ratchet "$tmp/out.2"; then
  echo "FAIL: the gate accepted a queue growth with no matching text removal" >&2
  exit 1
fi
if ! row_says 'queue\+' 'FAIL' "$tmp/out.2"; then
  echo "FAIL: census-ratchet failed, but not on the queue+ gate this case targets." >&2
  exit 1
fi
cp -f "$tmp/backup.$n_backups" "$q"

echo
echo "== direction 3: a length-4 union growth must FAIL =="
if grep -Fxq "$probe4" "$q4"; then
  echo "FAIL: the probe shape is already in $q4, so injecting it proves nothing." >&2
  exit 1
fi
back_up "$q4"
printf '%s\n' "$probe4" >> "$q4"
LC_ALL=C sort -u -o "$q4" "$q4"
if ratchet "$tmp/out.3"; then
  echo "FAIL: the gate accepted a length-4 union growth" >&2
  exit 1
fi
# Direction 1 has already established that union4 is live at this base, so the
# only way this row can be anything but a verdict on the injection is a gate
# that changed under us mid-run. `FAIL -- 1 added` is therefore the whole
# assertion: a skipped or missing row cannot match it either.
if ! row_says union4 'FAIL -- 1 added' "$tmp/out.3"; then
  echo "FAIL: census-ratchet failed, but not on the union4 gate this case targets." >&2
  exit 1
fi
cp -f "$tmp/backup.$n_backups" "$q4"

echo
echo "== direction 4: a length-3 union growth must FAIL =="
# Into the permanent file, not the queue. `check perm` is `report`, so it
# cannot speak; `queue+` and `check queue` never read this file; the text gate
# never reads it either. `union` is the only gate left, which is what makes
# `union ... FAIL -- 1 added` a verdict on the length-3 union specifically
# rather than on whichever of the nine rows happened to fail first. The same
# shape appended to the queue would trip `queue+` as well and prove nothing.
#
# Both files are checked for the probe, not just the one being written: a shape
# already anywhere in the union is not an addition to it.
if grep -Fxq "$probe3" "$perm" || grep -Fxq "$probe3" "$q"; then
  echo "FAIL: the probe shape is already in the length-3 union, so injecting it" >&2
  echo "      proves nothing." >&2
  exit 1
fi
back_up "$perm"
# Appended, NOT sorted in place the way directions 2 and 3 sort the queue
# files. Those carry no comment header; this one carries eighty lines of it,
# and a whole-file sort would interleave the header into the shapes. It makes
# no difference to the gate either way -- `at_head` drops `#` lines and sorts
# what is left -- so the tidier mutation is the one that leaves the file
# readable if a run is killed between here and the restore.
printf '%s\n' "$probe3" >> "$perm"
if ratchet "$tmp/out.4"; then
  echo "FAIL: the gate accepted a length-3 union growth" >&2
  exit 1
fi
if ! row_says union 'FAIL -- 1 added' "$tmp/out.4"; then
  echo "FAIL: census-ratchet failed, but not on the union gate this case targets." >&2
  exit 1
fi
cp -f "$tmp/backup.$n_backups" "$perm"

# Directions 5 and 6 differ only in which ceiling they raise, so they are one
# case called twice -- the same reasoning `ceiling_check` itself gives for
# being written once and called per length.
#
# The raise is computed from the ceiling's value AT THE BASE, not at HEAD. On
# this branch they are equal, but on a branch that had already lowered a
# ceiling, HEAD+1 could still be at or below the base and `ceiling_check` would
# correctly stay silent -- a direction that then reported "the gate accepted a
# gratuitous raise" would be accusing the gate of the tester's arithmetic.
ceiling_raise_case() { # n label ceiling-path
  local n="$1" label="$2" cpath="$3" cb
  echo
  echo "== direction $n: a ceiling($label) raise with nothing newly permanent must FAIL =="
  cb="$(git show "$base:$cpath" | tr -d '[:space:]')"
  back_up "$cpath"
  printf '%s\n' "$((cb + 1))" > "$cpath"
  if ratchet "$tmp/out.$n"; then
    echo "FAIL: the gate accepted a ceiling($label) raise with no shape newly" >&2
    echo "      claimed inexpressible." >&2
    exit 1
  fi
  # The exact numbers, not just the label: a check that reported some other
  # ceiling movement would otherwise match.
  if ! line_says "^FAIL ceiling\($label\) raised $cb -> $((cb + 1))" "$tmp/out.$n"; then
    echo "FAIL: census-ratchet failed, but not on the ceiling($label) check this" >&2
    echo "      case targets." >&2
    exit 1
  fi
  cp -f "$tmp/backup.$n_backups" "$cpath"
}
ceiling_raise_case 5 len3 "$ceil"
ceiling_raise_case 6 len4 "$ceil4"

echo
echo "== direction 7: a JUSTIFIED ceiling raise must PASS =="
# The positive direction, and the only one here that asserts a green run. It is
# not symmetry for its own sake: `ceiling_check` fails on `raised AND nothing
# moved in`, and directions 5 and 6 hold the second term fixed. Drop
# `&& [ "$grew" -eq 0 ]` from the task and both of them still pass -- the check
# would then reject every raise, including the legitimate ones the ceiling
# exists to make reviewable, and nothing else in this script would notice.
#
# A promotion is modelled the way a real one happens: one shape leaves the
# queue for the permanent file, and the ceiling rises by one in the same
# commit. That keeps the union byte-identical (the shape stays inside it,
# having only changed which member file holds it), leaves `queue_added` empty,
# and gives `ceiling_check` its one newly-permanent shape. The whole task must
# come back green, not merely quiet on the ceiling.
mover="$(awk '!/^#/ && NF { print; exit }' "$q")"
if [ -z "$mover" ]; then
  echo "FAIL: the length-3 queue is empty, so no promotion can be modelled." >&2
  exit 1
fi
if grep -Fxq "$mover" "$perm"; then
  echo "FAIL: $mover is already in the permanent file, so moving it there adds" >&2
  echo "      nothing and \`grew\` would stay 0 -- the direction would pass for" >&2
  echo "      the wrong reason." >&2
  exit 1
fi
cb3="$(git show "$base:$ceil" | tr -d '[:space:]')"
back_up "$q"
back_up "$perm"
back_up "$ceil"
grep -Fxv "$mover" "$q" > "$tmp/q.promoted"
cp -f "$tmp/q.promoted" "$q"
printf '%s\n' "$mover" >> "$perm"
printf '%s\n' "$((cb3 + 1))" > "$ceil"
if ! ratchet "$tmp/out.7"; then
  echo "FAIL: the gate rejected a justified ceiling raise -- one shape promoted" >&2
  echo "      out of the queue and the ceiling raised by one in the same change." >&2
  exit 1
fi
# Exit 0 alone would also be what a deleted ceiling check produces. The line
# proves the check ran, saw the raise, and counted the shape that justified it.
if ! line_says "^ceiling\(len3\): $cb3 -> $((cb3 + 1)) \(1 shape\(s\) newly permanent\)" "$tmp/out.7"; then
  echo "FAIL: the run passed, but ceiling(len3) did not report the raise and the" >&2
  echo "      one shape that justified it." >&2
  exit 1
fi

echo
echo "== direction 8: a length-5 union growth must FAIL =="
back_up "$counts5"
# +1 on the gated number only. queue5 and perm5 are report-only, so leaving
# them alone is what proves union5 spoke rather than a neighbour: this file
# cannot trip any other row.
awk '$1 == "union" { print $1, $2 + 1; next } { print }' "$counts5" > "$tmp/counts5.new"
cat "$tmp/counts5.new" > "$counts5"
if ratchet "$tmp/out.8"; then
  echo "FAIL: the gate accepted a length-5 union growth" >&2
  exit 1
fi
if ! row_says union5 'FAIL' "$tmp/out.8"; then
  echo "FAIL: census-ratchet failed, but not on the union5 row -- so this" >&2
  echo "      direction proves nothing about the gate under test." >&2
  exit 1
fi
echo "  ok: union5 refused the growth"
cp -f "$tmp/backup.$n_backups" "$counts5"

echo
echo "every direction behaved correctly"
