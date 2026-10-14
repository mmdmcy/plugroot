pub(crate) fn print_help() {
    println!(
        r#"Plugroot - private-first selfhost harness

Usage:
  plugroot [--root <path>] status [--json]
  plugroot [--root <path>] list
  plugroot [--root <path>] apply [--dry-run] [--sync-repos]
  plugroot [--root <path>] repos sync
  plugroot [--root <path>] doctor [--json] [--strict]
  plugroot [--root <path>] release-check
  plugroot [--root <path>] backup [check] [directory]
  plugroot restore [check|finish] [directory-or-archive]
  plugroot [--root <path>] emergency export --output <directory> --yes
  plugroot [--root <path>] up|down|restart|logs <service|all>
  plugroot [--root <path>] tui [--once]
  plugroot [--root <path>] web [--bind <addr:port>]...
  plugroot [--root <path>] boundary [--strict]
  plugroot [--root <path>] audit-public [--install-hook]

Files:
  plugroot.toml                         public manifest in the code root
  $PLUGROOT_STATE_ROOT/.env             private local values
  $PLUGROOT_STATE_ROOT/plugroot.local.toml  private local overlay
"#
    );
}
