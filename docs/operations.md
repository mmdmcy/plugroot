# Operations

Plugroot should have one predictable host command:

```bash
plugroot <subcommand>
```

The installed `/usr/local/bin/plugroot` launcher delegates to the real binary in
the code checkout, normally `/opt/plugroot/bin/plugroot`, and adds the expected
`--root /opt/plugroot` argument. This keeps daily operations consistent without
requiring operators to remember checkout paths.

## Command Shape

Use subcommands for operator actions:

```bash
plugroot status
plugroot doctor
plugroot release-check
plugroot tui
plugroot apply --dry-run
plugroot up example-web
plugroot logs example-web
```

Avoid adding separate dash-named host commands such as `plugroot-tui`. They make
the interface harder to predict and drift away from the CLI help text. If a
shortcut is needed later, prefer a shell alias outside the repo instead of a new
installed command.

## Health Checks

`plugroot doctor` is the read-only first stop before cleanup work. It checks:

- the code/state boundary
- public audit findings
- declared service health
- CPU load, memory, and root disk pressure
- failed system and user units
- broad UFW allow rules
- public Tailscale Funnel exposure
- unexpected wildcard TCP listeners
- stale Docker containers, networks, volumes, and cache
- retired tmux sessions
- stale FUSE mounts

By default it exits nonzero for errors. Use `--strict` when warnings should also
fail a check, and `--json` when another tool needs structured output.

## Release Checks

Run `plugroot release-check` before publishing, pulling into production, or
restarting an important service. It fails when the code/state boundary is not
clean, tracked files fail the public audit, the Plugroot checkout is dirty, or a
manifest-managed repo checkout has uncommitted changes.

## Applying Changes

Before a real `plugroot apply`, run:

```bash
plugroot boundary --strict
plugroot audit-public
plugroot apply --dry-run
```

The dry run should show only the files, directories, repos, and units you expect
to touch. Host-specific files copied by `apply` should come from the private
state root, not from the public checkout.
