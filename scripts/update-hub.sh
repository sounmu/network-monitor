#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
# NetSentinel hub — updater
#
# Refreshes the local repo (compose + scripts + docs) and recreates the
# server container on a newer image. Existing data, .env, and any
# `docker-compose.override.yml` are preserved — this is the same flow
# documented in docs/DEPLOYMENT.md §3, scripted for one-line use.
#
# Typical usage on the hub host:
#
#     bash ~/netsentinel/scripts/update-hub.sh                # → latest
#     bash ~/netsentinel/scripts/update-hub.sh --version v0.5.1
#
# Curl-able variant (handy in cron):
#
#     curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/update-hub.sh \
#       | bash -s -- --install-dir ~/netsentinel
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail

INSTALL_DIR="${NS_INSTALL_DIR:-${HOME}/netsentinel}"
NEW_VERSION=""
SKIP_GIT_PULL=0

print_help() {
  cat <<'HLP'
NetSentinel hub updater

Pulls the latest scripts/compose/docs from git and recreates the
server container on a newer published image. Database, .env, and any
docker-compose.override.yml are left untouched.

Usage:
  bash update-hub.sh [options]

Options:
  --install-dir DIR     where install-hub.sh placed the stack
                        (default: $HOME/netsentinel)            env: NS_INSTALL_DIR
  --version TAG         pin the server image to a specific release
                        (e.g. v0.5.1) by writing NETSENTINEL_VERSION
                        into .env. Omit to keep whatever .env says,
                        or `latest` if unset.
  --skip-git-pull       only refresh the docker image; do not touch
                        local docs / scripts (handy if you have local
                        commits on the install dir).
  -h, --help

Examples:
  # latest published image, fast-forward git
  bash update-hub.sh

  # pin to a specific release tag and update
  bash update-hub.sh --version v0.5.1

  # only refresh the image, leave local repo alone
  bash update-hub.sh --skip-git-pull
HLP
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --install-dir)   INSTALL_DIR="${2:-}"; shift 2 ;;
    --install-dir=*) INSTALL_DIR="${1#*=}"; shift ;;
    --version)       NEW_VERSION="${2:-}"; shift 2 ;;
    --version=*)     NEW_VERSION="${1#*=}"; shift ;;
    --skip-git-pull) SKIP_GIT_PULL=1; shift ;;
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

if [[ ! -d "$INSTALL_DIR/.git" ]]; then
  echo "❌ '$INSTALL_DIR' is not a git checkout." >&2
  echo "    Pass --install-dir, set NS_INSTALL_DIR, or re-install with install-hub.sh." >&2
  exit 1
fi
cd "$INSTALL_DIR"

if [[ ! -f docker-compose.yml ]]; then
  echo "❌ docker-compose.yml missing in $INSTALL_DIR." >&2
  exit 1
fi
if [[ ! -f .env ]]; then
  echo "❌ .env missing — run scripts/bootstrap.sh first." >&2
  exit 1
fi

# ── pin .env version if requested ───────────────────────────────────
if [[ -n "$NEW_VERSION" ]]; then
  if grep -q '^NETSENTINEL_VERSION=' .env; then
    # portable in-place edit (BSD + GNU sed)
    tmp="$(mktemp)"
    awk -v v="$NEW_VERSION" '
      /^NETSENTINEL_VERSION=/ { print "NETSENTINEL_VERSION=" v; next }
      { print }
    ' .env > "$tmp"
    mv "$tmp" .env
  else
    printf '\nNETSENTINEL_VERSION=%s\n' "$NEW_VERSION" >> .env
  fi
  echo "✅ Pinned server image tag to ${NEW_VERSION} in .env"
fi

# ── refresh local checkout (compose / docs / scripts) ───────────────
if [[ $SKIP_GIT_PULL -ne 1 ]]; then
  branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)"
  echo "▶ git fetch --tags + pull on branch ${branch}…"
  git fetch --tags origin
  if ! git pull --ff-only origin "$branch"; then
    echo "⚠ Could not fast-forward (local commits or diverged branch?)."
    echo "  Resolve manually:"
    echo "      cd $INSTALL_DIR && git status"
    echo "  Or re-run with --skip-git-pull to update only the image."
    exit 1
  fi
fi

# ── refresh the image ───────────────────────────────────────────────
echo "▶ docker compose pull server…"
docker compose pull server

echo "▶ docker compose up -d server (recreates the container)…"
docker compose up -d server

# ── smoke test ──────────────────────────────────────────────────────
if [[ -x ./scripts/smoke-test.sh ]]; then
  echo "▶ Running smoke test…"
  if ! ./scripts/smoke-test.sh; then
    cat >&2 <<'EOM'
⚠ Smoke test failed. Inspect with:
    docker compose logs --tail=80 server
    ./scripts/doctor.sh
EOM
    exit 1
  fi
fi

ver="$(grep -E '^NETSENTINEL_VERSION=' .env 2>/dev/null | tail -n1 | cut -d= -f2- || true)"
ver="${ver:-latest}"
echo "✅ Hub updated to ghcr.io/sounmu/netsentinel-server:${ver}"
