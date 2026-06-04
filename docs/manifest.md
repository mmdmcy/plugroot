# Manifest

Plugroot uses TOML because it is readable, comment-friendly, and easy to parse
from Rust.

## Files

```text
plugroot.toml
plugroot.local.toml
.env
```

`plugroot.toml` is safe to commit. `plugroot.local.toml` and `.env` are ignored.

## Repos

```toml
[[repo]]
id = "example-app"
url = "https://github.com/example/example-app.git"
path = "${PLUGROOT_REPO_DIR:-/opt/plugroot/repos}/example-app"
ref = "main"
```

`plugroot repos sync` clones missing repos and fetches/pulls existing ones.

## Managed Directories And Files

`plugroot apply` can prepare directories and copy local template files:

```toml
[[directory]]
path = "/opt/plugroot/services/example/data"
owner = "plugroot:plugroot"
mode = "0755"

[[file]]
source = "systemd/example.service"
target = "/etc/systemd/system/example.service"
mode = "0644"
```

Use ignored local overlays for host-specific files and paths.

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

Compose services use the root `.env` automatically when it exists. A service can
override that with:

```toml
env_file = "services/example/.env"
```

## Generated Units

A `systemd` or `user-systemd` service with `command` can generate a unit:

```toml
kind = "user-systemd"
unit = "example-app.service"
user = "${PLUGROOT_USER:-plugroot}"
working_dir = "${PLUGROOT_REPO_DIR:-/opt/plugroot/repos}/example-app"
command = ["/usr/bin/python3", "server.py"]
```

Alternatively, copy a complete unit template from the repo or local overlay:

```toml
kind = "systemd"
unit = "example.service"
unit_source = "systemd/example.service"
```

Use `plugroot apply --dry-run` before writing or installing anything.
