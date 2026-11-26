#!/bin/bash
# Usage: with_gate_queue.sh "<description>" <command...>
#        with_gate_queue.sh --status
# FIFO version of with_gate_lock.sh. Each waiter drops a ticket in $QDIR; only the oldest
# live ticket may try to take the lock, so a queued run cannot lose the race repeatedly.
# The lock itself is unchanged: GATE_RUNNING created with noclobber while BENCH_RUNNING is
# absent, so waiters on the old with_gate_lock.sh and this script still exclude each other.
# (Old waiters are not queued and may still jump ahead; the queue head polls every
# $POLL s, faster than their 60 s, to make that rare.)
# Tickets whose waiter PID has died are removed. A GATE_RUNNING whose holder looks dead is
# only reported, never removed: other tools write that file too.
# Default lock folder: the Windows gate machine's, or on Linux/macOS (where D:/... would be
# a relative folder named "D:") a private per-account folder in /tmp, refused unless this
# account owns it. To queue several accounts together, set GATE_LOCK_DIR to a folder they
# can all write (and in every older lock helper too).
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) WIN=1; D_DEFAULT=D:/CLB-Dev/ras-perf ;;
  *) WIN=0; D_DEFAULT=/tmp/ras-perf-$(id -u) ;;
esac
D=${GATE_LOCK_DIR:-$D_DEFAULT}
LOCK=$D/GATE_RUNNING; BENCH=$D/BENCH_RUNNING; QDIR=$D/GATE_QUEUE
POLL=${GATE_QUEUE_POLL:-5}; REPORT=${GATE_QUEUE_REPORT:-60}
if [ $WIN = 0 ] && [ -z "$GATE_LOCK_DIR" ]; then
  mkdir -p -m 700 "$D" && [ ! -L "$D" ] && [ -O "$D" ] || {
    echo "lock folder $D is not a folder owned by $(id -un); set GATE_LOCK_DIR" >&2; exit 2; }
fi
mkdir -p "$QDIR"

winpid() { cat /proc/$1/winpid 2>/dev/null; }
# Alive if the MSYS pid answers, or (waiter from another MSYS runtime) its Windows pid exists.
alive() {  # alive <msys-pid> <win-pid>
  kill -0 "$1" 2>/dev/null && return 0
  # Linux/macOS ticket (no Windows pid): kill -0 fails on another account's live process.
  [ -z "$2" ] && [ $WIN = 0 ] && { ps -p "$1" > /dev/null 2>&1; return; }
  [ -n "$2" ] && tasklist //FI "PID eq $2" //NH 2>/dev/null | grep -qw "$2"
}
# Print the oldest live ticket path; delete stale ones on the way.
head_ticket() {
  local t p w
  for t in $(ls "$QDIR" 2>/dev/null | grep '\.ticket$' | sort); do
    read -r p w < "$QDIR/$t" 2>/dev/null || continue
    if alive "$p" "$w"; then echo "$QDIR/$t"; return; fi
    echo "$(date +%T) removing stale ticket $t ($(sed -n 2p "$QDIR/$t" 2>/dev/null))" >&2
    rm -f "$QDIR/$t"
  done
}

if [ "$1" = --status ]; then
  echo "lock:  $(cat $LOCK 2>/dev/null || echo free)"; [ -e $BENCH ] && echo "bench: $(cat $BENCH)"
  head_ticket > /dev/null
  for t in $(ls "$QDIR" | grep '\.ticket$' | sort); do echo "queue: $(sed -n 2p "$QDIR/$t")"; done
  exit 0
fi

DESC=$1; shift
[ $# -gt 0 ] || { echo "usage: $0 \"<description>\" <command...>" >&2; exit 2; }
# Name sorts by arrival: zero-padded epoch nanoseconds, then pid as a tie-break.
# BSD/macOS date has no %N: fall back to whole seconds (pid then breaks a same-second tie).
NS=$(date +%s%N); case $NS in *[!0-9]*) NS=$(date +%s)000000000 ;; esac
TICKET=$QDIR/$(printf '%020d' "$NS")-$$.ticket
OWN="$DESC (pid $$, started $(date '+%F %T'))"
printf '%s %s\n%s\n' "$$" "$(winpid $$)" "$OWN" > "$TICKET.tmp" && mv "$TICKET.tmp" "$TICKET" || exit 1
HAVE=0
cleanup() { rm -f "$TICKET" "$TICKET.tmp"; [ $HAVE = 1 ] && [ "$(cat $LOCK 2>/dev/null)" = "$OWN" ] && rm -f $LOCK; }
trap cleanup EXIT; trap 'exit 130' INT; trap 'exit 143' TERM; trap 'exit 129' HUP

last=0
while true; do
  h=$(head_ticket)
  if [ "$h" = "$TICKET" ] && [ ! -e $BENCH ] && ( set -o noclobber; echo "$OWN" > $LOCK ) 2>/dev/null; then
    HAVE=1; rm -f "$TICKET"; break
  fi
  now=$(date +%s)
  if [ $((now - last)) -ge $REPORT ]; then
    last=$now; pos=$(ls "$QDIR" | grep '\.ticket$' | sort | grep -n "$(basename $TICKET)" | cut -d: -f1)
    held=$(cat $LOCK $BENCH 2>/dev/null)
    hp=$(sed -n 's/.*(pid \([0-9]*\),.*/\1/p' $LOCK 2>/dev/null)
    [ -n "$hp" ] && ! kill -0 "$hp" 2>/dev/null && held="$held [holder pid $hp not visible here: possibly stale]"
    echo "$(date +%T) queue position $pos; waiting for: ${held:-queue head}"
  fi
  sleep $POLL
done
echo "$(date +%T) got $LOCK"
"$@"
