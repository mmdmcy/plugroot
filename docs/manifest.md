# Manifest

Plugroot uses TOML because it is readable, comment-friendly, and easy to parse
from Rust.

## Files

```text
plugroot.toml
$PLUGROOT_STATE_ROOT/plugroot.local.toml
$PLUGROOT_STATE_ROOT/.env
```

`plugroot.toml` is safe to commit. Real local overlays and `.env` files belong
under the private state root, not in the Git checkout. Code-root
`plugroot.local.toml` and `.env` files are still read for old installs, but
`plugroot boundary --strict` flags them.

## Plugroot Section

```toml
[plugroot]
state_root = "${PLUGROOT_STATE_ROOT:-../plugroot-state}"
repo_dir = "${PLUGROOT_REPO_DIR:-../plugroot-state/repos}"
```

`state_root` is local-only machine state. Its default is a sibling
`../plugroot-state` directory relative to the code root. `repo_dir` is where
Plugroot-managed app repos are cloned. Both should be outside the Plugroot code
checkout.

## Repos

```toml
[[repo]]
id = "example-app"
url = "https://github.com/example/example-app.git"
path = "${PLUGROOT_REPO_DIR:-../plugroot-state/repos}/example-app"
ref = "main"
```

`plugroot repos sync` clones missing repos and fetches/pulls existing ones.

### Standalone Helper-Managed Applications

Keep application source in its own repository instead of vendoring it into
Plugroot. Register the checkout with `[[repo]]`, then use a manual service when
the application has a helper that owns runtime modes or non-default Compose
overlays.

RelayQuay is one example:

```toml
[[repo]]
id = "relayquay"
url = "https://github.com/mmdmcy/RelayQuay.git"
path = "<absolute-path-to-relayquay-checkout>"
ref = "main"

[[service]]
id = "relayquay"
name = "RelayQuay RustOpViewer Gateway"
plane = "public-authenticated"
category = "remote-access"
kind = "manual"
host = "127.0.0.1"
port = 8788
repo = "relayquay"
url = "https://rov.example.com/healthz"
access = "Cloudflare Access allowlist"
description = "Standalone gateway monitored by Plugroot."
controls = []
ports = [
  "8788/tcp loopback gateway",
  "45080/tcp loopback app upstream",
]
optional = true
```

Use an absolute repo path in the private overlay. Managed repo paths currently
resolve from the Plugroot process working directory, so a relative path can
refer to different locations depending on how Plugroot was launched.

Run `plugroot repos sync` to update the checkout. Use RelayQuay's own
`bin/relayquay` helper for `doctor`, `activate`, `cover`, `pause`, and runtime
logs; Plugroot deliberately exposes no lifecycle controls for this manual
monitor. RelayQuay maintains a complete private-overlay example in its
[Plugroot integration file](https://github.com/mmdmcy/RelayQuay/blob/main/examples/plugroot/plugroot.local.example.toml).

## Managed Directories And Files

`plugroot apply` can prepare directories and copy local template files:

```toml
[[directory]]
path = "${PLUGROOT_STATE_ROOT:-../plugroot-state}/services/example/data"
owner = "plugroot:plugroot"
mode = "0755"

[[file]]
source = "systemd/example.service"
target = "/etc/systemd/system/example.service"
mode = "0644"
```

Use state-root local overlays for host-specific files and paths.

## Services

Supported service kinds:

```text
compose
systemd
user-systemd
port
manual
```

Controls are always explicit:

```toml
controls = ["start", "stop", "restart", "logs"]
```

If a control is absent, the CLI, TUI, and web dashboard refuse that action.

Compose services use the state-root `.env` automatically when it exists. A
service can override that with:

```toml
env_file = "${PLUGROOT_STATE_ROOT:-../plugroot-state}/services/example/.env"
```

## Generated Units

A `systemd` or `user-systemd` service with `command` can generate a unit:

```toml
kind = "user-systemd"
unit = "example-app.service"
user = "${PLUGROOT_USER:-plugroot}"
working_dir = "${PLUGROOT_REPO_DIR:-../plugroot-state/repos}/example-app"
command = ["/usr/bin/python3", "server.py"]
```

Alternatively, copy a complete unit template from the repo or local overlay:

```toml
kind = "systemd"
unit = "example.service"
unit_source = "systemd/example.service"
```

Use `plugroot apply --dry-run` before writing or installing anything. Repository
updates are separate by default; run `plugroot repos sync` when you actually
want to clone, fetch, or pull managed repos.
