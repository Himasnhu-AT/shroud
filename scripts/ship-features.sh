# Sourced by package-macos.sh and package-windows.sh: which cargo features a ship build gets.
#
#   TELEMETRY=0 (default)  --no-default-features: no ashlar crate at all, nothing is ever sent.
#   TELEMETRY=1            --no-default-features --features telemetry: crash, performance and match
#                          reports (ashlar-trace only; never the control port, MCP or devtools).
#                          Needs SHROUD_BUILD_TRACE_ENDPOINT and SHROUD_BUILD_TRACE_KEY, which the
#                          game bakes in (scripts/release.sh sets them from the backend's keys).
TELEMETRY="${TELEMETRY:-0}"
SHROUD_BUILD_SHA="${SHROUD_BUILD_SHA:-$(git rev-parse --short HEAD 2>/dev/null || true)}"
export SHROUD_BUILD_SHA
if [ "$TELEMETRY" = 1 ]; then
  [ -n "${SHROUD_BUILD_TRACE_ENDPOINT:-}" ] && [ -n "${SHROUD_BUILD_TRACE_KEY:-}" ] || {
    echo "error: TELEMETRY=1 needs SHROUD_BUILD_TRACE_ENDPOINT and SHROUD_BUILD_TRACE_KEY (run scripts/release.sh)" >&2; exit 1; }
  export SHROUD_BUILD_TRACE_ENDPOINT SHROUD_BUILD_TRACE_KEY
  FEATURES=(--no-default-features --features telemetry)
  SHIP_KIND="telemetry on"
else
  # Never bake a server into a build that isn't meant to report.
  unset SHROUD_BUILD_TRACE_ENDPOINT SHROUD_BUILD_TRACE_KEY
  FEATURES=(--no-default-features)
  SHIP_KIND="telemetry off"
fi

# The ship build's dependency tree: no ashlar crate, or (telemetry) only ashlar-trace and its proto.
check_ship_tree() {
  local found
  found=$(cargo tree -p game "${FEATURES[@]}" -e normal "$@" | grep -o 'ashlar-[a-z-]*' | sort -u || true)
  if [ "$TELEMETRY" = 1 ]; then found=$(grep -vxE 'ashlar-trace|ashlar-trace-proto' <<<"$found" || true); fi
  if [ -n "$found" ]; then echo "error: dev tooling in the ship build: $found" >&2; exit 1; fi
  echo "ok: $SHIP_KIND"
}

# The built binary carries the server address exactly when telemetry is on.
check_baked() {
  local bin="$1" host="${SHROUD_BUILD_TRACE_ENDPOINT:-}"
  if [ "$TELEMETRY" = 1 ]; then
    LC_ALL=C grep -q -a -F "$host" "$bin" || { echo "error: $bin does not carry $host" >&2; exit 1; }
    echo "ok: $bin reports to $host"
  elif LC_ALL=C grep -q -a -E 'https://api\.marlstudio\.com' "$bin"; then
    echo "error: $bin carries a telemetry address but TELEMETRY is off (stale build?)" >&2; exit 1
  fi
}
