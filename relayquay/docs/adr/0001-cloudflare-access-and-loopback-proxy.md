# 0001: Cloudflare Access and Loopback Proxy

## Status

Accepted.

## Context

RustOp Viewer is designed to run locally and expose a browser UI from the host.
For off-device browser access, the host needs a private authenticated entry
point without opening the ROV origin directly to the Internet.

## Decision

RelayQuay uses Cloudflare Access plus Cloudflare Tunnel as the public entry
point. The tunnel targets Caddy on `127.0.0.1:8788`; Caddy proxies to
`ROV_UPSTREAM`, normally `127.0.0.1:45080`.

## Consequences

- No inbound public firewall rule is required for the origin.
- Access authentication happens before the browser reaches the host.
- ROV pairing/session authorization remains required.
- Cloudflare can still observe traffic metadata for the protected hostname.
