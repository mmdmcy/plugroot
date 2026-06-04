# Migration Notes

Plugroot is meant to replace hand-maintained selfhost scripts gradually.

Recommended path:

```bash
plugroot status
plugroot apply --dry-run
plugroot repos sync
plugroot apply --dry-run
```

Only run real `plugroot apply` after the dry run matches what you expect.

For an existing host, keep service data in place until Plugroot proves it can
see and control the services. Move data last, not first.
