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
state_root = "${PLUGROOT_STATE_ROOT:-/var/lib/plugroot}"
repo_dir = "${PLUGROOT_REPO_DIR:-/var/lib/plugroot/repos}"
```

`state_root` is local-only machine state. `repo_dir` is where Plugroot-managed
app repos are cloned. Both should be outside the Plugroot code checkout.

## Repos

```toml
[[repo]]
id = "example-app"
url = "https://github.com/example/example-app.git"
path = "${PLUGROOT_REPO_DIR:-/var/lib/plugroot/repos}/example-app"
ref = "main"
```

`plugroot repos sync` clones missing repos and fetches/pulls existing ones.

## Managed Directories And Files

`plugroot apply` can prepare directories and copy local template files:

```toml
[[directory]]
path = "${PLUGROOT_STATE_ROOT:-/var/lib/plugroot}/services/example/data"
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
env_file = "${PLUGROOT_STATE_ROOT:-/var/lib/plugroot}/services/example/.env"
```

## Generated Units

A `systemd` or `user-systemd` service with `command` can generate a unit:

```toml
kind = "user-systemd"
unit = "example-app.service"
user = "${PLUGROOT_USER:-plugroot}"
working_dir = "${PLUGROOT_REPO_DIR:-/var/lib/plugroot/repos}/example-app"
command = ["/usr/bin/python3", "server.py"]
```

Alternatively, copy a complete unit template from the repo or local overlay:

```toml
kind = "systemd"
unit = "example.service"
unit_source = "systemd/example.service"
```

Use `plugroot apply --dry-run` before writing or installing anything.
