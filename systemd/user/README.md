# User Systemd Units

Plugroot writes generated user units into `.plugroot/generated/systemd/user`
and installs them into the selected user's systemd directory when `plugroot
apply` runs without `--dry-run`.

This tracked directory is kept for hand-written templates if a service needs
one later.
