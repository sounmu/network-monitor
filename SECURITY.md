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
| 1.x     | Yes       |
| < 1.0   | No        |

## Security Best Practices for Self-Hosters

- Always deploy behind a reverse proxy (Cloudflare Tunnel, nginx, Caddy) with HTTPS
- Use strong, unique values for `JWT_SECRET` (`openssl rand -hex 32`)
- Use strong `POSTGRES_PASSWORD` (not the default)
- Keep Docker images updated
- Restrict `ALLOWED_ORIGINS` to your actual domain (not `*`)
- The server container runs as a non-root user (`monitor`) by default
