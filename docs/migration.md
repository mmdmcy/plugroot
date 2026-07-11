# Migration Notes

Plugroot is meant to replace hand-maintained selfhost scripts gradually.

## Code And State Split

Use two locations:

```text
code checkout  a normal Git clone, safe to push and pull
state root     private host files, no Git repo
```

A common host layout is:

```text
/opt/plugroot        selfhost code checkout
/opt/plugroot-state  private state root
```

Your development checkout can live somewhere else, such as a GitHub workspace.
Make changes there, commit and push, then update the selfhost checkout with
`git pull`. Do not put real `.env`, `plugroot.local.toml`, cloned repos,
service databases, media, or backups inside either checkout.

Recommended path:

```bash
sudo install -d -o "$USER" -g "$USER" -m 700 /opt/plugroot-state
cp .env.example /opt/plugroot-state/.env
cp plugroot.local.example.toml /opt/plugroot-state/plugroot.local.toml
cargo run -- boundary --strict
plugroot status
plugroot apply --dry-run
plugroot repos sync
plugroot apply --dry-run
```

Only run real `plugroot apply` after the dry run matches what you expect. A real
`apply` does not sync repos unless you explicitly pass `--sync-repos`; use
`plugroot repos sync` as the normal code update step.

For an existing host, keep service data in place until Plugroot proves it can
see and control the services. Move data last, not first.

For a mixed old checkout, move private files out before treating the checkout as
publishable:

```text
.env                         -> /opt/plugroot-state/.env
plugroot.local.toml          -> /opt/plugroot-state/plugroot.local.toml
.plugroot/                   -> /opt/plugroot-state/.plugroot/
repos/                       -> /opt/plugroot-state/repos/
services/*/data/             -> /opt/plugroot-state/services/*/data/
backups/                     -> /opt/plugroot-state/backups/
```

For older hosts, `/var/lib/plugroot` can remain as a compatibility symlink to
`/opt/plugroot-state`.

Run `plugroot boundary --strict` after each migration step. It should be clean
before an agent commits, pushes, or restarts services from that checkout.
