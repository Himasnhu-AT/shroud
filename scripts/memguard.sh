#!/usr/bin/env bash
# memguard: run a command (usually cargo) but kill it before the laptop runs out of RAM or disk.
#
#   scripts/memguard.sh cargo build -p game
#
# Env overrides:
#   MEMGUARD_START_PCT  min free-memory % needed to start      (default 20)
#   MEMGUARD_KILL_PCT   kill the job if free memory drops below (default 8)
#   MEMGUARD_MIN_DISK_GB  min free disk GB to start / keep going (default 6)
#   MEMGUARD_WAIT_S     how long to wait for memory before giving up (default 180)
set -u
START_PCT=${MEMGUARD_START_PCT:-20}
KILL_PCT=${MEMGUARD_KILL_PCT:-8}
MIN_DISK_GB=${MEMGUARD_MIN_DISK_GB:-6}
WAIT_S=${MEMGUARD_WAIT_S:-180}

free_pct() { memory_pressure -Q 2>/dev/null | awk -F': ' '/free percentage/ {gsub("%","",$2); print $2+0}'; }
free_disk_gb() { df -g "${CARGO_TARGET_DIR:-$PWD}" 2>/dev/null | awk 'NR==2 {print $4+0}'; }

if [ $# -eq 0 ]; then echo "usage: $0 <command...>" >&2; exit 2; fi

waited=0
while :; do
  m=$(free_pct); d=$(free_disk_gb)
  if [ "${d:-0}" -lt "$MIN_DISK_GB" ]; then
    echo "memguard: only ${d}GB disk free (< ${MIN_DISK_GB}GB). Refusing to start. Try 'cargo clean' or free space." >&2
    exit 75
  fi
  [ "${m:-100}" -ge "$START_PCT" ] && break
  if [ "$waited" -ge "$WAIT_S" ]; then
    echo "memguard: free memory ${m}% stayed below ${START_PCT}% for ${WAIT_S}s. Giving up." >&2
    exit 75
  fi
  [ "$waited" -eq 0 ] && echo "memguard: free memory ${m}% < ${START_PCT}%, waiting (another build running?)..." >&2
  sleep 5; waited=$((waited + 5))
done

# Run in its own process group so we can kill rustc/linker children too.
set -m
"$@" &
pid=$!
pgid=$(ps -o pgid= "$pid" | tr -d ' ')
trap 'kill -TERM -"$pgid" 2>/dev/null; exit 130' INT TERM

killed=""
while kill -0 "$pid" 2>/dev/null; do
  m=$(free_pct); d=$(free_disk_gb)
  if [ "${m:-100}" -lt "$KILL_PCT" ]; then killed="free memory fell to ${m}%"; fi
  if [ "${d:-99}" -lt 2 ]; then killed="disk free fell to ${d}GB"; fi
  if [ -n "$killed" ]; then
    echo "memguard: $killed. Killing job (pgid $pgid) to protect the machine." >&2
    kill -TERM -"$pgid" 2>/dev/null; sleep 3; kill -KILL -"$pgid" 2>/dev/null
    wait "$pid" 2>/dev/null
    exit 137
  fi
  sleep 2
done
wait "$pid"
