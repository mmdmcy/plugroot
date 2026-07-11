# Restore And Standby

Plugroot restore is meant for two related scenarios:

- reinstall this laptop or replace its disk
- prepare a second active/passive selfhost server that can take over if this one
  dies

It is not live replication. Do not run the same service databases writable on
two machines at the same time unless a service has been explicitly designed for
replication.

## What The Backup Provides

`plugroot backup` carries the important shape of the selfhost:

- the code root recorded in the companion manifest
- the separate state root recorded in the companion manifest
- Docker Compose files and service-local configs
- systemd unit definitions
- service data and SQLite databases under the state root
- selected host config and inventory for reference

It does not carry a cloned operating system. A fresh host still needs base
packages such as Git, GPG, Zstandard, Docker with the Compose plugin, systemd,
Tailscale when used, and any desktop/remote-access packages you expect.

## Old Host

Create the encrypted backup:

```bash
plugroot backup /media/$USER/USB
```

Use `plugroot backup check /media/$USER/USB` first when you only want to inspect
what will be captured.

Store the GPG passphrase somewhere independent of this host and independent of
Vaultwarden.

## New Host With Plugroot Available

Inspect the USB or archive:

```bash
plugroot restore check /media/$USER/USB
```

Restore the two Plugroot roots:

```bash
plugroot restore /media/$USER/USB
```

This extracts the two roots recorded by the companion manifest. Without a
manifest, it uses the selected `--root` and a sibling `<code-root>-state`
directory.

```text
<code-root>
<state-root>
```

When the state root differs from the legacy location, it also creates the
compatibility link:

```text
/var/lib/plugroot -> <state-root>
```

The command refuses to overwrite existing target roots or the compatibility
link.

After host prerequisites are installed, finish the restore:

```bash
plugroot restore finish
```

`restore finish` runs:

- `plugroot boundary --strict`
- `plugroot apply --dry-run`
- after confirmation, `plugroot apply`
- `plugroot up all`
- `plugroot doctor`

Repository sync is intentionally not part of `restore finish`. The backup
contains the managed repo checkouts, and `plugroot repos sync` should be run
later only when you want to fetch current upstream code.

## New Host Without Plugroot Available

If the command is not installed yet, bootstrap manually from the archive:

```bash
gpg --decrypt /media/$USER/USB/plugroot-emergency-*.tar.zst.gpg > /tmp/plugroot-restore.tar.zst
# Read the companion manifest, then use its roots with leading slashes removed.
sudo tar --zstd -xpf /tmp/plugroot-restore.tar.zst -C / <archive-code-root> <archive-state-root>
sudo <code-root>/bin/plugroot --root <code-root> restore finish
rm /tmp/plugroot-restore.tar.zst
```

That path is less polished, but it is enough to recover after a reinstall.

## Second Server

Use the same flow for a second standby server, but treat it as active/passive:

```bash
plugroot restore check /media/$USER/USB
plugroot restore /media/$USER/USB
plugroot restore finish
```

Before leaving the second server powered on, decide which services should stay
stopped. Password managers, Git forges, and media databases generally should not
be writable on both hosts at once from the same backup lineage.

For a simple standby, keep the second host patched and tested, then refresh it
from a newer `plugroot backup` when you want a newer recovery point.
