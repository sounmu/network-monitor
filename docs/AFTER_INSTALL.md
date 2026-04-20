# After Install — first admin, first host, first agent

This page covers everything that happens *after* `docker compose up -d --build` finishes. Target time: **10 minutes** from a fresh clone to "the dashboard is showing real metrics from a real machine".

If anything below fails, run `./scripts/doctor.sh` — it prints the exact next command for each broken check.

---

## Step 1 — Verify the stack is healthy (30 seconds)

```bash
./scripts/smoke-test.sh
```

Expected output:

```
✅ /api/health responded within 20s
✅ Health payload confirms DB connectivity
✅ Web root / served (static bundle OK)
✅ /api/auth/status — first-time setup is pending (expected on fresh install)
✅ /host/?key=… static shell served

Summary: 5 passed, 0 failed

👉 Next:
    open http://localhost:3000/setup   # create the first admin account
```

Any ❌ line tells you exactly which log to inspect (usually `docker compose logs --tail=60 server`).

---

## Step 2 — Create the first admin (1 minute)

Open **<http://localhost:3000/setup>** in a browser.

The `/setup` page is only reachable while the `users` table is empty. Fill in:

| Field | Notes |
|---|---|
| Username | Any letters/digits. Case-insensitive unique. |
| Password | **≥ 8 characters** + uppercase + lowercase + digit + special character. The frontend validates live; the server enforces the same rules. |
| Confirm password | Must match. |

Click **Create admin** → you are redirected to `/login` → sign in with the same credentials.

> If you navigated to `/setup` but the page says "setup already completed", an admin was created earlier. To start over, stop the stack and delete the SQLite file:
>
> ```bash
> docker compose down
> rm data/netsentinel.db data/netsentinel.db-wal data/netsentinel.db-shm
> docker compose up -d
> ```
>
> This wipes **all** state (users, hosts, metrics, alerts). If you only want to reset the admin account, sign in at `/login` instead.

---

## Step 3 — Add your first host (1 minute, in the browser)

In the navbar, click **Agents** → **+ Add Agent**. Fill in:

| Field | Example | What it means |
|---|---|---|
| `host_key` | `192.168.1.10:9101` | The URL the SERVER will pull metrics from. Format: `host:port`. Must be reachable from the server container. Use the agent machine's LAN IP, not `localhost`. |
| `display_name` | `homeserver` | Shows up in the dashboard. |
| `scrape_interval_secs` | `10` | How often the server polls this host. |
| `load_threshold` | `4.0` | Triggers the high-load alert. |
| `ports` | `80, 443` | Comma-separated; the agent probes each and reports up/down. |
| `containers` | (blank) | Comma-separated Docker container names you want tracked. |

Hit **Save** → the host shows up in `/agents` immediately with status **`pending`**. It turns **`online`** after the first successful scrape — once the agent on that machine is answering.

---

## Step 4 — Install the agent on the target machine (~2 minutes)

NetSentinel ships a one-liner agent installer. On the machine you want to monitor, paste:

```bash
curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/install-agent.sh \
  | sudo bash -s -- --jwt-secret "PASTE_THE_JWT_SECRET_HERE"
```

Common variants:

```bash
# Default: listen on every interface, port 9101.
curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/install-agent.sh \
  | sudo bash -s -- \
      --jwt-secret "PASTE_THE_JWT_SECRET_HERE" \
      --bind "0.0.0.0" \
      --port 9101 \
      --ref main

# Tailscale-only exposure: register 100.x.y.z:9101 in the hub.
curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/install-agent.sh \
  | sudo bash -s -- \
      --jwt-secret "PASTE_THE_JWT_SECRET_HERE" \
      --bind "100.x.y.z" \
      --port 9101 \
      --ref main

# Custom port: register <agent-ip>:9200 in the hub.
curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/install-agent.sh \
  | sudo bash -s -- \
      --jwt-secret "PASTE_THE_JWT_SECRET_HERE" \
      --bind "0.0.0.0" \
      --port 9200 \
      --ref main
```

Read back the shared secret from the hub's `.env`:

```bash
# on the hub
grep ^JWT_SECRET= .env | cut -d= -f2-
```

The installer:

1. Checks for `git` and `cargo` (prints install commands if missing).
2. Clones the NetSentinel repo and installs `netsentinel-agent` via `cargo install --path` into `/usr/local/bin`.
3. Writes `/etc/netsentinel/agent.env` with the JWT + port (chmod 600).
4. Drops `/etc/systemd/system/netsentinel-agent.service` (Linux) or `/Library/LaunchDaemons/dev.netsentinel.agent.plist` (macOS), enables and starts it.
5. Prints the exact `host_key` — `<lan-ip>:9101` — you should paste into the hub UI.

Optional flags:

```bash
--port 9102            # non-default listen port
--bind 192.168.1.10    # only bind to a specific interface
--bind 100.x.y.z       # Tailscale-only native agent exposure
--prefix /opt          # binary goes to /opt/bin instead of /usr/local/bin
--ref v0.3.5           # build a specific tag / branch
--uninstall            # stop + remove service, binary, and /etc/netsentinel/
```

To update an installed native agent, re-run the installer with the same
`JWT_SECRET` and the target release/tag. The binary, service definition, and
`/etc/netsentinel/agent.env` are replaced, then the service is restarted:

```bash
curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/install-agent.sh \
  | sudo bash -s -- \
      --jwt-secret "PASTE_THE_JWT_SECRET_HERE" \
      --bind "0.0.0.0" \
      --port 9101 \
      --ref main
```

On Linux, the installer creates a systemd unit that reads
`/etc/netsentinel/agent.env`. If you change `JWT_SECRET`, `AGENT_PORT`, or
`AGENT_BIND` manually, restart the service:

```bash
sudo systemctl restart netsentinel-agent
```

### 4.1 Confirm the server picks up the agent

Back in the hub UI, the `Agents` row flips from `pending` → `online` within one scrape cycle. Live metrics land on the Overview dashboard and `/host/?key=<host_key>`.

### 4.2 If it stays `pending`

| Symptom | Fix |
|---|---|
| Agent service keeps restarting | `sudo journalctl -u netsentinel-agent --since '1 min ago'` — look for JWT / bind errors |
| Agent is up (port listens) but hub says `pending` | `docker compose exec server curl -v http://<host>:<port>/metrics` from the hub — if curl times out, open the firewall on the agent host |
| Hub logs say `401` / `403` | JWT_SECRET mismatch — recopy the hub's `.env` value into `/etc/netsentinel/agent.env` then `sudo systemctl restart netsentinel-agent` |
| Hub logs say `bincode decode error` | Agent built from a ref more than one minor release away from the hub. Rebuild one side: re-run the installer with matching `--ref` |

### 4.3 Offline alternative (no curl|bash)

If the target host can't reach GitHub, clone and build manually:

```bash
git clone https://github.com/sounmu/netsentinel.git
cd netsentinel/netsentinel-agent
cp .env.example .env          # set JWT_SECRET + AGENT_PORT
cargo build --release
./target/release/netsentinel-agent
```

Register the same `<lan-ip>:<AGENT_PORT>` in the hub's Agents page.

---

## Step 5 — (Optional) wire up one notification channel (2 minutes)

In **Alerts** → **Notification channels** → **+ Add channel**:

| Channel | Required field |
|---|---|
| Discord | Webhook URL (from the server settings of your Discord channel) |
| Slack | Incoming Webhook URL |
| Email | SMTP host / port / user / password / from / to |

Hit **Test** on the saved channel to verify delivery. Afterwards, configure thresholds in **Alerts** → **Global defaults** or per host.

---

## Total: ~10 minutes

You now have:

- A web dashboard at `http://localhost:3000`
- One admin account
- One host being scraped every 10 s
- (Optional) one notification channel with a real test notification delivered

---

## Troubleshooting

| Symptom | Most common cause | Fix |
|---|---|---|
| Dashboard shows "No agents registered" | No host added yet | See Step 3 |
| Host stuck at `pending` | Agent not running / JWT mismatch / server can't reach `host:port` | See Step 4.3 |
| Browser shows "Host Not Found" on `/host/?key=…` | You edited the URL to a value that isn't in `/api/hosts` | Register the host first (Step 3) |
| `/setup` returns 404 or redirects to `/login` | Admin already provisioned | Sign in at `/login`; reset via `TRUNCATE users` in Postgres if you truly need a fresh setup |
| `./scripts/smoke-test.sh` fails on `/api/health` | Server container still starting, or DB is unreachable | `docker compose logs --tail=60 server` — look for DATABASE_URL / migration errors |
| `./scripts/doctor.sh` flags `JWT_SECRET is shorter than 32 characters` | Manual edit, or leftover from an older example | Re-run `./scripts/bootstrap.sh --force` to regenerate the secret (⚠️ invalidates every existing agent's JWT — you will need to recopy the new value to each agent) |
| Port `3000` already in use | Another service owns it | Set `SERVER_PORT=XXXX` in `.env` and `docker compose up -d` again |

For production-specific concerns (Cloudflare Tunnel, TLS, custom hostname, reverse proxy), see [`docs/DEPLOYMENT.md`](DEPLOYMENT.md).
