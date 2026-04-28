#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE_NAME="${IMAGE_NAME:-ghcr.io/sounmu/netsentinel-server}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
NEXT_PUBLIC_API_URL="${NEXT_PUBLIC_API_URL:-}"
BUILDER_NAME="${BUILDER_NAME:-netsentinel-release-builder}"

usage() {
  cat <<'EOF'
Usage:
  scripts/publish-server-image.sh TAG [--latest]

Build and push the NetSentinel server image from the local machine.

Examples:
  scripts/publish-server-image.sh v0.4.3-beta.1
  scripts/publish-server-image.sh v0.4.3 --latest

Environment:
  IMAGE_NAME            image repository [ghcr.io/sounmu/netsentinel-server]
  PLATFORMS            buildx platforms [linux/amd64,linux/arm64]
  NEXT_PUBLIC_API_URL  baked web API URL [same-origin]
  BUILDER_NAME          buildx builder name [netsentinel-release-builder]

Notes:
  - Login first with: gh auth token | docker login ghcr.io -u USERNAME --password-stdin
  - Prerelease tags should not use --latest.
EOF
}

ensure_builder() {
  local driver

  if docker buildx inspect "$BUILDER_NAME" >/dev/null 2>&1; then
    driver="$(docker buildx inspect "$BUILDER_NAME" | awk '/^Driver:/ {print $2; exit}')"
    if [[ "$driver" != "docker-container" ]]; then
      echo "Builder '$BUILDER_NAME' uses driver '$driver'; expected docker-container." >&2
      echo "Remove it or set BUILDER_NAME to another name." >&2
      exit 1
    fi
  else
    echo "Creating buildx builder '$BUILDER_NAME' with docker-container driver"
    docker buildx create --name "$BUILDER_NAME" --driver docker-container >/dev/null
  fi

  docker buildx inspect "$BUILDER_NAME" --bootstrap >/dev/null
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -lt 1 ]]; then
  usage >&2
  exit 2
fi

tag="$1"
shift

publish_latest=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --latest)
      publish_latest=true
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ -z "$tag" ]]; then
  echo "TAG is required." >&2
  exit 2
fi

if [[ "$publish_latest" == true && "$tag" == *-* ]]; then
  echo "Refusing to publish prerelease tag '$tag' as latest." >&2
  exit 2
fi

if ! docker buildx version >/dev/null 2>&1; then
  echo "docker buildx is required." >&2
  exit 1
fi

ensure_builder

tags=(-t "${IMAGE_NAME}:${tag}")
if [[ "$publish_latest" == true ]]; then
  tags+=(-t "${IMAGE_NAME}:latest")
fi

echo "Building ${IMAGE_NAME}:${tag} for ${PLATFORMS}"
if [[ "$publish_latest" == true ]]; then
  echo "Also publishing ${IMAGE_NAME}:latest"
fi

docker buildx build \
  --builder "$BUILDER_NAME" \
  --platform "$PLATFORMS" \
  --push \
  --build-arg "NEXT_PUBLIC_API_URL=${NEXT_PUBLIC_API_URL}" \
  "${tags[@]}" \
  -f "${REPO_ROOT}/server/Dockerfile" \
  "$REPO_ROOT"
