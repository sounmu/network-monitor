# netsentinel-web

Next.js frontend dashboard for **NetSentinel**. Displays real-time CPU, memory, load, port, and Docker container metrics with interactive time-series charts.

## Quick Start

```bash
cp .env.example .env   # configure API URL
npm install
npm run dev             # http://localhost:3001
```

## Tech Stack

- **Next.js 16** (App Router, standalone output)
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
├── host/[host_key]/page.tsx # Host detail view
├── agents/page.tsx          # Agent management
├── alerts/page.tsx          # Alert configuration
├── components/              # Shared UI components
├── i18n/                    # Internationalization
├── lib/                     # API client, SSE context
└── types/                   # TypeScript types
```

See the [root README](../README.md) for full project documentation.
