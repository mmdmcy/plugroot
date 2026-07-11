# Plugroot Architecture

Plugroot follows the KatRust ownership layout. `src/main.rs` is only the binary
entry point and `src/lib.rs` is the command dispatch and module inventory.

## Ownership map

| Responsibility | Source | Owns |
| --- | --- | --- |
| Application | `src/app/` | Manifest/environment loading, application context, and manifest types |
| Apply | `src/features/apply/` | Repositories, managed files/directories, and generated systemd units |
| Backup | `src/features/backup/` | Emergency export planning, source discovery, metadata, archives, and encryption |
| Doctor | `src/features/doctor/` | Read-only host, Docker, resource, and service health checks |
| Restore | `src/features/restore/` | Archive discovery, validation, extraction, and restore finishing |
| Services | `src/features/services/` | Start, stop, restart, and log actions |
| Status | `src/features/status/` | Service state collection and list/status presentation data |
| CLI | `src/interfaces/cli/` | Command-line help |
| TUI | `src/interfaces/tui/` | Terminal dashboard state, input, viewport, and rendering |
| Web | `src/interfaces/web/` | Private HTTP handling, authentication/session flow, and HTML rendering |
| Integrations | `src/integrations/` | Process execution plus Git and systemd host protocols |
| Security | `src/security/` | Code/state boundary checks and public-source auditing |
| Shared | `src/shared/` | Path helpers used by several capabilities |

## Dependency direction

```text
main -> library dispatch -> interfaces / features
interfaces -> features -> integrations
features and security -> app context
```

The private state root remains outside the public code checkout. External
command execution belongs in `integrations`; host and product decisions belong
in the owning feature. Shared code must stay narrow—there is deliberately no
generic `utils.rs` or runtime framework.

## Refactor safety

The original multi-thousand-line binary was split by top-level responsibility,
with all existing tests run after each extraction checkpoint. Continue that
pattern: move one responsibility cluster, preserve behavior, test, then update
this ownership map. Use `cargo katrust inspect`, `cargo katrust map`, and
`cargo katrust check` when changing the structure.

## Verification

```bash
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo katrust check
cargo run -- audit-public
cargo run -- boundary --strict
```
