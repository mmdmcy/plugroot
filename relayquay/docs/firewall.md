# Firewall

RelayQuay does not automatically change the host firewall. Firewall policy is
host-specific and a bad rule can lock you out or break private clients.

The intended policy is:

- Allow outbound `cloudflared` traffic to Cloudflare.
- Allow loopback access to the Caddy gateway on `127.0.0.1:8788`.
- Allow RustDesk raw ports only from private networks you trust.
- Deny public access to `21115-21119/tcp` and `21116/udp`.

## RustDesk Ports

```text
21115/tcp       NAT type test
21116/tcp+udp   rendezvous, heartbeat, and TCP hole punching
21117/tcp       relay
21118/tcp       hbbs WebSocket
21119/tcp       hbbr WebSocket
```

## UFW Sketch

Adjust the source networks before applying:

```bash
sudo ufw allow in on tailscale0 to any port 21115:21119 proto tcp
sudo ufw allow in on tailscale0 to any port 21116 proto udp
sudo ufw allow from 192.168.0.0/16 to any port 21115:21119 proto tcp
sudo ufw allow from 192.168.0.0/16 to any port 21116 proto udp

sudo ufw deny 21115:21119/tcp
sudo ufw deny 21116/udp
```

## nftables Example

See [examples/nftables/relayquay.nft](../examples/nftables/relayquay.nft).

Review it before use. It is intentionally an example, not an installer.
