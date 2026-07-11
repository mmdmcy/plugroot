# RelayQuay

RelayQuay is a small deployment kit for publishing a local RustOp
Viewer-compatible host runtime through a Cloudflare Access protected hostname.

The default design is intentionally narrow:

```text
browser
  -> Cloudflare Access protected hostname
  -> Cloudflare Tunnel
  -> loopback Caddy gateway
  -> RustOp Viewer on 127.0.0.1:45080
```

Cloudflare Access performs identity authentication before traffic reaches the
host. RustOp Viewer still handles its own host-approved pairing, short-lived
browser sessions, and remembered-browser trust.

## Quick Start

Create private state and an env file:

```bash
./bin/relayquay init
```

Edit `/var/lib/relayquay/relayquay.env` and set:

- `RELAYQUAY_PUBLIC_HOST`
- `CLOUDFLARE_TUNNEL_TOKEN`
- `ROV_UPSTREAM` if RustOp Viewer is not on `127.0.0.1:45080`

Start RustOp Viewer on the host:

```bash
cd /path/to/rustopviewer
cargo run --release -- --headless
```

If your viewer checkout lives elsewhere, run the equivalent binary from that
location. RelayQuay only needs the viewer to listen at `ROV_UPSTREAM`.

Then start RelayQuay:

```bash
./bin/relayquay doctor
./bin/relayquay up
./bin/relayquay logs
```

## Cloudflare Setup

1. Create a Cloudflare Tunnel for the public hostname.
2. Route the hostname to `http://127.0.0.1:8788`.
3. Create a Cloudflare Access self-hosted application for the same hostname.
4. Add an allow policy for only your identity or identity group.
5. Use an MFA-capable identity provider, or Cloudflare One-time PIN for a
   low-friction personal setup.

After Cloudflare Access login, the browser reaches RustOp Viewer through
RelayQuay. A new browser still needs a host-approved ROV pairing code before it
can view or control the desktop.

## Security Defaults

- No Cloudflare token is committed.
- No real hostname is committed.
- The gateway binds to `127.0.0.1:8788`.
- Caddy proxies one same-origin upstream: `ROV_UPSTREAM`.
- RustOp Viewer should stay loopback-only unless you explicitly add a trusted
  local listener.
- RelayQuay does not store ROV pairing codes or browser session tokens.
- The helper script avoids printing Cloudflare secrets.

## Network Usage

RelayQuay is quiet while idle. The main bandwidth cost is an active ROV browser
session. Use ROV's Data Saver or Emergency stream profiles on metered links,
and stop `cloudflared` when remote browser access is not needed:

```bash
./bin/relayquay stop cloudflared
./bin/relayquay up cloudflared
```

See [docs/network-usage.md](docs/network-usage.md).

## Plugroot

RelayQuay is standalone. If you also use a host orchestrator such as Plugroot,
keep this repository as a code-only checkout and register it with private local
configuration based on
[examples/plugroot/plugroot.local.example.toml](examples/plugroot/plugroot.local.example.toml).

Do not put real tunnel tokens, private hostnames, generated state, screenshots,
or local-only notes in this checkout. Put those in the private env file selected
by `RELAYQUAY_ENV_FILE`.

## Project Docs

- [Architecture](docs/architecture.md)
- [Cloudflare Access](docs/cloudflare-access.md)
- [Firewall](docs/firewall.md)
- [Infrastructure](docs/infrastructure.md)
- [Network Usage](docs/network-usage.md)
- [Operations](docs/operations.md)
- [Repository Layout](docs/repository-layout.md)
- [Threat Model](docs/threat-model.md)
