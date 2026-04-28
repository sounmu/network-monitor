#!/bin/bash

# Target SSH host for deployment (override via environment variable)
if [ -z "${DEPLOY_HOST}" ]; then
  echo "❌ Error: DEPLOY_HOST environment variable is required."
  echo "   Usage: DEPLOY_HOST=your-server ./deploy.sh"
  exit 1
fi

# 1. Sync source files to remote server
echo "📤  [1/2] Syncing source to remote..."
rsync -avz --delete \
    --exclude 'target/' \
    --exclude '.env' \
    ./ "${DEPLOY_HOST}":~/netsentinel/server/

# 2. Rebuild and restart via Docker Compose on remote
echo "🔄  [2/2] Building & deploying (Docker Compose)..."
ssh "${DEPLOY_HOST}" "cd ~/netsentinel && docker compose up -d --build server"

echo "✨ Deployment complete!"
