#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
# NetSentinel doctor
#
# Diagnoses an install that *isn't* working. Runs ordered checks from
# the cheapest (does .env exist?) to the most involved (is the server
# container healthy?). Every failed check prints the exact command to
# run next.
#
# Usage:
#   ./scripts/doctor.sh
# ─────────────────────────────────────────────────────────────────────
set -u

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_PATH="${REPO_ROOT}/.env"

GREEN='\033[0;32m'; RED='\033[0;31m'; YELLOW='\033[1;33m'; CLEAR='\033[0m'
PASS=0; FAIL=0; WARN=0

ok()   { printf "${GREEN}✅${CLEAR} %s\n" "$1"; PASS=$((PASS+1)); }
bad()  { printf "${RED}❌${CLEAR} %s\n    ↳ %s\n" "$1" "$2"; FAIL=$((FAIL+1)); }
warn() { printf "${YELLOW}⚠️ ${CLEAR} %s\n    ↳ %s\n" "$1" "$2"; WARN=$((WARN+1)); }

echo "NetSentinel doctor — running $(date +%FT%T)"
echo

# ── 1. core tooling ────────────────────────────────────────────────
for tool in docker curl openssl; do
  if command -v "$tool" >/dev/null 2>&1; then
    ok "$tool is on PATH"
  else
    bad "$tool is missing" "install it before continuing"
  fi
done

if docker compose version >/dev/null 2>&1; then
  ok "docker compose v2 plugin available"
else
  bad "docker compose v2 plugin not found" \
      "upgrade Docker Desktop or install docker-compose-plugin"
fi

echo

# ── 2. repo state ──────────────────────────────────────────────────
if [[ -f "$ENV_PATH" ]]; then
  ok ".env exists at repo root"

  # The only long-lived secret is JWT_SECRET. SQLite deployment has
  # no DB password.
  val="$(grep -E "^JWT_SECRET=" "$ENV_PATH" | head -n1 | cut -d= -f2- || true)"
  if [[ -z "$val" || "$val" == "change_me"* ]]; then
    bad "JWT_SECRET is unset or still the placeholder" \
        "re-run ./scripts/bootstrap.sh --force to regenerate"
  elif [[ ${#val} -lt 32 ]]; then
    bad "JWT_SECRET is shorter than 32 characters (${#val})" \
        "server will refuse to start — re-run ./scripts/bootstrap.sh --force"
  else
    ok "JWT_SECRET is set (len ${#val})"
  fi
else
  bad ".env missing at repo root" \
      "run ./scripts/bootstrap.sh to generate one"
fi

# SQLite data dir — compose bind-mounts it; bootstrap creates it.
if [[ -d "${REPO_ROOT}/data" ]]; then
  ok "SQLite data directory present at ./data"
else
  bad "./data directory missing" \
      "mkdir -p ./data (or re-run ./scripts/bootstrap.sh)"
fi

echo

# ── 3. port availability ────────────────────────────────────────────
if lsof -iTCP:3000 -sTCP:LISTEN -Pn 2>/dev/null | grep -q LISTEN; then
  owner="$(lsof -iTCP:3000 -sTCP:LISTEN -Pn 2>/dev/null | awk 'NR==2 {print $1 " (pid " $2 ")"}')"
  if echo "$owner" | grep -qiE 'docker|com.docke'; then
    ok "port 3000 is held by Docker (expected when stack is up)"
  else
    warn "port 3000 is held by ${owner}" \
         "stop that process or set SERVER_PORT=… in .env"
  fi
else
  warn "port 3000 is free" \
       "the stack is not running — try 'docker compose pull server && docker compose up -d server'"
fi

echo

# ── 4. Docker stack state ──────────────────────────────────────────
if ! docker compose ps >/dev/null 2>&1; then
  bad "'docker compose ps' failed" \
      "run from the repo root and make sure the Docker daemon is up"
  echo
else
  for svc in server; do
    state="$(docker compose ps --format '{{.Name}} {{.Status}}' 2>/dev/null | awk -v s="$svc" '$1 ~ s {print $0; exit}')"
    if [[ -z "$state" ]]; then
      warn "${svc} container not running" \
           "run 'docker compose pull server && docker compose up -d server' to start it"
    elif echo "$state" | grep -qi 'healthy\|Up'; then
      ok "${svc} is ${state#* }"
    else
      bad "${svc} reported: ${state}" \
          "docker compose logs --tail=60 ${svc}"
    fi
  done

  echo

  # ── 5. live health endpoint ──────────────────────────────────────
  if curl -sf --max-time 3 http://localhost:3000/api/health >/dev/null 2>&1; then
    ok "GET /api/health returned 200"
  else
    bad "GET /api/health did not respond" \
        "check 'docker compose logs server' for startup errors (DATABASE_URL, JWT_SECRET, migrations)"
  fi
fi

echo
echo "─────────────────────────────────────────────────────────────"
printf "${GREEN}%d passed${CLEAR}  " "$PASS"
printf "${YELLOW}%d warnings${CLEAR}  " "$WARN"
printf "${RED}%d failed${CLEAR}\n" "$FAIL"

if (( FAIL == 0 )); then
  if (( WARN == 0 )); then
    echo "Everything looks healthy. Try ./scripts/smoke-test.sh for the happy-path check."
  fi
  exit 0
fi
exit 1
