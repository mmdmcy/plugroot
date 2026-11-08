use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DoctorSeverity {
    Ok,
    Info,
    Warn,
    Error,
}

impl DoctorSeverity {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DoctorFinding {
    pub(crate) severity: DoctorSeverity,
    pub(crate) check: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
}

impl DoctorFinding {
    pub(crate) fn new(
        severity: DoctorSeverity,
        check: impl Into<String>,
        message: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            severity,
            check: check.into(),
            message: message.into(),
            detail,
        }
    }

    pub(crate) fn ok(check: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(DoctorSeverity::Ok, check, message, None)
    }

    pub(crate) fn info(
        check: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::new(DoctorSeverity::Info, check, message, Some(detail.into()))
    }

    pub(crate) fn warn(
        check: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::new(DoctorSeverity::Warn, check, message, Some(detail.into()))
    }

    pub(crate) fn error(
        check: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::new(DoctorSeverity::Error, check, message, Some(detail.into()))
    }
}
