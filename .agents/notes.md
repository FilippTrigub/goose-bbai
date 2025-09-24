# Notes: Stable Cloudflare Tunnel (Low/No Cost)

Goal: Expose the Goose CLI on a stable HTTPS hostname to support OAuth redirect URLs (e.g., GitHub) without relying on ephemeral tunnels.

Key idea: Use a Cloudflare Tunnel with a named tunnel + DNS record on a domain you control. Cloudflare’s Free plan allows stable hostnames; you only pay for the domain (if you don’t already have one).

Steps
- Prereqs
  - Cloudflare account (Free plan).
  - A domain added to Cloudflare (change nameservers to Cloudflare). Domain cost is the only expense.
  - Install `cloudflared` on the machine running the CLI.
- Login and create a named tunnel
  - `cloudflared login` (authorize in browser)
  - `cloudflared tunnel create goose-cli`
- Map a stable DNS name to the tunnel
  - `cloudflared tunnel route dns goose-cli cli.yourdomain.com`
  - This creates/updates a CNAME in Cloudflare DNS that points to the tunnel. The hostname is now stable.
- Configure ingress to your local service
  - Create `~/.cloudflared/config.yml`:
    ```yaml
    tunnel: goose-cli
    credentials-file: /home/USER/.cloudflared/<UUID>.json
    ingress:
      - hostname: cli.yourdomain.com
        service: http://localhost:8080   # your CLI web port
      - service: http_status:404
    ```
  - Start: `cloudflared tunnel run goose-cli` (or install as a service: `cloudflared service install`).

OAuth With GitHub
- In your GitHub OAuth App, set the callback to `https://cli.yourdomain.com/oauth_callback` (exact match required; no wildcards).
- In Goose, set `GOOSE_AUTH_REDIRECT_URL=https://cli.yourdomain.com/oauth_callback`.
- Keep PKCE/state checks enabled.

Costs & Limits
- Cloudflare Tunnel is free on the Free plan. No per-GB or hourly charges.
- You need a domain (cost varies). No need for Cloudflare Access (Zero Trust) unless you want additional gatekeeping.

Pitfalls
- Do not use Quick Tunnels (`trycloudflare.com`) for OAuth—they’re ephemeral.
- Ensure the tunnel host is reachable on port 443 (Cloudflare handles TLS).
- If you enable Cloudflare Access, exempt the callback path so GitHub can reach it.

Alternatives (if you can’t bring a domain)
- Host a stable callback on Cloudflare Workers and relay to the CLI (more complex), or prefer GitHub Device Flow to avoid callbacks entirely.

