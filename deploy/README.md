# Selfhost deployment set (plugroot-managed hosts)

Standardized, reproducible deployment of the selfhost stack that plugroot
manages: Forgejo (git), Jellyfin (media), Navidrome (music), RustDesk relay
(remote support), plus auxiliary units (ytdlp-web, autodale-monitor).

## Layout

- `compose.yaml` — the 5-container docker stack. No secrets inline: every
  per-host value comes from `/opt/plugroot-state/.env` (interpolation).
- `env.example` — per-host variable template (copy keys into the state-root
  `.env`, never overwrite it).
- `units/` — systemd unit templates (`@USER@` placeholder).

## Install on a new host

1. State root must exist: `/opt/plugroot-state` with `services/`, `media/`,
   `releases/`, `.env`. (Migration: rsync from the previous host.)
2. Merge `env.example` keys into `/opt/plugroot-state/.env`, set `TAILNET_IP`
   and `FORGEJO_DOMAIN` for the new host.
3. Start the stack:

   ```bash
   cd /opt/plugroot/deploy
   docker compose --env-file /opt/plugroot-state/.env up -d
   ```

4. Install units:

   ```bash
   sed -i 's/@USER@/<user>/' units/ytdlp-web.service
   sudo install -m 0644 units/ytdlp-web.service /etc/systemd/system/
   sudo systemctl daemon-reload && sudo systemctl enable --now ytdlp-web
   ```

5. Tailscale serve (daled, ytdlp-web, identity, rustopviewer):

   ```bash
   tailscale serve --bg --set-path=/ 7346 http://127.0.0.1:7345
   tailscale serve --bg --set-path=/ 8443 http://127.0.0.1:8443
   tailscale serve --bg --set-path=/ 45081 http://127.0.0.1:45080
   ```

## Forgejo domain changes

`FORGEJO_DOMAIN` in the state-root `.env` overrides the domain at container
start. Changing it invalidates existing git remotes and the web UI base URL —
coordinate with clients (gitpriv store, docs) before switching.

## RustDesk relay

The relay keypair lives in `/opt/plugroot-state/rustdesk/data`. Copying the
state root preserves the keypair, so clients do not need re-registration. The
`RUSTDESK_RELAY` value is what hbbs tells clients to reach; update client
`RustDesk2.toml` relays to the new host after switchover.
