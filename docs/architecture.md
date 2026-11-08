# Plugroot Architecture

Plugroot is one Go program (`package main` at the repository root) with the
standard library only: no third-party modules, so a build fetches nothing and
`go list -m all` prints only `github.com/mmdmcy/plugroot`. It replaced the Rust
version on 2026-11-08 with the same commands, flags, output, files and HTTP
behaviour; that version is kept, buildable and tested, in `archive/rust`.

## Ownership map

| Responsibility | Source | Owns |
| --- | --- | --- |
| Entry | `main.go` | Global `--root`, command dispatch, help |
| Application | `manifest.go` | Manifest/environment loading, overlays, application context, manifest types |
| Manifest reader | `toml.go` | The TOML subset Plugroot's manifests use |
| Apply | `apply.go` | Repositories, managed files/directories/symlinks, and generated systemd units |
| Backup | `backup.go` | Emergency export planning, source discovery, metadata, archives, and encryption |
| Doctor | `doctor.go` | Read-only host, Docker, resource, and service health checks; release-check |
| Release | `release.go` | Binary releases for systemd services: install, list, rollback |
| Restore | `restore.go` | Archive discovery, validation, extraction, and restore finishing |
| Services | `services.go` | Start, stop, restart, and log actions |
| Status | `status.go` | Service state collection, list, status table and JSON |
| TUI | `tui.go` | Terminal dashboard: raw mode, escape codes, input, viewport |
| Web | `web.go`, `views.go`, `templates.go` | Private HTTP on raw TCP, authentication/session flow, HTML |
| Integrations | `process.go` | Process execution, Git and systemd user-manager helpers, error wording |
| Security | `audit.go`, `boundary.go` | Public-source auditing and code/state boundary checks |
| Shared | `paths.go`, `jsonout.go` | Path rules and pretty JSON shared by several commands |

## Dependency direction

```text
main -> commands (status, apply, doctor, web, tui, ...)
commands -> process (external programs) and manifest (context)
```

The private state root remains outside the public code checkout. External
command execution goes through `process.go`; host and product decisions belong
in the owning command's file. There is deliberately no generic utilities
package or runtime framework.

## Compatibility notes

Paths are joined and compared the way the Rust version did (no cleaning on
join, component-wise comparison), JSON is written exactly as serde_json's
pretty printer did, and error texts for operating-system errors keep the Rust
wording ("No such file or directory (os error 2)"), so scripts reading the
output see no change. The web dashboard answers on raw TCP with the same
bytes as before; it does not use `net/http`.

## Verification

```bash
gofmt -l .
go vet ./...
go test ./...
go list -m all
go run . audit-public
go run . boundary --strict
cargo test --manifest-path archive/rust/Cargo.toml   # the archived version
```
