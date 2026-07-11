# Network Usage

RelayQuay is quiet while idle. The always-on cost is mostly Cloudflare Tunnel
keepalive traffic plus occasional DNS/TLS control traffic.

The expensive part is an active ROV browser session. Screen resolution, stream
profile, motion, and polling cadence dominate bandwidth.

## Defaults That Reduce Waste

- Cloudflare Access rejects unauthenticated users at Cloudflare's edge before
  they can consume origin bandwidth.
- `cloudflared` runs with `--no-autoupdate`, avoiding surprise update
  downloads.
- The default `cloudflared` tag is pinned instead of `latest`.
- ROV has Balanced, Data Saver, and Emergency stream profiles.

## Metered Links

For metered Wi-Fi or mobile links:

```bash
./bin/relayquay stop cloudflared
./bin/relayquay up cloudflared
```

Use ROV's Data Saver or Emergency profile during long sessions. Keep Cloudflare
Access enabled so anonymous traffic is blocked at the edge.

## Configuration

```bash
CLOUDFLARED_PROTOCOL=quic
RELAYQUAY_RESTART_POLICY=unless-stopped
```

Use `RELAYQUAY_RESTART_POLICY=no` if you want Docker to leave the tunnel down
after you stop it manually.
