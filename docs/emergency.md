# Emergency Export

`plugroot backup` creates a manual encrypted exit bundle for a reinstall, disk
replacement, or distribution migration.

The export is intentionally manual. Keep scheduled backups separate and use this
command when you want a point-in-time bundle before risky host work.

## What It Captures

The encrypted archive includes:

- the Plugroot code checkout
- the Plugroot state root, excluding rebuildable repo build caches
- RustDesk system and user config paths when present
- selected system edge config such as hostname, hosts, fstab, UFW, Tailscale,
  Docker, and Plugroot-managed systemd units
- encrypted metadata inventory for packages, disks, systemd, Docker, Tailscale,
  UFW, and Flatpak

The code checkout excludes build/cache-heavy directories such as `target`,
`.cache`, and `node_modules`. Managed repos under the state root use the same
cache exclusions, so service data and source checkouts are preserved without
packing rebuildable build output.

The command also writes a small companion `.manifest.json` next to the encrypted
archive. That manifest is meant to be non-secret and only records paths,
filenames, skipped missing paths, and the inventory commands that were run.

## Simple Usage

Use the short command for normal emergency backups:

```bash
plugroot backup
```

That writes to:

```text
~/Documents/plugroot-emergency-exports
```

To preview the capture list without writing an archive:

```bash
plugroot backup check
```

To write directly to a mounted USB or external disk, pass the destination as the
only extra word:

```bash
plugroot backup /media/$USER/USB
```

To inspect the backup from a new install or standby host:

```bash
plugroot restore check /media/$USER/USB
```

Plugroot creates the output directory when needed. It refuses output paths inside
the code checkout or state root, because that would put the rescue bundle inside
the data it is trying to protect.

By default GPG prompts for a symmetric passphrase:

```text
plugroot-emergency-<host>-<timestamp>.tar.zst.gpg
plugroot-emergency-<host>-<timestamp>.manifest.json
```

Store the passphrase somewhere that survives this laptop and does not depend on
the selfhosted services in the bundle. A separate trusted password manager or
offline record is appropriate. Do not keep the only copy in the code root, state
root, Documents directory, or the services being recovered.

## Advanced Usage

The explicit export command is kept for scripts and unusual destinations:

```bash
plugroot emergency export --output /media/$USER/USB --dry-run
plugroot emergency export --output /media/$USER/USB --yes
```

For noninteractive automation, provide a root-readable passphrase file:

```bash
plugroot emergency export \
  --output /media/$USER/USB \
  --passphrase-file /root/plugroot-emergency.pass \
  --yes
```

Do not keep the passphrase file in the Plugroot code checkout or state root.

## Restore Notes

See `docs/restore.md` for the full reinstall and second-server workflow. The
short version, once the `plugroot` command is available on the new host, is:

```bash
plugroot restore check /media/$USER/USB
plugroot restore /media/$USER/USB
plugroot restore finish
```

If the `plugroot` command is not available yet, manually bootstrap the roots:

```bash
gpg --decrypt /media/$USER/USB/plugroot-emergency-*.tar.zst.gpg > /tmp/plugroot-restore.tar.zst
# Read the companion manifest, then use its roots with leading slashes removed.
sudo tar --zstd -xpf /tmp/plugroot-restore.tar.zst -C / <archive-code-root> <archive-state-root>
sudo <code-root>/bin/plugroot --root <code-root> restore finish
```
