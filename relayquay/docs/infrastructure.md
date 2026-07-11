# Infrastructure

RelayQuay keeps infrastructure small:

- one Caddy container on host networking, bound to loopback
- one `cloudflared` container on host networking
- one private env file outside Git
- one local ROV upstream

Do not commit Cloudflare credentials, real hostnames, logs, screenshots, local
notes, pairing codes, or browser session material.

## Required External Pieces

- A Cloudflare zone and tunnel.
- A Cloudflare Access self-hosted application.
- A running RustOp Viewer host runtime.

## Private State

Use `/var/lib/relayquay` or another private state directory selected by
`RELAYQUAY_ENV_FILE` and `RELAYQUAY_STATE_DIR`.
