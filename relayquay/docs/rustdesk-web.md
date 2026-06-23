# RustDesk Web Notes

RustDesk's browser client uses WebSocket Secure support on the self-hosted
server:

- `hbbs` WebSocket endpoint on `21118`
- `hbbr` WebSocket endpoint on `21119`
- reverse proxy paths `/ws/id` and `/ws/relay`

RelayQuay maps those paths through Caddy and Cloudflare Tunnel.

## Existing RustDesk Server

If RustDesk is already running on this machine, leave the default upstreams:

```text
RUSTDESK_HBBS_WS_UPSTREAM=127.0.0.1:21118
RUSTDESK_HBBR_WS_UPSTREAM=127.0.0.1:21119
```

Then run:

```bash
./bin/relayquay doctor
./bin/relayquay up
```

## RustDesk Key

The web client needs your server key. RelayQuay can print it without exposing
private key material:

```bash
./bin/relayquay key
```

Only the public key is printed.

## Relay Address

The web client must be able to reach both signaling and relay. A common failure
mode is:

1. `hbbs` is reachable through the public WSS hostname.
2. `hbbs` advertises an `hbbr` relay address that only works on Tailscale or LAN.
3. The web client times out because the relay leg is unreachable.

For browser access, use one of these approaches:

- Configure RustDesk Web to use the public WSS hostname for both ID and relay if
  the UI exposes both fields.
- Configure the server's relay address so browser clients receive a reachable
  public hostname.
- Run a separate RustDesk server profile for browser-only access.

The right choice depends on whether your native RustDesk clients should keep
using private Tailscale/LAN routing or move to WSS as well.

## Optional Server Profile

RelayQuay can run RustDesk Server OSS itself:

```bash
COMPOSE_PROFILES=server ./bin/relayquay up
```

This uses Docker host networking, matching RustDesk's Docker recommendation.
Only enable it when you have a firewall policy ready for the raw RustDesk ports.
