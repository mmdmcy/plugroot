# Security Model

Plugroot is private-first.

## Defaults

- The web dashboard binds to `127.0.0.1:8786` unless configured otherwise.
- Set `PLUGROOT_WEB_PASSWORD` to require HTTP Basic auth for the web dashboard.
- Example services bind to `PLUGROOT_PRIVATE_IP`, which should be localhost or
  a private-network address.
- Actions are allowlisted per service.
- Plugroot does not run arbitrary commands from the dashboard or TUI.
- The code root is a Git checkout and should contain reusable code only.
- The state root is local-only private machine state and defaults to
  `/var/lib/plugroot`.
- Real `.env` files, local overlays, generated units, cloned app repos, logs,
  databases, backups, and service data belong in the state root.
- `plugroot audit-public` checks tracked files for ignored private paths,
  common secret markers, private key material, Tailscale-style private IPs,
  and optional local denylist terms.
- `plugroot audit-public --install-hook` installs local pre-commit and
  pre-push hooks so the audit and boundary check run before commits and pushes.
- `plugroot boundary --strict` checks that private runtime paths are not inside
  the code checkout and that the state root is not inside a Git repo.
- App-specific service definitions, real domains, service data, backup
  archives, and tokens belong in ignored local files only.

## Filesystem Boundary

Use a code-only checkout and a separate private state root:

```text
GitHub workspace checkout
  -> edit code
  -> commit and push

selfhost checkout
  -> git pull
  -> run Plugroot

private state root
  -> .env
  -> plugroot.local.toml
  -> repos/
  -> services/*/data/
  -> backups/
```

The selfhost checkout is still code-only. Pulling updates there should never
move private files into Git scope. If a file is specific to one host, it belongs
under the state root, not beside `plugroot.toml`.

## Network Boundary

Plugroot should be reachable only from a private network or local shell:

```text
trusted client
  -> private VPN or localhost
  -> Plugroot services
```

Do not bind private services to a public IP unless you intentionally design and
document that exposure first.

For high-value services, prefer localhost-bound backends behind private-network
HTTPS. Do not enable public tunnels unless that exposure is explicitly designed
and documented.

## Local Denylist

For host-specific terms that should never appear in the public repo, create an
ignored file under the private state root:

```text
$PLUGROOT_STATE_ROOT/.plugroot/audit-denylist.txt
```

Use one literal term per line. Lines starting with `#` are ignored. Legacy
code-root denylist paths are still read for old installs, but the boundary
check flags `.plugroot/` inside the checkout.
