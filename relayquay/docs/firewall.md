# Firewall

RelayQuay does not automatically change the host firewall. Firewall policy is
host-specific and a bad rule can lock you out.

The intended policy is:

- Allow outbound `cloudflared` traffic to Cloudflare.
- Allow loopback access to Caddy on `127.0.0.1:8788`.
- Keep RustOp Viewer on `127.0.0.1:45080` by default.
- Do not publish ROV directly on public interfaces.

## UFW Sketch

Most single-host installs need no inbound public rule for RelayQuay because
Cloudflare Tunnel is outbound-only. You can still document explicit denials:

```bash
sudo ufw deny 8788/tcp
sudo ufw deny 45080/tcp
```

If you deliberately expose ROV to a private interface, allow only that trusted
interface or network.

## nftables Example

See [examples/nftables/relayquay.nft](../examples/nftables/relayquay.nft).

Review it before use. It is intentionally an example, not an installer.
