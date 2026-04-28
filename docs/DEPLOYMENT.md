# Deployment & Operations

This document covers everything the localhost-first Quick Start in the README does **not**: public hostnames, TLS, Cloudflare Tunnel, reverse proxies, upgrades, rollbacks.

If you just want to try NetSentinel on your laptop or homelab, stop here and go back to [`README.md`](../README.md) → Quick Start.

---

## 1. Same-origin vs split-origin

Out of the box the dashboard and the API share one origin — whatever hostname serves port `3000`. CORS is effectively unused, `SameSite=Strict` cookies Just Work.

You only need to think about origins when an operator puts the dashboard and the API on **different hostnames**, typically behind a reverse proxy:

```
browser ─┬─→ https://dashboard.example.com    (static bundle)
         └─→ https://api.example.com          (Axum API + SSE)
```

The published `ghcr.io/sounmu/netsentinel-server` image is built for same-origin deployments. In a split-origin deployment you should either put both routes behind one public hostname, or build a custom image with the API hostname baked into the static bundle.

The custom-build path uses a `docker-compose.override.yml` next to the base compose file. Compose automatically merges it on top of `docker-compose.yml`, and the override is `.gitignore`d so it never leaks back into the upstream:

1. Create `docker-compose.override.yml`:
   ```yaml
   services:
     server:
       image: netsentinel-server:custom
       build:
         context: .
         dockerfile: server/Dockerfile
         args:
           - NEXT_PUBLIC_API_URL=${NEXT_PUBLIC_API_URL:-}
   ```
2. Build with the API hostname baked in:
   ```bash
   NEXT_PUBLIC_API_URL=https://api.example.com \
     docker compose up -d --build server
   ```
3. Add **both** hostnames to `ALLOWED_ORIGINS` in `.env`:
   ```
   ALLOWED_ORIGINS=https://dashboard.example.com,https://api.example.com
   ```
4. Make sure the reverse proxy forwards SSE correctly (`proxy_buffering off` for nginx, or native WebSocket/SSE handling for Caddy / Traefik).

---

## 2. Cloudflare Tunnel (optional)

NetSentinel is designed to run behind Cloudflare Zero Trust, with the server **pulling** from agents through the tunnel. Nothing in `docker-compose.yml` assumes this — you add it with an override.

### 2.1 Base: run the tunnel as a sibling service

Create `docker-compose.tunnel.yml`:

```yaml
services:
  tunnel:
    image: cloudflare/cloudflared:latest
    restart: unless-stopped
    command: tunnel --no-autoupdate run
    environment:
      - TUNNEL_TOKEN=${CLOUDFLARE_TUNNEL_TOKEN}
    networks:
      - netsentinel

networks:
  netsentinel:
    driver: bridge
```

Add `CLOUDFLARE_TUNNEL_TOKEN=…` to `.env` and bring both compose files up:

```bash
docker compose -f docker-compose.yml -f docker-compose.tunnel.yml up -d
```

### 2.2 Configure the tunnel

In the Cloudflare Zero Trust dashboard, route your public hostname to the internal service. Example:

| Public hostname | Service URL inside the stack |
|---|---|
| `https://dashboard.example.com` | `http://server:3000` |

Both the UI and API are on the same origin, so a single hostname is all you need.

### 2.3 Scrape agents over the tunnel

Agents register their own public hostname (e.g. `agent1.example.com`) in Cloudflare and the server reaches them as `http://agent1.example.com/metrics`. The `host_key` in `/api/hosts` should then be `agent1.example.com:443` (port is required in the key format).

---

## 3. Upgrading

NetSentinel migrates the DB schema forward on every server start via `sqlx::migrate!()`. To upgrade:

```bash
cd netsentinel
git pull                         # get the latest compose/scripts/docs
docker compose pull server       # download the published image
docker compose up -d server      # restart on the new image
./scripts/smoke-test.sh          # verify the upgrade
```

There is no downtime-safe rolling upgrade yet: `docker compose up` recreates the server container atomically (~a few seconds blackout). DB data survives because `./data/` is a bind-mount — the SQLite file, its `-wal` sidecar, and its `-shm` sidecar all persist across container re-creation.

**After upgrade** read the new release's CHANGELOG for any breaking surface — API contract changes, env var additions, or migrations that change behaviour.

---

## 4. Rolling back

The repository tags every release (`v0.4.x+`). To roll back to a known-good version, pin the image tag in `.env`:

```bash
cd netsentinel
# edit .env: NETSENTINEL_VERSION=v0.4.2
docker compose pull server
docker compose up -d server
```

Migrations are forward-only. If you roll back across a migration that added a column or widened a CHECK constraint, the older binary still works against the newer schema — it just won't use the new column. If you roll back across a migration that **removed** something, you will need to restore from a backup.

---

## 5. Backups

All server state lives in a single SQLite file at `./data/netsentinel.db`, WAL-mode. Two backup strategies work; pick one.

### Option A — online `VACUUM INTO` (recommended)

SQLite's `VACUUM INTO` produces a **crash-consistent** copy of the database without stopping writes. Safe to run against a live server.

```bash
# Daily backup (cron-friendly)
docker compose exec -T server /app/server --vacuum-into /app/data/backups/netsentinel-$(date +%F).db
```

If your server binary does not expose a `--vacuum-into` flag (pre-v0.5.1), invoke SQLite directly through the container:

```bash
docker compose exec -T server sh -c \
  "sqlite3 /app/data/netsentinel.db \"VACUUM INTO '/app/data/backups/netsentinel-$(date +%F).db'\""
```

Compress the resulting file if long-term storage matters:

```bash
gzip data/backups/netsentinel-$(date +%F).db
```

### Option B — stop-the-world file copy

Good enough for a homelab where a 2-second blackout is fine:

```bash
docker compose stop server
cp -a data/netsentinel.db data/backups/netsentinel-$(date +%F).db
docker compose start server
```

Do **not** `cp` a live WAL database without stopping the server — the three-file set (`.db`, `.db-wal`, `.db-shm`) is only consistent at a commit boundary that `cp` cannot guarantee.

### Restore

Stop the server, replace the files, and start it again:

```bash
docker compose down
rm data/netsentinel.db data/netsentinel.db-wal data/netsentinel.db-shm
cp data/backups/netsentinel-YYYY-MM-DD.db data/netsentinel.db
docker compose up -d server
```

The WAL and shm sidecars are regenerated on next open. `./data/` is the canonical storage — snapshot the whole directory if your volume driver supports it.

---

## 6. Image tagging (for CI pipelines)

Every tagged release on GitHub produces a multi-arch Docker image and matching prebuilt agent binaries:

```
ghcr.io/sounmu/netsentinel-server:<release-tag>
ghcr.io/sounmu/netsentinel-server:latest
netsentinel-agent-linux-amd64.tar.gz
netsentinel-agent-linux-arm64.tar.gz
netsentinel-agent-darwin-amd64.tar.gz
netsentinel-agent-darwin-arm64.tar.gz
SHA256SUMS
```

Pin to `<release-tag>` for reproducible deploys:

```yaml
services:
  server:
    image: ghcr.io/sounmu/netsentinel-server:v0.4.2
```

---

## 7. Port map (for firewall configuration)

| Port | Who listens | Exposed? |
|---|---|---|
| `3000` | Axum (API + static web) | Yes, via `docker-compose.yml` `ports:` |
| `9101` | Agent (default) | Yes, but only on the agent's LAN / tunnel — the server *pulls* |

Nothing else is reachable from outside the stack.
