# Vaultwarden Pattern

This is a private-first pattern for running Vaultwarden from Plugroot.

Vaultwarden is an unofficial Bitwarden-compatible server. It should be treated
as a high-value service: keep it private, patched, backed up, and boring.

## Access Model

Recommended shape:

```text
Bitwarden clients
  -> private VPN HTTPS URL
  -> localhost-bound Vaultwarden container
  -> ignored service data
```

Do not expose Vaultwarden to the public internet by default. If Tailscale is the
private network, prefer Tailscale Serve over Tailscale Funnel. Serve keeps the
service inside the tailnet; Funnel intentionally publishes it to the internet.

Bitwarden clients require an `https://` self-hosted server URL. Tailscale HTTPS
is convenient, but certificate names are recorded in public Certificate
Transparency logs. Use non-sensitive machine and tailnet names before enabling
certificates.

## Setup Defaults

- Pin the Vaultwarden image tag and digest.
- Bind the container to `127.0.0.1`.
- Put HTTPS in front of it with a private-network proxy.
- Disable anonymous signups after creating the first account.
- Disable invitations unless sharing is explicitly needed.
- Disable password hints.
- Disable organization creation for normal users.
- Keep the admin panel disabled after setup.
- Store data under `services/vaultwarden/data`, which is ignored.
- Store backups under `backups/vaultwarden`, which is ignored.
- Do not use the Bitwarden npm CLI in automation.

## Client Notes

In the Bitwarden app or browser extension, choose the self-hosted environment
and enter the private HTTPS URL.

Offline behavior is read-only for already-synced clients. KeepPass-style offline
editing is a different model and is not what Vaultwarden optimizes for.

## Public Repo Safety

The public repo can contain generic examples and placeholders only. Never commit:

```text
VAULTWARDEN_DOMAIN with a real host
admin tokens
master passwords
database files
exports from Proton Pass or Bitwarden
Vaultwarden service data
private backup archives
```

Run the public audit before publishing:

```bash
cargo run -- audit-public
gitleaks detect --source . --redact
```
