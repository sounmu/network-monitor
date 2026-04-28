#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
# NetSentinel hub — uninstaller
#
# Reverses what `install-hub.sh` did. Default behaviour stops the stack
# and removes the containers / network, but KEEPS the SQLite database
# and `.env` so a re-install picks up where you left off. Pass `--purge`
# to wipe everything, including the install directory.
#
# Typical usage on the hub host:
#
#     bash ~/netsentinel/scripts/remove-hub.sh
#     # or, to obliterate data + config + repo dir:
#     bash ~/netsentinel/scripts/remove-hub.sh --purge
#
# Curl-able variant (when the install dir is unknown / partially gone):
#
#     curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/remove-hub.sh \
#       | bash -s -- --install-dir ~/netsentinel --purge
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail

INSTALL_DIR="${NS_INSTALL_DIR:-${HOME}/netsentinel}"
PURGE=0
REMOVE_IMAGE=0
ASSUME_YES=0

print_help() {
  cat <<'HLP'
NetSentinel hub uninstaller

Stops the stack started by install-hub.sh. By default, your SQLite
database under <install-dir>/data/ and your .env are preserved, so a
later `install-hub.sh` (or `docker compose up -d server`) resumes
seamlessly. Pass --purge to wipe everything.

Usage:
  bash remove-hub.sh [options]

Options:
  --install-dir DIR     where install-hub.sh placed the stack
                        (default: $HOME/netsentinel)            env: NS_INSTALL_DIR
  --remove-image        also `docker rmi` the pulled server image
  --purge               also delete data/, .env, and the install dir
                        ⚠ this destroys your SQLite DB
  -y, --yes             skip the interactive confirmation
  -h, --help

Examples:
  # stop the stack but keep the DB + .env so you can re-install later
  bash remove-hub.sh

  # full wipe, no prompt
  bash remove-hub.sh --purge --remove-image -y
HLP
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --install-dir)   INSTALL_DIR="${2:-}"; shift 2 ;;
    --install-dir=*) INSTALL_DIR="${1#*=}"; shift ;;
    --remove-image)  REMOVE_IMAGE=1; shift ;;
    --purge)         PURGE=1; shift ;;
    -y|--yes)        ASSUME_YES=1; shift ;;
    -h|--help)       print_help; exit 0 ;;
    *) echo "❌ Unknown argument: $1" >&2; echo "    Try --help" >&2; exit 2 ;;
  esac
done

if ! command -v docker >/dev/null 2>&1; then
  echo "❌ docker is not on PATH." >&2
  exit 1
fi
if ! docker compose version >/dev/null 2>&1; then
  echo "❌ docker compose v2 plugin is missing." >&2
  exit 1
fi

if [[ ! -d "$INSTALL_DIR" ]]; then
  echo "ℹ️  Install dir '$INSTALL_DIR' not found — nothing to do."
  exit 0
fi
if [[ ! -f "$INSTALL_DIR/docker-compose.yml" ]]; then
  echo "❌ '$INSTALL_DIR/docker-compose.yml' missing." >&2
  echo "    Pass --install-dir or set NS_INSTALL_DIR if it lives elsewhere." >&2
  exit 1
fi

# ── confirmation banner ─────────────────────────────────────────────
cat <<EOM
About to remove the NetSentinel hub at:
    ${INSTALL_DIR}

Will:
  • docker compose down --remove-orphans (stops + removes containers/network)
EOM
[[ $REMOVE_IMAGE -eq 1 ]] && echo "  • docker rmi the pulled server image"
if [[ $PURGE -eq 1 ]]; then
  cat <<EOM
  • rm -rf ${INSTALL_DIR}/data    ← deletes the SQLite DB (irreversible)
  • rm -f  ${INSTALL_DIR}/.env
  • rm -rf ${INSTALL_DIR}         ← removes the cloned repo
EOM
else
  echo "  (data/, .env, and the repo dir are kept — pass --purge to wipe them)"
fi

if [[ $ASSUME_YES -ne 1 ]]; then
  read -r -p "Proceed? [y/N] " ans
  case "$ans" in
    y|Y|yes|YES) ;;
    *) echo "Aborted."; exit 0 ;;
  esac
fi

# ── stop the stack ──────────────────────────────────────────────────
cd "$INSTALL_DIR"
echo "▶ docker compose down --remove-orphans…"
docker compose down --remove-orphans || true

# ── optionally remove the image ─────────────────────────────────────
if [[ $REMOVE_IMAGE -eq 1 ]]; then
  ver="$(grep -E '^NETSENTINEL_VERSION=' .env 2>/dev/null | tail -n1 | cut -d= -f2- || true)"
  ver="${ver:-latest}"
  resolved="ghcr.io/sounmu/netsentinel-server:${ver}"
  echo "▶ docker rmi ${resolved}…"
  docker rmi "$resolved" 2>/dev/null || true
fi

# ── purge ───────────────────────────────────────────────────────────
if [[ $PURGE -eq 1 ]]; then
  cd /
  if [[ -d "${INSTALL_DIR}/data" ]]; then
    echo "▶ rm -rf ${INSTALL_DIR}/data…"
    rm -rf "${INSTALL_DIR}/data"
  fi
  if [[ -f "${INSTALL_DIR}/.env" ]]; then
    echo "▶ rm -f ${INSTALL_DIR}/.env…"
    rm -f "${INSTALL_DIR}/.env"
  fi
  echo "▶ rm -rf ${INSTALL_DIR}…"
  rm -rf "${INSTALL_DIR}"
  echo "✅ Hub fully removed."
else
  echo "✅ Hub stopped. Data + config kept under ${INSTALL_DIR}/."
  echo "   To wipe everything later:  bash $0 --purge"
fi
