# Security Model

Plugroot is private-first.

## Defaults

- The web dashboard binds to `127.0.0.1:8786` unless configured otherwise.
- Set `PLUGROOT_WEB_PASSWORD` to require HTTP Basic auth or a login for the web
  dashboard. `plugroot web` refuses to start without it when any bind address
  is not loopback. Login cookies are HMAC-signed with a random secret stored
  in `web-session.key` (mode 0600) under the state root, never with the
  password; the password itself is still read from the environment as plain
  text. Request bodies above 64 KiB are rejected before authentication.
- Example services bind to `PLUGROOT_PRIVATE_IP`, which should be localhost or
  a private-network address.
- Actions are allowlisted per service.
- Plugroot does not run arbitrary commands from the dashboard or TUI.
- The code root is a Git checkout and should contain reusable code only.
- The state root is local-only private machine state and defaults to a sibling
  `../plugroot-state` directory unless configured.
- Real `.env` files, local overlays, generated units, cloned app repos, logs,
  databases, backups, service data, and private workspace files belong in the
  state root.
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
  -> workspaces/*/
  -> backups/
```

The selfhost checkout is still code-only. Pulling updates there should never
move private files into Git scope. If a file is specific to one host, it belongs
under the state root, not beside `plugroot.toml`.

Applications that require private files inside their checkout can use managed
symlinks from `workspaces/` under the state root. Plugroot requires the source
to remain inside the state root and requires Git targets to be ignored before
creating a link.

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
