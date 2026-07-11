# Architecture

RelayQuay publishes a local RustOp Viewer runtime through a single
Access-protected HTTPS hostname.

```text
browser
  -> Cloudflare Access
  -> Cloudflare Tunnel
  -> loopback Caddy gateway
  -> RustOp Viewer loopback server
```

## Components

- `cloudflared` creates an outbound-only tunnel to Cloudflare.
- Cloudflare Access authenticates users before requests reach the tunnel.
- Caddy listens on loopback and reverse-proxies all non-health requests to
  `ROV_UPSTREAM`.
- RustOp Viewer serves the browser UI and owns pairing, sessions, frame
  capture, and remote input authorization.
- Deployment files live under `deploy/` so runtime configuration is separated
  from docs, examples, and infrastructure templates.

## Runtime State

Private state belongs outside the repository, normally under
`/var/lib/relayquay`.

That includes tunnel tokens, real hostnames, logs, generated files, screenshots,
and host-specific operational notes.

## Decisions

- [0001: Cloudflare Access and Loopback Proxy](adr/0001-cloudflare-access-and-loopback-proxy.md)
