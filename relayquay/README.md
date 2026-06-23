# RelayQuay

RelayQuay is a small deployment kit for exposing a self-hosted RustDesk server
to the RustDesk Web client without publishing the raw RustDesk server ports to
the Internet.

The default design is intentionally narrow:

```text
rustdesk.com/web
  -> Cloudflare Access protected hostname
  -> Cloudflare Tunnel
  -> loopback Caddy WebSocket gateway
  -> hbbs :21118 and hbbr :21119
```

Cloudflare Access performs the human authentication. Caddy only listens on
loopback. RustDesk's normal TCP/UDP ports remain private to localhost, LAN,
Tailscale, WireGuard, or another explicitly allowed private network.

## Why This Exists

RustDesk Server OSS exposes several ports for rendezvous, relay, NAT checks,
and WebSocket support. The browser client needs WebSocket Secure endpoints, but
it does not need the raw RustDesk TCP/UDP ports to be open to the whole
Internet.

RelayQuay gives you an open-source, repeatable way to publish only:

- `https://rustdesk-wss.example.com/ws/id`
- `https://rustdesk-wss.example.com/ws/relay`
- `https://rustdesk-wss.example.com/healthz`

Everything sensitive stays outside the repo in `/var/lib/plugroot/relayquay`.

## Quick Start

Create private state and an env file:

```bash
sudo install -d -m 700 /var/lib/plugroot/relayquay
install -m 600 .env.example /var/lib/plugroot/relayquay/relayquay.env
```

Edit `/var/lib/plugroot/relayquay/relayquay.env` and set:

- `RELAYQUAY_PUBLIC_HOST`
- `CLOUDFLARE_TUNNEL_TOKEN`
- `RUSTDESK_DATA_DIR` if your current RustDesk data lives somewhere else

Start only the WSS gateway:

```bash
./bin/relayquay doctor
./bin/relayquay up
./bin/relayquay logs
```

If you want RelayQuay to run its own RustDesk OSS containers too:

```bash
COMPOSE_PROFILES=server ./bin/relayquay up
```

## Cloudflare Setup

1. Create a Cloudflare Tunnel for the public hostname.
2. Route the hostname to `http://127.0.0.1:8788`.
3. Create a Cloudflare Access self-hosted application for the same hostname.
4. Add an allow policy for only your email address or identity group.
5. Enable an MFA-capable identity provider, or Cloudflare One-time PIN for a
   low-friction personal setup.
6. Open `https://rustdesk-wss.example.com/healthz` in the browser first so
   Cloudflare Access can set its session cookie.
7. Open `https://rustdesk.com/web`, set the self-hosted ID server to your
   public hostname, enter your RustDesk server key, and connect.

See [docs/cloudflare-access.md](docs/cloudflare-access.md) and
[docs/rustdesk-web.md](docs/rustdesk-web.md) for the details and caveats.

## Security Defaults

- No Cloudflare token is committed.
- No real hostname is committed.
- The gateway binds to `127.0.0.1:8788`.
- The RustDesk server profile uses Docker host networking only when you opt in.
- Firewall examples deny public access to `21115-21119` and allow private
  networks deliberately.
- The helper script refuses to print Cloudflare secrets.

## Browser Relay Note

If an existing `hbbs` advertises a relay address that only works on a private
network, the RustDesk Web session may signal successfully and then time out
during relay setup. For browser use, the relay address visible to the web client
must also be reachable through the WSS hostname, or the web client must be
configured to use that hostname for both ID and relay where the UI supports it.

## Plugroot

RelayQuay is code-only inside `/opt/plugroot/relayquay`. Register it in
Plugroot with a private local overlay based on
[examples/plugroot/plugroot.local.example.toml](examples/plugroot/plugroot.local.example.toml).

Do not put real tunnel tokens, private hostnames, or local-only notes in this
checkout.
