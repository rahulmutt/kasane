#!/usr/bin/env bash
# Both directions of the queue gate, run against real history.
#
# The gate admits `queue_added \ text_removed`: a shape may enter the structure
# queue only if the same shape left census-known-corrupt.txt in the same
# commit. This script proves it accepts this branch's promotion and still
# rejects the case design spec §2b.4 of the abutment ledger recorded, where a
# shape entered the queue with the text file unchanged.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

echo "== direction 1: this branch's 32-shape promotion must PASS =="
mise run census-ratchet

echo
echo "== direction 2: a queue growth with no text removal must FAIL =="
tmp="$(mktemp -d)"
q=crates/kasane-writer/tests/census-known-structure-corrupt.txt
cp "$q" "$tmp/q.orig"
# Restore $q -- a tracked census file -- before cleanup on every exit path,
# signals included. An EXIT trap that only removed $tmp would delete the one
# backup before a signal handler ever got a chance to run a restore of its
# own, so a SIGTERM/Ctrl-C during the mutation just below or during the
# `mise run` call would leave the working tree with a modified allowlist and
# nothing left to recover it from. Putting the restore in the trap itself
# means there is no exit path -- normal, early, or signaled -- that skips it.
trap 'cp -f "$tmp/q.orig" "$q" 2>/dev/null; rm -rf "$tmp"' EXIT
printf '%s\n' '[Code("x"), Code("x"), Emph([Emph([Text("a")])])]' >> "$q"
LC_ALL=C sort -u -o "$q" "$q"
if mise run census-ratchet; then
  echo "FAIL: the gate accepted a queue growth with no matching text removal" >&2
  exit 1
fi
echo
echo "both directions behaved correctly"
