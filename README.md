# Plugroot

Plugroot is a private-first selfhost harness for people who want one small
repo to describe and operate their local server.

It is not a PaaS. It is a manifest-driven control center for:

- Docker Compose stacks
- Git repository checkouts
- systemd and user systemd services
- private ports/manual listeners
- a terminal dashboard
- a tiny built-in web dashboard

The intended access boundary is localhost, Tailscale, WireGuard, or another
private network. Do not expose private control surfaces to the public internet.

## Quick Start

```bash
cp .env.example .env
cargo run -- status
cargo run -- apply --dry-run
cargo run -- tui
```

Build a release binary:

```bash
cargo build --release
./target/release/plugroot status
```

## Manifest

Plugroot reads:

```text
plugroot.toml        public/default manifest
plugroot.local.toml  ignored local overlay
.env                ignored local values
```

The public manifest can define services and repos. The ignored overlay can
replace or add entries with the same `id` for one machine.

## Commands

```text
plugroot status [--json]
plugroot list
plugroot apply [--dry-run]
plugroot repos sync
plugroot up|down|restart|logs <service|all>
plugroot tui [--once]
plugroot web [--bind <addr:port>]
```

## Included Examples

- Neutral Compose stack bound to `PLUGROOT_PRIVATE_IP`.
- Git checkout and systemd examples in `docs/manifest.md`.
- Plugroot Web as a local private dashboard.
- Optional Plugroot Web Basic auth through `PLUGROOT_WEB_USER` and
  `PLUGROOT_WEB_PASSWORD`.

## Safety

Real secrets and private state are ignored by default:

```text
.env
plugroot.local.toml
repos/
services/*/data/
services/*/config/
media/
backups/
*.db
*.log
```

Before publishing changes:

```bash
cargo fmt --check
cargo test
plugroot audit-public
gitleaks detect --source . --redact
```

For local protection without CI, install the pre-push hook:

```bash
cargo run -- audit-public --install-hook
```
