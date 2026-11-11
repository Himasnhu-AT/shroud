#!/usr/bin/env bash
# Build a release of Pattern of Life with crash, performance and match reports on, for macOS and
# Windows, and (with --upload) push it to itch.io.
#
#   scripts/release.sh                 # check the server and key, build both platforms
#   scripts/release.sh --upload        # ... then scripts/upload-itch.sh
#   scripts/release.sh --mac-only | --windows-only
#   NOTARIZE=0 scripts/release.sh      # local test build (passed to package-macos.sh)
#
# Reports go to the studio's backend (https://api.marlstudio.com, app `shroud`). Its write key is
# SHROUD_WRITE_KEY in the backend's infra/secrets/keys.env (made and registered by the backend's
# infra/deploy-backend.sh); it is never printed. Env: ENDPOINT, KEYS_FILE, VERSION.
#
# Steps, stopping at the first failure:
#   1. GET <endpoint>/v1/health answers
#   2. the write key is accepted (an empty envelope from a throwaway install, then forgotten)
#   3. scripts/package-windows.sh and scripts/package-macos.sh with TELEMETRY=1 (they bake the
#      endpoint and key in, allow only ashlar-trace in the tree, and check the binaries carry it)
#   4. --upload: scripts/upload-itch.sh
# Players can switch reports off in Settings > Gameplay; DO_NOT_TRACK=1 also turns them off.
set -euo pipefail
cd "$(dirname "$0")/.."
ENDPOINT="${ENDPOINT:-https://api.marlstudio.com}"
KEYS_FILE="${KEYS_FILE:-$HOME/codes/synthai/backend/infra/secrets/keys.env}"
UPLOAD=0 MAC=1 WIN=1
for a in "$@"; do
  case "$a" in
    --upload) UPLOAD=1 ;;
    --mac-only) WIN=0 ;;
    --windows-only) MAC=0 ;;
    -h | --help) sed -n '2,21p' "$0"; exit 0 ;;
    *) echo "error: unknown argument $a" >&2; exit 2 ;;
  esac
done
step() { printf '\n==> %s\n' "$*"; }
die() { echo "error: $*" >&2; exit 1; }
[[ "$ENDPOINT" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?$ ]] || die "ENDPOINT must be https://host with no path"

# The key, read without echoing it.
KEY="${SHROUD_BUILD_TRACE_KEY:-}"
if [ -z "$KEY" ]; then
  [ -r "$KEYS_FILE" ] || die "no $KEYS_FILE: run the backend's infra/deploy-backend.sh (it makes SHROUD_WRITE_KEY), or set KEYS_FILE"
  KEY=$(sed -n 's/^SHROUD_WRITE_KEY=//p' "$KEYS_FILE" | tail -1)
fi
[ -n "$KEY" ] || die "no SHROUD_WRITE_KEY in $KEYS_FILE: run the backend's infra/deploy-backend.sh"

step "Server: $ENDPOINT"
curl -fsS --max-time 10 "$ENDPOINT/v1/health" >/dev/null || die "$ENDPOINT/v1/health does not answer"
echo "ok: healthy"

step "Write key for app shroud"
id="release-check-$(date +%s)-$$"
now=$(python3 -I -c 'import time; print(int(time.time()*1000))')
body="{\"schema\":1,\"app\":{\"name\":\"shroud\",\"version\":\"0\",\"process\":\"gui\"},\"install_id\":\"$id\",\"session_id\":\"$id\",\"sent_at\":$now,\"items\":[]}"
code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 -H 'content-type: application/json' \
  -H @<(printf 'x-tracer-key: %s\n' "$KEY") --data "$body" "$ENDPOINT/v1/ingest")
curl -sS -o /dev/null --max-time 10 -H 'content-type: application/json' -H @<(printf 'x-tracer-key: %s\n' "$KEY") \
  --data "{\"schema\":1,\"install_id\":\"$id\"}" "$ENDPOINT/v1/forget" || true
[ "$code" = 200 ] || die "the server refused the shroud write key (HTTP $code): deploy the backend first"
echo "ok: accepted"

export TELEMETRY=1 SHROUD_BUILD_TRACE_ENDPOINT="$ENDPOINT" SHROUD_BUILD_TRACE_KEY="$KEY"
[ "$WIN" = 1 ] && { step "Windows"; scripts/package-windows.sh; }
[ "$MAC" = 1 ] && { step "macOS"; scripts/package-macos.sh; }
unset SHROUD_BUILD_TRACE_KEY KEY

if [ "$UPLOAD" = 1 ]; then
  step "itch.io"
  scripts/upload-itch.sh
fi
step "Done (telemetry on, $ENDPOINT)"
