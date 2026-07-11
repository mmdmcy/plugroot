use crate::PathBuf;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug)]
pub(crate) struct EmergencyExportOptions {
    pub(crate) output_dir: PathBuf,
    pub(crate) yes: bool,
    pub(crate) dry_run: bool,
    pub(crate) passphrase_file: Option<PathBuf>,
    pub(crate) create_output_dir: bool,
    pub(crate) owner_user: Option<String>,
}

#[derive(Debug)]
pub(crate) struct EmergencySource {
    pub(crate) label: String,
    pub(crate) path: PathBuf,
}

#[derive(Debug)]
pub(crate) struct EmergencyCommandSpec {
    pub(crate) file_name: &'static str,
    pub(crate) program: &'static str,
    pub(crate) args: &'static [&'static str],
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct EmergencyExportManifest {
    pub(crate) version: u8,
    pub(crate) created_utc: String,
    pub(crate) code_root: String,
    pub(crate) state_root: String,
    pub(crate) output_dir: String,
    pub(crate) encrypted_archive: String,
    pub(crate) included_paths: Vec<String>,
    pub(crate) skipped_paths: Vec<String>,
    pub(crate) metadata_commands: Vec<String>,
}
