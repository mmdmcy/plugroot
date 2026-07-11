# Contributing

RelayQuay is public-source infrastructure code. Treat every contribution as if
it will be published.

## Development

Run the lightweight checks before opening a pull request:

```bash
bash -n bin/relayquay
RELAYQUAY_ENV_FILE=.env.example ./bin/relayquay config >/tmp/relayquay-compose.yaml
```

## Privacy

Do not commit real `.env` files, tunnel tokens, hostnames for private
deployments, logs, databases, screenshots, pairing codes, browser session
tokens, or local operator notes.

Use placeholders in examples and keep real values in the private env file
selected by `RELAYQUAY_ENV_FILE`.
