# Security Model

Plugroot is private-first.

## Defaults

- The web dashboard binds to `127.0.0.1:8786` unless configured otherwise.
- Set `PLUGROOT_WEB_PASSWORD` to require HTTP Basic auth for the web dashboard.
- Example services bind to `PLUGROOT_PRIVATE_IP`, which should be localhost or
  a private-network address.
- Actions are allowlisted per service.
- Plugroot does not run arbitrary commands from the dashboard or TUI.
- `.env`, local overlays, repos, logs, databases, and service data are ignored.
- `plugroot audit-public` checks tracked files for ignored private paths,
  common secret markers, private key material, Tailscale-style private IPs,
  and optional local denylist terms.
- `plugroot audit-public --install-hook` installs local pre-commit and
  pre-push hooks so the audit runs before commits and pushes.
- Password-manager services are high-value targets. Keep their databases,
  exports, backup archives, tokens, and real private URLs in ignored local
  files only.

## Network Boundary

Plugroot should be reachable only from a private network or local shell:

```text
trusted client
  -> private VPN or localhost
  -> Plugroot services
```

Do not bind private services to a public IP unless you intentionally design and
document that exposure first.

For Vaultwarden-style password-manager services, prefer a localhost-bound
backend behind private-network HTTPS. Do not enable public tunnels for password
managers by default.

## Local Denylist

For host-specific terms that should never appear in the public repo, create an
ignored file at one of:

```text
docs/private/audit-denylist.txt
.plugroot/audit-denylist.txt
```

Use one literal term per line. Lines starting with `#` are ignored.
