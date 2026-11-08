#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ApplyOptions {
    pub(crate) dry_run: bool,
    pub(crate) sync_repos: bool,
}
