# Multi‑User OAuth for Hosted CLI (Minimal Plan)

## Context & Goal
- We will deploy the Goose CLI behind a stable Cloudflare URL and allow many users to access the same URL.
- Today, auth is process‑wide: a single GitHub token (keyring or file) gates all users.
- Goal: enable per‑user authentication where each browser session authenticates with GitHub. The browser should hold the session credential; the server should not share one user’s creds with others.

## Current State (Source Review)
- CLI web server (`crates/goose-cli/src/commands/web.rs`): serves a simple Axum app + WebSocket; no auth or sessions.
- CLI auth (`crates/goose-cli/src/commands/auth.rs`): Authorization Code + PKCE (S256) flow for the whole process, storing token via `Config` (keyring/file). Not per‑browser.
- Server (`crates/goose-server`): protects routes via `X-Secret-Key` middleware; no OAuth/OIDC for per‑user.
- OAuth helpers exist for provider setup in `crates/goose/src/oauth/*`, not for per‑user web sessions.

## Minimal Architecture (Per‑Browser Session)
- Use OAuth2 Authorization Code + PKCE via GitHub in the browser.
- Store only a short‑lived, signed session cookie in the browser (HttpOnly, SameSite=Lax). Keep the GitHub access token server‑side (in memory) keyed by session id → satisfies “creds in browser” by storing the session credential (not the provider token) in the browser.
- Gate all CLI web endpoints (HTTP + WS) by validating the session cookie → map to user context.
- Remove process‑wide token persistence: Do not write provider tokens to keyring or secrets.yaml in cloud mode. Use in‑memory session storage only.

## HTTP Endpoints (Axum in `web.rs`)
- `GET /auth/login` → generates `state` + PKCE challenge, stores `state` in a temporary cookie, redirects to GitHub authorize URL with `redirect_uri=<host>/auth/callback`.
- `GET /auth/callback` → validates `state`, exchanges `code` for token, fetches `GET https://api.github.com/user` (login,id). Creates a server‑side session `{session_id -> {user_id, token, expiry}}`, sets `Set-Cookie: goose_session=<id>; HttpOnly; SameSite=Lax; Secure`, redirects to `/`.
- `POST /auth/logout` → clears `goose_session` cookie and removes the session.
- `GET /api/me` → returns `{ user: {login, id}, authenticated: true }` for UI use.

## Request Gating
- HTTP routes: Axum middleware that reads `goose_session` cookie, validates session in memory, or 401.
- WebSocket (Required): Authenticate the upgrade request by validating the `goose_session` cookie before establishing the socket; reject unauthenticated upgrades. Also perform an `Origin` check when applicable.

## Storage Strategy
- Server‑side: in‑memory `Arc<RwLock<HashMap<SessionId, SessionData>>>`; optional TTL cleanup task.
- Browser: HttpOnly cookie `goose_session`. This meets “store creds in the browser” while keeping tokens server‑side.
- Persistence across restarts (optional): write session map to a temp file with expirations; not needed for minimal.

## Security
- PKCE S256 + CSRF state (in a non‑HttpOnly cookie or encrypted temp store tied to IP/UA if desired).
- HttpOnly + Secure cookies; SameSite=Lax; set `Secure` behind HTTPS (your deployment must terminate TLS so cookies are sent securely).
- Token never sent to the browser. Limit scopes to `read:user user:email`.

## Code Changes
- `crates/goose-cli/src/commands/web.rs`:
  - Add new routes: `/auth/login`, `/auth/callback`, `/auth/logout`, `/api/me`.
  - Add a `SessionStore` (in memory) and attach to `AppState`.
  - Add middleware to enforce session on `/`, `/ws`, `/api/*` (except `/auth/*`, `/static/*`, `/api/health`).
  - Modify WebSocket handler to verify session via cookies.
- `crates/goose-cli/src/commands/auth.rs`:
  - No longer used for per‑browser login; keep as a local/dev fallback only. In cloud mode, disable/panic if token persistence is attempted.

## Config & Env
- `GOOSE_GITHUB_CLIENT_ID`, `GOOSE_GITHUB_CLIENT_SECRET` (GitHub requires secret at token endpoint).
- `GOOSE_AUTH_REDIRECT_URL=https://<host>/auth/callback` (stable tunnel hostname).
- Optional: `GOOSE_SESSION_SECRET` (HMAC key for signing session ids); otherwise generate a random per‑process key.
- Cloud mode flag: `GOOSE_AUTH_IN_MEMORY_ONLY=1` to strictly avoid persistence and use in‑memory sessions only.

## Minimal UX
- Unauthenticated visitors to `/` get redirected to `/auth/login`.
- After login, cookie is set and the chat UI loads.
- Logout clears cookie.

## Dependencies (minimal)
- Use existing `reqwest` in workspace (via goose providers) or add it to goose-cli for token exchange.
- Add `axum-extra` (for typed cookies) or manual cookie parsing with `headers` crate.

## Rollout Steps
1) Add session store + cookie middleware to web server.
2) Implement `/auth/login` and `/auth/callback` with PKCE S256 and GitHub token exchange.
3) Gate routes and WebSocket by cookie session.
4) Test locally and via Cloudflare with stable callback URL.

## Future Enhancements
- Refresh token handling (if enabled in GitHub OAuth App).
- User-level quotas/limits; per-user audit.
- Persistent session store; revocation endpoint.

## Critical Questions & Suggested Answers

- Session Store Scope and Scaling
  - Question: Will we run a single CLI instance behind one Cloudflare Tunnel, or multiple replicas?
  - Suggested: Start with a single instance (in-memory `HashMap` for sessions). For HA/scaling later, switch to a shared store (Redis) and enable sticky sessions.
  - Answer: there will be a single instance for now. 

- Cookie Signing/Secrets
  - Question: How do we sign/validate session cookies?
  - Suggested: Use an HMAC secret from `GOOSE_SESSION_SECRET` (generated if missing). Store only opaque `session_id` in the cookie; keep tokens server-side.
  - Answer: sounds good
  
- Callback URL and Host
  - Question: How do we form `redirect_uri` reliably behind Cloudflare?
  - Suggested: Use `GOOSE_AUTH_REDIRECT_URL` (e.g., `https://cli.example.com/auth/callback`). Do not derive from `Host` header unless validated; GitHub requires exact match.
  - Answer: The app will be deployed at a static URL. This will permit an exact match. Forget about cloudflare ephemeral tunnels for now.

- GitHub OAuth Requirements
  - Question: Do we need a client secret with PKCE?
  - Suggested: Yes, for GitHub OAuth Apps at the token endpoint. Provide `GOOSE_GITHUB_CLIENT_ID` and `GOOSE_GITHUB_CLIENT_SECRET`. Keep PKCE S256.
  - Answer: the secret is already available as env var

- State + PKCE Verifier Storage
  - Question: Where to keep `state` and `code_verifier` between `/auth/login` and `/auth/callback`?
  - Suggested: Keep a short-lived in-memory map `{state -> verifier, issued_at}` and also set a non-HttpOnly cookie with the `state` to cross-check. TTL ~5 minutes.
  - Answer: sounds good

- WebSocket Auth
  - Question: How to authenticate WS upgrades?
  - Suggested: Read `Cookie` header on upgrade, extract `goose_session`, validate against session store; reject if invalid.

- Cookie Attributes
  - Question: What flags to set on the session cookie?
  - Suggested: `HttpOnly; SameSite=Lax; Secure`. Set `Domain` to the tunnel host if needed. Cloudflare provides TLS so `Secure` is valid.
  - Answer: This sounds ok. We will not require cloudflare though. The app will be deployed at a static url.

- Token Lifetime and Refresh
  - Question: Will tokens expire? Do we need refresh?
  - Suggested: GitHub tokens are non-expiring by default; if app enables expiry, we can store `expires_in` and re-login on expiry (refresh optional in v1).
  - Answer: sounds good

- Logout Semantics
  - Question: Should we revoke provider tokens on logout?
  - Suggested: Minimal v1: delete session and cookie only. No token revocation.
  - Answer: sounds good

- UI Behavior
  - Question: How does the client react when unauthenticated?
  - Suggested: Redirect unauthenticated `GET /` to `/auth/login`. For API/WS 401s, the page should redirect to `/auth/login` or show a simple link.
  - Answer: sounds good

- Data Separation
  - Question: Could per-user tokens leak into another user’s session?
  - Suggested: Never store tokens globally. Tie tokens strictly to `session_id` in the session store and access only via validated cookie.
  - Answer: sounds good
