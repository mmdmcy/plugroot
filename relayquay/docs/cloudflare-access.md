# Cloudflare Access

RelayQuay assumes Cloudflare is only the authenticated front door for the ROV
browser UI. The origin still lives on this host.

## Tunnel Ingress

Create one public hostname and route it to the loopback Caddy listener:

```text
Hostname: rov.example.com
Service:  http://127.0.0.1:8788
```

Do not route directly to ROV. RelayQuay owns the small loopback gateway and
health endpoint.

## Access Application

Create a self-hosted Access application:

```text
Application domain: rov.example.com
Session duration:   short enough for the device risk
Policy:             allow only your identity or identity group
```

Use MFA where possible. Cloudflare One-time PIN can work for a personal setup,
but it is only the outer gate. ROV pairing is still required for a new browser.

## Expected Flow

1. Open the protected hostname.
2. Complete Cloudflare Access login.
3. Enter a host-generated ROV pairing code, or use an already remembered
   browser.

If Cloudflare returns `502 Bad Gateway`, verify:

- `cloudflared` is running.
- The tunnel ingress points to `http://127.0.0.1:8788`.
- Caddy is up.
- ROV is reachable at `ROV_UPSTREAM`.
