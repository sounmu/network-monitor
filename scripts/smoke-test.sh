#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
# NetSentinel smoke test
#
# Verifies a freshly-installed stack. Exits 0 when everything responds
# as expected, non-zero with a clear ❌ line otherwise. Each check is
# self-contained and tells the user exactly what to look at when it
# fails — no "command not found" / "connection refused" raw output.
#
# Usage:
#   ./scripts/smoke-test.sh                  # defaults (localhost:3000)
#   BASE_URL=https://dash.example.com ./scripts/smoke-test.sh
#   TIMEOUT_SECS=30 ./scripts/smoke-test.sh  # how long to wait for health
# ─────────────────────────────────────────────────────────────────────
set -u

BASE_URL="${BASE_URL:-http://localhost:3000}"
TIMEOUT_SECS="${TIMEOUT_SECS:-20}"

PASS=0
FAIL=0

say_pass() { echo "✅ $1"; PASS=$((PASS+1)); }
say_fail() { echo "❌ $1"; if [[ $# -gt 1 ]]; then echo "    ↳ $2"; fi; FAIL=$((FAIL+1)); }

check_tool() {
  if ! command -v "$1" >/dev/null 2>&1; then
    say_fail "$1 not installed" "install it before running the smoke test"
    exit 1
  fi
}

check_tool curl
check_tool jq

echo "▶ Target: ${BASE_URL}"
echo

# ── 1. /api/health (retry up to TIMEOUT_SECS) ───────────────────────
deadline=$((SECONDS + TIMEOUT_SECS))
health_ok=0
while (( SECONDS < deadline )); do
  if curl -sf --max-time 3 "${BASE_URL}/api/health" > /tmp/ns-health.json 2>/dev/null; then
    health_ok=1; break
  fi
  sleep 2
done
if (( health_ok )); then
  say_pass "/api/health responded within ${TIMEOUT_SECS}s"
else
  say_fail "/api/health did not respond within ${TIMEOUT_SECS}s" \
    "docker compose logs --tail=60 server  # shows startup errors"
  echo
  echo "Summary: ${PASS} passed, ${FAIL} failed"
  exit 1
fi

# ── 2. DB connectivity reported by /api/health ──────────────────────
if jq -e '.db == "ok" or .status == "ok"' /tmp/ns-health.json > /dev/null 2>&1; then
  say_pass "Health payload confirms DB connectivity"
else
  say_fail "Health payload did not confirm DB" \
    "payload was: $(cat /tmp/ns-health.json)"
fi

# ── 3. Web static bundle root ──────────────────────────────────────
if curl -sf --max-time 3 "${BASE_URL}/" | grep -q '<html'; then
  say_pass "Web root / served (static bundle OK)"
else
  say_fail "Web root / did not return HTML" \
    "STATIC_ASSETS_DIR may not have been populated inside the image"
fi

# ── 4. /api/auth/status tells us if setup is needed ────────────────
auth_json="$(curl -sf --max-time 3 "${BASE_URL}/api/auth/status" 2>/dev/null || echo '')"
if [[ -n "$auth_json" ]]; then
  setup_needed="$(echo "$auth_json" | jq -r '.setup_needed // .needs_setup // empty' 2>/dev/null || true)"
  if [[ "$setup_needed" == "true" ]]; then
    say_pass "/api/auth/status — first-time setup is pending (expected on fresh install)"
    NEXT_STEP="open ${BASE_URL}/setup   # create the first admin account"
  elif [[ "$setup_needed" == "false" ]]; then
    say_pass "/api/auth/status — an admin account already exists"
    NEXT_STEP="open ${BASE_URL}/login   # admin already provisioned; just sign in"
  else
    say_pass "/api/auth/status responded (payload format newer than this script, still OK)"
    NEXT_STEP="open ${BASE_URL}/        # follow the UI prompt for first-time setup"
  fi
else
  say_fail "/api/auth/status did not respond" \
    "older server build missing the endpoint? upgrade to v0.4+"
  NEXT_STEP="open ${BASE_URL}/"
fi

# ── 5. Host-detail static shell ────────────────────────────────────
if curl -sf --max-time 3 "${BASE_URL}/host/?key=smoke-test:9101" > /dev/null 2>&1; then
  say_pass "/host/?key=… static shell served"
else
  say_fail "/host/?key=… did not respond" \
    "the web bundle may be stale — update with 'docker compose pull server && docker compose up -d server'"
fi

echo
echo "─────────────────────────────────────────────────────────────"
echo "Summary: ${PASS} passed, ${FAIL} failed"
if (( FAIL == 0 )); then
  echo
  echo "👉 Next:"
  echo "    ${NEXT_STEP}"
  echo "    See docs/AFTER_INSTALL.md for the first-host checklist."
  exit 0
else
  echo
  echo "See docs/AFTER_INSTALL.md → 'Troubleshooting' for each red line above."
  exit 1
fi
