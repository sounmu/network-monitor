# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in this project, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please email the maintainer directly at the email address listed in the GitHub profile: [@sounmu](https://github.com/sounmu)

### What to include

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

### Response timeline

- **Acknowledgement**: Within 48 hours
- **Initial assessment**: Within 1 week
- **Fix release**: As soon as practical, depending on severity

## Supported Versions

| Version | Supported |
|---------|-----------|
| >= 0.4  | Yes       |
| < 0.4   | No        |

## Security Best Practices for Self-Hosters

- Always deploy behind a reverse proxy (Cloudflare Tunnel, nginx, Caddy) with HTTPS
- Use strong, unique values for `JWT_SECRET` (`openssl rand -hex 32`) — **this is now the only long-lived secret**, since v0.4.0 retired the Postgres database in favour of embedded SQLite
- Ensure the repo-root `.env` is `chmod 600` (bootstrap script does this automatically) — it contains the plaintext `JWT_SECRET`
- Back up `./data/netsentinel.db` regularly; see [`docs/DEPLOYMENT.md`](./docs/DEPLOYMENT.md) §5 for the `VACUUM INTO` pattern. The file contains password hashes and refresh-token hashes, so treat backups with the same care as the live DB
- Keep Docker images updated
- Restrict `ALLOWED_ORIGINS` to your actual domain (not `*`)
- The server container runs as `root` inside the container so the bind-mounted `./data` SQLite file works without host-side `chown` dances. The deployment model assumes the container is fronted by Tailscale / WireGuard / Cloudflare Tunnel, so the only externally reachable surface is the dashboard HTTP port — defended by the in-app auth, SSRF, and input-validation layers documented above. If you intend to expose the container directly to the public internet, drop privileges in your own compose override (`user: "1000:1000"`) and pre-create `./data` with matching ownership
