use crate::PathBuf;

#[derive(Debug)]
pub(crate) struct AuditFinding {
    pub(crate) path: String,
    pub(crate) line: Option<usize>,
    pub(crate) message: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BoundarySeverity {
    Error,
    Warning,
}

impl BoundarySeverity {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

#[derive(Debug)]
pub(crate) struct BoundaryFinding {
    pub(crate) severity: BoundarySeverity,
    pub(crate) path: Option<PathBuf>,
    pub(crate) message: String,
}

impl BoundaryFinding {
    pub(crate) fn error(path: Option<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            severity: BoundarySeverity::Error,
            path,
            message: message.into(),
        }
    }

    pub(crate) fn warning(path: Option<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            severity: BoundarySeverity::Warning,
            path,
            message: message.into(),
        }
    }
}
