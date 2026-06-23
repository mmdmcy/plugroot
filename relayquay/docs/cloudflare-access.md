# Cloudflare Access

RelayQuay assumes Cloudflare is only the authenticated front door for the WSS
gateway. RustDesk traffic still goes to your self-hosted `hbbs` and `hbbr`
processes.

## Tunnel

Create a Cloudflare Tunnel and route your public hostname to the local Caddy
gateway:

```text
Hostname: rustdesk-wss.example.com
Service:  http://127.0.0.1:8788
```

The compose file uses a tunnel token through `CLOUDFLARE_TUNNEL_TOKEN`. Keep
that token only in `/var/lib/plugroot/relayquay/relayquay.env`.

## Access Application

Create a Cloudflare Access self-hosted application for the same hostname:

```text
Application domain: rustdesk-wss.example.com
Policy action:      Allow
Include:            your email address or your identity group
Session duration:   short enough for your risk tolerance
```

For personal use, Cloudflare One-time PIN is low-friction. A real identity
provider with MFA is stronger.

## Browser Flow

Cloudflare Access protects the WebSocket handshake. Before using
`https://rustdesk.com/web`, visit:

```text
https://rustdesk-wss.example.com/healthz
```

After Access authentication succeeds, the browser has the Access session cookie
for your hostname. Then open RustDesk Web and configure your self-hosted server.

## CORS and Cookies

If the web client cannot connect after Access login, check the Access
application's additional settings:

- The hostname must match exactly.
- The Access session cookie must be available to the browser profile using
  RustDesk Web.
- If you enable CORS controls in Cloudflare Access, allow
  `https://rustdesk.com` as an origin.

## Why Not Cloudflare Arbitrary TCP

Cloudflare Access can proxy arbitrary TCP, but that mode requires `cloudflared`
on the client device. That does not fit a browser-only `rustdesk.com/web` flow.
RelayQuay uses normal HTTPS and WebSocket support so the client side can remain
just a browser.
