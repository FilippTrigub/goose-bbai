# goose-bbai

Goose, adapted into an API backend for [space-goose](https://github.com/FilippTrigub/space-goose).

This repository is based on [aaif-goose/goose](https://github.com/aaif-goose/goose), but it is no longer just a local agent UI/CLI. The main change is that Goose now exposes a REST API so another app can drive sessions, messages, extensions, and settings remotely.

## What changed from upstream Goose

- Added `goose api-server` to run an Axum-based REST API.
- Added session endpoints for creating, listing, exporting, deleting, and messaging conversations.
- Added both agent-event streaming and provider-level token streaming.
- Added MongoDB-backed persistence for sessions and session history.
- Added endpoints for extension management, settings management, health checks, and agent status.
- Added CORS defaults so a browser frontend can talk to the API.

## How it fits with space-goose

`space-goose` is the companion application that talks to this server.
Instead of using Goose only as a local interactive agent, `space-goose` can treat it as a backend API and orchestrate conversations from another UI or workflow.

## API server

Start the server with:

```bash
goose api-server --port 3000 --host 127.0.0.1 --database-url mongodb://localhost:27017
```

Notes:

- `--database-url` is required, or you can set `MONGODB_URL`.
- The database name comes from `MONGODB_DATABASE` and defaults to `goose_sessions`.
- There is no local-storage fallback; this API server requires MongoDB.
- Goose must already be configured/authenticated before the server starts.

## Available API areas

- `GET /api/v1/health`
- `GET /api/v1/agent/status`
- `POST /api/v1/sessions`
- `GET /api/v1/sessions`
- `GET /api/v1/sessions/:id`
- `POST /api/v1/sessions/:id/messages`
- `POST /api/v1/sessions/:id/send`
- `POST /api/v1/sessions/:id/stream`
- `GET /api/v1/sessions/:id/export`
- `GET /api/v1/extensions`
- `GET /api/v1/settings`

## Original project

Upstream Goose: [block/goose](https://github.com/block/goose)
