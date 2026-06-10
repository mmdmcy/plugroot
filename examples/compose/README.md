# Example Compose Stack

This neutral stack exists only to show the manifest shape for a Docker Compose
service. Replace it in `plugroot.local.toml` with services for your own host.
On a real host, keep that local overlay under `PLUGROOT_STATE_ROOT`, not in the
code checkout.

The example binds to `PLUGROOT_PRIVATE_IP`, which should be a localhost or
private-network address.
