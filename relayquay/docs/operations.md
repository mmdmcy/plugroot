# Operations

## Start

Start ROV first:

Run your RustOp Viewer-compatible host binary in headless mode. For example:

```bash
cd /path/to/rustopviewer
cargo run --release -- --headless
```

Then start RelayQuay:

```bash
./bin/relayquay doctor
./bin/relayquay up
./bin/relayquay status
```

## Stop Metered Tunnel Traffic

Stop only the Cloudflare connector while keeping local configuration intact:

```bash
./bin/relayquay stop cloudflared
```

Start it again:

```bash
./bin/relayquay up cloudflared
```

## Verify Public Access

Unauthenticated requests should redirect to Cloudflare Access:

```bash
curl -I https://rov.example.com/
```

The local gateway should respond directly on loopback:

```bash
curl -fsS http://127.0.0.1:8788/healthz
curl -I http://127.0.0.1:8788/
```

If Cloudflare returns `502 Bad Gateway` after Access login, check that the
tunnel ingress uses `http://127.0.0.1:8788`, that `cloudflared` is running, and
that ROV is listening at `ROV_UPSTREAM`.

## Rotate Tunnel Token

1. Create or rotate the tunnel token in Cloudflare.
2. Update only the private env file selected by `RELAYQUAY_ENV_FILE`.
3. Restart `cloudflared`:

   ```bash
   ./bin/relayquay up cloudflared
   ```
