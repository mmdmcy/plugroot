# Manifest

Plugroot uses TOML because it is readable and comment-friendly. Plugroot reads
it with its own small reader (standard library only): tables, arrays of tables,
"basic" and 'literal' strings, integers, booleans and arrays. Floats, dates,
inline tables, dotted keys and multi-line strings are refused with their line
number rather than half understood.

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

## Workspace Private State And Symlinks

Keep ignored credentials and other private workspace state under one canonical
state-root folder, then project them into application checkouts with managed
symlinks. Declare host-specific targets in the private local overlay:

```toml
[[directory]]
path = "${PLUGROOT_STATE_ROOT:-../plugroot-state}/workspaces/example-app"
mode = "0700"

[[symlink]]
source = "${PLUGROOT_STATE_ROOT:-../plugroot-state}/workspaces/example-app/.env"
target = "${EXAMPLE_APP_REPO}/.env"
```

Create or migrate the source file before applying the manifest. The source must
exist inside the state root. The target parent must already exist, and targets
inside Git checkouts must be ignored. `plugroot apply` is idempotent when the
correct link already exists and refuses to replace regular files or conflicting
links.

This keeps one private source of truth under the state root while applications
can continue reading files from their usual paths. Emergency exports already
include the complete state root, so the linked private state travels with the
rest of Plugroot without copying Git worktrees or build output.

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

A service's `url` is its link in the overview: the web dashboard opens it from
the service's name and its Open button, and `plugroot status --json` and
`plugroot list` report it. Give the address the people who use the dashboard
reach the service at, by name where the private network has names (a loopback
URL only works on the host itself); a service without a page of its own has no
`url` and is listed under "No direct link". `access` says how it is reached,
for people. The web dashboard reads the manifests and `.env` files again on
every request, as the TUI does on refresh, so a service added to or changed in
the overlay shows on the next page load without a restart; when the files no
longer load, it keeps serving the last manifest that did and logs why once.

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
