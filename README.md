# Plugroot

Plugroot is a private-first selfhost harness for people who want one small
code-only repo to describe and operate their local server.

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
cargo run -- status
cargo run -- doctor
cargo run -- boundary --strict
cargo run -- apply --dry-run
cargo run -- tui
```

On an installed host, Plugroot can install a `plugroot` launcher:

```bash
sudo /opt/plugroot/bin/plugroot --root /opt/plugroot apply
plugroot doctor
plugroot tui
```

For a real host, keep private values outside the checkout:

```bash
sudo install -d -o "$USER" -g "$USER" -m 700 /var/lib/plugroot
cp .env.example /var/lib/plugroot/.env
cp plugroot.local.example.toml /var/lib/plugroot/plugroot.local.toml
```

Build a release binary:

```bash
cargo build --release
./target/release/plugroot status
```

## Manifest

Plugroot reads:

```text
plugroot.toml                         public/default manifest in the code root
$PLUGROOT_STATE_ROOT/.env             private values
$PLUGROOT_STATE_ROOT/plugroot.local.toml  private local overlay
```

The public manifest can define reusable services and repos. The private overlay
can replace or add entries with the same `id` for one machine.

## Commands

```text
plugroot status [--json]
plugroot list
plugroot apply [--dry-run]
plugroot repos sync
plugroot doctor [--json] [--strict]
plugroot release-check
plugroot up|down|restart|logs <service|all>
plugroot tui [--once]
plugroot web [--bind <addr:port>]
plugroot boundary [--strict]
plugroot audit-public [--install-hook]
```

## Included Examples

- Neutral Compose stack bound to `PLUGROOT_PRIVATE_IP`.
- Git checkout and systemd examples in `docs/manifest.md`.
- Operator command conventions and health checks in `docs/operations.md`.
- Plugroot Web as a local private dashboard.
- Optional Plugroot Web Basic auth through `PLUGROOT_WEB_USER` and
  `PLUGROOT_WEB_PASSWORD`.

## Safety

Use two roots:

```text
code root   Git checkout, safe to push, no private machine state
state root  local-only runtime state, no Git repo, default /var/lib/plugroot
```

Real secrets and private state are ignored by default:

```text
.env
plugroot.local.toml
*.local.toml
*.local.json
.plugroot/
repos/
services/*/data/
services/*/config/
services/*/secrets/
media/
backups/
*.bitwarden-export*
*.proton-pass-export*
*.kdbx
*.pem
*.key
*.db
*.log
```

Before publishing changes:

```bash
cargo fmt --check
cargo test
plugroot audit-public
plugroot boundary --strict
plugroot release-check
gitleaks detect --source . --redact
```

For local protection without CI, install the pre-commit and pre-push hooks:

```bash
cargo run -- audit-public --install-hook
```

For host-specific names, domains, or literal terms that should never appear in
the public repo, add one term per line to an ignored denylist:

```text
$PLUGROOT_STATE_ROOT/.plugroot/audit-denylist.txt
```
