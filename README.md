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
cargo run -- repos sync
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
sudo install -d -o "$USER" -g "$USER" -m 700 /opt/plugroot-state
cp .env.example /opt/plugroot-state/.env
cp plugroot.local.example.toml /opt/plugroot-state/plugroot.local.toml
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

Application source stays in standalone repositories under `repo_dir`; it is not
vendored into Plugroot. When an application has its own mode-aware lifecycle
helper, Plugroot can sync its checkout and monitor its port while that helper
remains responsible for start, stop, and mode transitions.

## Commands

```text
plugroot status [--json]
plugroot list
plugroot apply [--dry-run] [--sync-repos]
plugroot repos sync
plugroot doctor [--json] [--strict]
plugroot release-check
plugroot backup [check] [directory]
plugroot restore [check|finish] [directory-or-archive]
plugroot emergency export --output <directory> --yes
plugroot up|down|restart|logs <service|all>
plugroot tui [--once]
plugroot web [--bind <addr:port>]...
plugroot boundary [--strict]
plugroot audit-public [--install-hook]
```

## Included Examples

- Neutral Compose stack bound to `PLUGROOT_PRIVATE_IP`.
- Git checkout and systemd examples in `docs/manifest.md`.
- Standalone helper-managed application pattern, including RelayQuay, in
  `docs/manifest.md`.
- Operator command conventions and health checks in `docs/operations.md`.
- Manual encrypted emergency exports in `docs/emergency.md`.
- Fresh-install and standby-server recovery in `docs/restore.md`.
- Plugroot Web as a local private dashboard.
- Optional Plugroot Web Basic auth through `PLUGROOT_WEB_USER` and
  `PLUGROOT_WEB_PASSWORD`.
- Minimal unauthenticated component health at
  `/.well-known/linuxmice/component`; it exposes no service inventory, state
  paths, repository details, or credentials.
- `plugroot web` accepts repeated `--bind` flags, and `PLUGROOT_WEB_BIND` may
  list comma-separated addresses. Keep `127.0.0.1:8786` in that list on a host
  whose dashboard binds a private carrier address, so the loopback endpoint
  declared in `linuxmice-component.toml` stays reachable for LinuxMice.

`linuxmice-component.toml` is the canonical optional catalog declaration for
that status surface. Plugroot does not require LinuxMice to run and LinuxMice
does not read Plugroot's private manifest or runtime state.

## Safety

Use two roots:

```text
code root   Git checkout, safe to push, no private machine state
state root  local-only runtime state, no Git repo, default sibling of the code root
```

Private application files can stay under `state-root/workspaces/` while managed
symlinks expose them at the paths applications expect. This preserves one
backup boundary without mixing secrets or mutable state into public Git code.

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
cargo katrust check
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

Plain identifier terms match only at identifier boundaries, so a short name
does not accidentally match a longer ordinary word. Terms containing dots,
slashes, or other punctuation remain literal substring checks.
