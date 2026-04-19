# netsentinel-web

Next.js frontend dashboard for **NetSentinel**. Displays real-time CPU, memory, load, port, and Docker container metrics with interactive time-series charts.

## Quick Start

```bash
cp .env.example .env   # configure API URL
npm install
npm run dev             # http://localhost:3001
```

## Tech Stack

- **Next.js 16** (App Router, `output: 'export'`)
- **Recharts** — time-series charts
- **SWR** — data fetching with 5s polling
- **SSE** — real-time host status via EventSource
- **i18n** — English (default) + Korean, client-side locale switching

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `NEXT_PUBLIC_API_URL` | Backend API base URL | `http://localhost:3000` |
| `NEXT_PUBLIC_WEB_API_KEY` | API key for backend auth | — |

## Project Structure

```
app/
├── page.tsx                 # Overview dashboard
├── host/page.tsx            # Host detail shell (`/host/?key=<host_key>`)
├── host/HostPageClient.tsx  # Client-side host detail resolver
├── agents/page.tsx          # Agent management
├── alerts/page.tsx          # Alert configuration
├── components/              # Shared UI components
├── i18n/                    # Internationalization
├── lib/                     # API client, SSE context
└── types/                   # TypeScript types
```

Canonical host detail links use `/host?key=<host_key>`. The page is a static shell and reads the active host from `useSearchParams()` at runtime.

See the [root README](../README.md) for full project documentation.
