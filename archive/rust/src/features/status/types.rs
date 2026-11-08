use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct StatusRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) plane: String,
    pub(crate) category: String,
    pub(crate) kind: String,
    pub(crate) state: String,
    pub(crate) detail: String,
    pub(crate) url: Option<String>,
    pub(crate) access: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) ports: Vec<String>,
    pub(crate) repo: Option<String>,
    pub(crate) controls: Vec<String>,
    pub(crate) optional: bool,
}
