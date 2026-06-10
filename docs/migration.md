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
/var/lib/plugroot    private state root
```

Your development checkout can live somewhere else, such as a GitHub workspace.
Make changes there, commit and push, then update the selfhost checkout with
`git pull`. Do not put real `.env`, `plugroot.local.toml`, cloned repos,
service databases, media, or backups inside either checkout.

Recommended path:

```bash
sudo install -d -o "$USER" -g "$USER" -m 700 /var/lib/plugroot
cp .env.example /var/lib/plugroot/.env
cp plugroot.local.example.toml /var/lib/plugroot/plugroot.local.toml
cargo run -- boundary --strict
plugroot status
plugroot apply --dry-run
plugroot repos sync
plugroot apply --dry-run
```

Only run real `plugroot apply` after the dry run matches what you expect.

For an existing host, keep service data in place until Plugroot proves it can
see and control the services. Move data last, not first.

For a mixed old checkout, move private files out before treating the checkout as
publishable:

```text
.env                         -> /var/lib/plugroot/.env
plugroot.local.toml          -> /var/lib/plugroot/plugroot.local.toml
.plugroot/                   -> /var/lib/plugroot/.plugroot/
repos/                       -> /var/lib/plugroot/repos/
services/*/data/             -> /var/lib/plugroot/services/*/data/
backups/                     -> /var/lib/plugroot/backups/
```

Run `plugroot boundary --strict` after each migration step. It should be clean
before an agent commits, pushes, or restarts services from that checkout.
