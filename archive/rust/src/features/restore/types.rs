use crate::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestoreArchiveKind {
    Encrypted,
    Plain,
}

impl RestoreArchiveKind {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Encrypted => "encrypted GPG archive",
            Self::Plain => "plain unencrypted tarball",
        }
    }
}

#[derive(Debug)]
pub(crate) struct RestorePlan {
    pub(crate) archive: PathBuf,
    pub(crate) kind: RestoreArchiveKind,
    pub(crate) manifest: Option<PathBuf>,
    pub(crate) code_root: PathBuf,
    pub(crate) state_root: PathBuf,
}
