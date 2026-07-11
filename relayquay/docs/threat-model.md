# Threat Model

RelayQuay is for personal browser access to a RustOp Viewer host from devices
that are not on your private VPN.

## Goals

- Require Cloudflare Access identity authentication before a browser reaches
  the local ROV origin.
- Keep the origin listener on loopback.
- Preserve ROV's own host-approved pairing and session checks.
- Keep real operational state under `/var/lib/relayquay`, outside Git.

## Non-Goals

- RelayQuay is not an account system for ROV.
- RelayQuay is not a replacement for ROV pairing codes, remembered-browser
  trust, or input authorization.
- RelayQuay does not make an untrusted browser or unmanaged device trusted.
- RelayQuay does not hide traffic metadata from Cloudflare.

## Main Risks

### Origin Exposure

The intended public surface is the Cloudflare Access application only. Caddy
binds to `127.0.0.1:8788`, and ROV should stay on `127.0.0.1:45080` unless you
deliberately add a trusted local listener.

### Shared or Untrusted Devices

Cloudflare Access proves the browser user authenticated. It does not prove the
device is safe. Use short Access session durations on shared machines and avoid
remembering browsers you do not control.

### Pairing Code Handling

ROV pairing codes authorize a browser session. Treat them as short-lived
secrets and generate them only when pairing a browser you control.

### Remote Input

ROV controls whether pointer and keyboard input are accepted. Keep view-only or
input-disabled modes available when inspecting an unknown environment.

## Recommended Controls

- Cloudflare Access allow policy for your identity only.
- MFA on the identity provider.
- Short Access session duration for this application.
- ROV remembered-browser trust only on your own devices.
- ROV Data Saver or Emergency profile on metered links.
- Regular review of Cloudflare Access logs and local ROV logs.
