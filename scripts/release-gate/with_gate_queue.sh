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
D=${GATE_LOCK_DIR:-D:/CLB-Dev/ras-perf}
LOCK=$D/GATE_RUNNING; BENCH=$D/BENCH_RUNNING; QDIR=$D/GATE_QUEUE
POLL=${GATE_QUEUE_POLL:-5}; REPORT=${GATE_QUEUE_REPORT:-60}
mkdir -p "$QDIR"

winpid() { cat /proc/$1/winpid 2>/dev/null; }
# Alive if the MSYS pid answers, or (waiter from another MSYS runtime) its Windows pid exists.
alive() {  # alive <msys-pid> <win-pid>
  kill -0 "$1" 2>/dev/null && return 0
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
TICKET=$QDIR/$(printf '%020d' "$(date +%s%N)")-$$.ticket
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
