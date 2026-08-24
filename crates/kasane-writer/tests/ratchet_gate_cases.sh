#!/usr/bin/env bash
# The census gates' negative directions, run against real history.
#
# `mise run census-ratchet` on its own only ever exercises its gates where they
# pass, which is indistinguishable from gates that always pass. This script
# drives two of them into failure on purpose and fails if either stays quiet:
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
#
# Still only ever seen passing, here and in CI: the length-3 union gate and
# both ceilings' no-gratuitous-raise check. Naming that is the point -- the
# gap is smaller than it was, not closed.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

q=crates/kasane-writer/tests/census-known-structure-corrupt.txt
q4=crates/kasane-writer/tests/census-len4-known-structure-corrupt.txt

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
# below mean anything. `census-ratchet` fails as a whole for any of eight
# reasons, so "it exited non-zero" cannot tell the gate under test from an
# unrelated one that spoke first -- and direction 3, which mutates a length-4
# file, cannot be checked that way at all.
row_says() { # label-regex verdict-regex out-file
  grep -Eq "^$1[[:space:]].*$2" "$3"
}

# A shape outside census_support::alphabet() -- Text("z") is not in it -- so it
# can never collide with a blessed entry and read as no growth at all. The
# absence check below is belt and braces on that.
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

echo
echo "every direction behaved correctly"
