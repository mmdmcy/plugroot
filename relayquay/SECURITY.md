# Security

RelayQuay is designed to be public source code with private runtime state.

## Do Not Commit

- Cloudflare Tunnel tokens.
- Real hostnames or domains for private deployments.
- Real `.env` files.
- Local firewall state, service logs, media, backups, screenshots, or machine
  notes.
- Pairing codes, browser session tokens, or generated runtime state.

Use `.env.example` and `rov.example.com` style placeholders in public files.
Keep real values in a private env file selected with `RELAYQUAY_ENV_FILE`.

## Expected Exposure

The only intended public surface is an HTTPS hostname protected by Cloudflare
Access. Caddy should bind to loopback, and RustOp Viewer should stay loopback
unless you deliberately add a trusted local listener.

## Reporting

Open an issue with a minimal reproduction and avoid including secrets, real
hostnames, or private network details.
