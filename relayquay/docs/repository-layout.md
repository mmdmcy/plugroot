# Repository Layout

```text
relayquay/
  bin/relayquay              helper script
  deploy/caddy/              Caddy origin config
  deploy/compose/            Docker Compose config
  docs/                      operator docs and ADRs
  examples/                  optional local integration examples
  infra/                     optional infrastructure templates
```

The repository is code and documentation only.

Do not commit:

- real `.env` files
- Cloudflare tokens
- real hostnames
- logs
- screenshots
- generated state
- pairing codes or session tokens
- local machine notes
