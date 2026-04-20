#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
# NetSentinel hub — one-liner installer
#
# Spins up the full stack on a fresh Linux box. Typical usage:
#
#     curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/install-hub.sh | bash
#
# Steps, in order:
#   1. Verify Docker + Compose v2 are available.
#   2. Clone (or pull) the repo into $HOME/netsentinel.
#   3. Run scripts/bootstrap.sh to generate .env with random secrets.
#   4. `docker compose up -d --build` to build & start the single
#      server+web container.
#   5. Run scripts/smoke-test.sh to verify the install.
#   6. Print the JWT_SECRET (so the operator can paste it into the
#      agent installers on every host they want to monitor) and the
#      URL of the dashboard.
#
# Safe to re-run: step 2 pulls instead of re-cloning, step 3 skips
# secret generation if .env already exists.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail

REPO_URL="${NS_REPO_URL:-https://github.com/sounmu/netsentinel.git}"
REF="${NS_REF:-main}"
INSTALL_DIR="${NS_INSTALL_DIR:-${HOME}/netsentinel}"

# ── prerequisites ──────────────────────────────────────────────────
for tool in git docker curl openssl; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "❌ '$tool' is not on PATH." >&2
    echo "    Install it first and re-run this script." >&2
    exit 1
  fi
done

if ! docker compose version >/dev/null 2>&1; then
  echo "❌ docker compose v2 plugin is missing." >&2
  echo "    Debian/Ubuntu: apt install docker-compose-plugin" >&2
  echo "    Or upgrade to a recent Docker Desktop / Docker Engine." >&2
  exit 1
fi

# ── clone or update ─────────────────────────────────────────────────
if [[ -d "${INSTALL_DIR}/.git" ]]; then
  echo "▶ Updating ${INSTALL_DIR}…"
  git -C "${INSTALL_DIR}" fetch --tags origin
  git -C "${INSTALL_DIR}" checkout "${REF}"
  git -C "${INSTALL_DIR}" pull --ff-only origin "${REF}" || true
else
  echo "▶ Cloning ${REPO_URL} into ${INSTALL_DIR}…"
  git clone --branch "${REF}" --depth 1 "${REPO_URL}" "${INSTALL_DIR}"
fi
cd "${INSTALL_DIR}"

# ── bootstrap .env ─────────────────────────────────────────────────
if [[ ! -f .env ]]; then
  ./scripts/bootstrap.sh
else
  echo "ℹ️  .env already exists at ${INSTALL_DIR}/.env — keeping it."
fi

# ── build + start ──────────────────────────────────────────────────
echo "▶ docker compose up -d --build (first run downloads images + compiles the Rust server — ~5 min)…"
docker compose up -d --build

# ── smoke test ─────────────────────────────────────────────────────
echo "▶ Running smoke test…"
if ! ./scripts/smoke-test.sh; then
  cat >&2 <<'EOM'

Smoke test did not fully pass. Diagnose with:
    ./scripts/doctor.sh
    docker compose logs --tail=80 server
EOM
  exit 1
fi

# ── pairing info ───────────────────────────────────────────────────
jwt="$(grep ^JWT_SECRET= .env | cut -d= -f2-)"
port="$(grep ^SERVER_PORT= .env 2>/dev/null | cut -d= -f2- || true)"
port="${port:-3000}"
lan_ip=""
if command -v hostname >/dev/null 2>&1 && hostname -I >/dev/null 2>&1; then
  lan_ip="$(hostname -I | awk '{print $1}')"
fi
[[ -z "${lan_ip}" ]] && lan_ip="localhost"

cat <<EOM

─────────────────────────────────────────────────────────────────────
✅ Hub is up at http://${lan_ip}:${port}/

👉 Next:
    1. open http://${lan_ip}:${port}/setup   # create the first admin

    2. On every machine you want to monitor, run the one-liner agent
       installer (replace the trailing secret with the value below):

       curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/${REF}/scripts/install-agent.sh \\
         | sudo bash -s -- --jwt-secret "${jwt}"

       The agent will print the host_key to paste into the hub's
       Agents UI.

    3. Full walk-through (first admin, first host, first agent,
       notification channels):
           ${INSTALL_DIR}/docs/AFTER_INSTALL.md

Keep this terminal output somewhere safe — the JWT_SECRET above is
what lets each agent authenticate to this hub. It is ALSO stored
(chmod 600) in ${INSTALL_DIR}/.env — read it back with:

    grep ^JWT_SECRET= ${INSTALL_DIR}/.env | cut -d= -f2-
─────────────────────────────────────────────────────────────────────
EOM
