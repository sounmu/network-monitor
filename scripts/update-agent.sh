#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
# NetSentinel agent — updater
#
# Re-runs `install-agent.sh` using the JWT_SECRET, AGENT_PORT, and
# AGENT_BIND already saved in /etc/netsentinel/agent.env. This is the
# same install-is-update path the agent has always supported, but
# without making the operator paste the JWT_SECRET again on every host.
#
# Typical usage on each agent host:
#
#     curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/update-agent.sh \
#       | sudo bash                                # → latest release
#     curl -fsSL .../update-agent.sh \
#       | sudo bash -s -- --ref v0.5.1             # → pinned tag
#
# Locally next to install-agent.sh:
#
#     sudo /usr/local/bin/update-agent || \
#     sudo bash ./scripts/update-agent.sh --ref v0.5.1
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail

CONFIG_FILE="${NS_CONFIG_FILE:-/etc/netsentinel/agent.env}"
REF="${NS_REF:-latest}"
INSTALLER_URL="${NS_INSTALLER_URL:-}"
EXTRA_ARGS=()

print_help() {
  cat <<HLP
NetSentinel agent updater

Refreshes the agent binary (and unit file) using the credentials
already written to ${CONFIG_FILE} by install-agent.sh. You almost
never need to pass --jwt-secret or --port again.

Usage:
  sudo bash update-agent.sh [options]

Options:
  --ref TAG               release tag to install [latest]    env: NS_REF
                          use with --build-from-source for branches
  --build-from-source     rebuild via cargo from --ref
                          (requires git + Rust toolchain)
  --installer-url URL     override the install-agent.sh URL  env: NS_INSTALLER_URL
                          (default: main for latest, otherwise --ref tag)
  --config-file PATH      agent.env to read credentials from env: NS_CONFIG_FILE
                          (default: /etc/netsentinel/agent.env)
  -h, --help

Examples:
  # latest release
  sudo bash update-agent.sh

  # pin to a specific tag
  sudo bash update-agent.sh --ref v0.5.1

  # rebuild from a branch (e.g. testing a fix before release)
  sudo bash update-agent.sh --build-from-source --ref dev
HLP
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ref)               REF="${2:-}"; shift 2 ;;
    --ref=*)             REF="${1#*=}"; shift ;;
    --build-from-source) EXTRA_ARGS+=(--build-from-source); shift ;;
    --installer-url)     INSTALLER_URL="${2:-}"; shift 2 ;;
    --installer-url=*)   INSTALLER_URL="${1#*=}"; shift ;;
    --config-file)       CONFIG_FILE="${2:-}"; shift 2 ;;
    --config-file=*)     CONFIG_FILE="${1#*=}"; shift ;;
    -h|--help)           print_help; exit 0 ;;
    *) echo "❌ Unknown argument: $1" >&2; echo "    Try --help" >&2; exit 2 ;;
  esac
done

if [[ -z "$INSTALLER_URL" ]]; then
  if [[ "$REF" == "latest" ]]; then
    INSTALLER_URL="https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/install-agent.sh"
  else
    INSTALLER_URL="https://raw.githubusercontent.com/sounmu/netsentinel/${REF}/scripts/install-agent.sh"
  fi
fi

if [[ $EUID -ne 0 ]]; then
  echo "❌ Must run as root (use sudo)." >&2
  exit 1
fi

if [[ ! -r "$CONFIG_FILE" ]]; then
  cat >&2 <<EOM
❌ ${CONFIG_FILE} not found or unreadable.

Looks like the agent was never installed via install-agent.sh on this
host (or the config lives elsewhere — pass --config-file).

To bootstrap from scratch instead:
    curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/install-agent.sh \\
      | sudo bash -s -- --jwt-secret "<paste-the-hub-secret>"
EOM
  exit 1
fi

# Load saved credentials. CONFIG_FILE format is plain KEY=VALUE.
JWT_SECRET=""
AGENT_PORT=""
AGENT_BIND=""
# shellcheck disable=SC1090
. "$CONFIG_FILE"

if [[ -z "${JWT_SECRET}" ]]; then
  echo "❌ JWT_SECRET missing from ${CONFIG_FILE}." >&2
  exit 1
fi

cmd=(--jwt-secret "$JWT_SECRET" --ref "$REF")
[[ -n "${AGENT_PORT}" ]] && cmd+=(--port "$AGENT_PORT")
[[ -n "${AGENT_BIND}" ]] && cmd+=(--bind "$AGENT_BIND")
if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
  cmd+=("${EXTRA_ARGS[@]}")
fi

# Prefer a local install-agent.sh next to this script (offline-friendly,
# also picks up local edits when developing). Otherwise fetch from the
# pinned URL.
self_dir="$(cd "$(dirname "$0")" 2>/dev/null && pwd || echo "")"
if [[ -n "$self_dir" && -x "${self_dir}/install-agent.sh" ]]; then
  echo "▶ Using local installer at ${self_dir}/install-agent.sh"
  exec "${self_dir}/install-agent.sh" "${cmd[@]}"
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "❌ curl is not on PATH and no local install-agent.sh found." >&2
  exit 1
fi

echo "▶ Fetching installer from ${INSTALLER_URL}…"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
curl -fsSL "$INSTALLER_URL" -o "$tmp"
chmod 755 "$tmp"
exec bash "$tmp" "${cmd[@]}"
