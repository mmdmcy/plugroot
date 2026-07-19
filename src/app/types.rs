use crate::load_env_file;
use crate::load_manifest_file;
use crate::merge_manifest;
use crate::resolve_path;
use crate::state_root_for_manifest;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::io;
use std::path::PathBuf;

#[derive(Clone)]
pub(crate) struct Context {
    pub(crate) root: PathBuf,
    pub(crate) manifest: Manifest,
    pub(crate) env_values: HashMap<String, String>,
}

impl Context {
    pub(crate) fn load(root: PathBuf) -> io::Result<Self> {
        let mut merged_env: HashMap<String, String> = env::vars().collect();

        for (key, value) in load_env_file(&root.join(".env"))? {
            merged_env.insert(key, value);
        }

        let main_path = root.join("plugroot.toml");
        let mut manifest = load_manifest_file(&main_path, &merged_env)?;
        let state_root = state_root_for_manifest(&root, &manifest);
        for (key, value) in load_env_file(&state_root.join(".env"))? {
            merged_env.insert(key, value);
        }

        manifest = load_manifest_file(&main_path, &merged_env)?;
        let state_root = state_root_for_manifest(&root, &manifest);
        let code_local_path = root.join("plugroot.local.toml");
        let state_local_path = state_root.join("plugroot.local.toml");
        for local_path in [&code_local_path, &state_local_path] {
            if local_path.exists() {
                let local = load_manifest_file(local_path, &merged_env)?;
                manifest = merge_manifest(manifest, local);
            }
        }

        Ok(Self {
            root,
            manifest,
            env_values: merged_env,
        })
    }

    pub(crate) fn service(&self, id: &str) -> Option<Service> {
        self.manifest
            .service
            .iter()
            .find(|service| service.id == id)
            .cloned()
    }

    pub(crate) fn services_for_target(&self, target: &str) -> Vec<Service> {
        if target == "all" {
            self.manifest
                .service
                .iter()
                .filter(|service| {
                    service
                        .controls
                        .as_ref()
                        .map(|controls| !controls.is_empty())
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        } else {
            self.service(target).into_iter().collect()
        }
    }

    pub(crate) fn state_root(&self) -> PathBuf {
        state_root_for_manifest(&self.root, &self.manifest)
    }

    pub(crate) fn repo_dir(&self) -> Option<PathBuf> {
        self.manifest
            .plugroot
            .as_ref()
            .and_then(|config| config.repo_dir.as_deref())
            .map(|path| resolve_path(&self.root, path))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) plugroot: Option<PlugrootConfig>,
    pub(crate) host: Option<HostConfig>,
    #[serde(default)]
    pub(crate) repo: Vec<Repo>,
    #[serde(default)]
    pub(crate) directory: Vec<ManagedDir>,
    #[serde(default)]
    pub(crate) file: Vec<ManagedFile>,
    #[serde(default)]
    pub(crate) symlink: Vec<ManagedSymlink>,
    #[serde(default)]
    pub(crate) service: Vec<Service>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct PlugrootConfig {
    pub(crate) code_root: Option<String>,
    pub(crate) root: Option<String>,
    pub(crate) state_root: Option<String>,
    pub(crate) repo_dir: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct HostConfig {
    pub(crate) name: Option<String>,
    pub(crate) private_ip: Option<String>,
    pub(crate) user: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Repo {
    pub(crate) id: String,
    pub(crate) url: String,
    pub(crate) path: String,
    #[serde(rename = "ref")]
    pub(crate) git_ref: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ManagedDir {
    pub(crate) path: String,
    pub(crate) owner: Option<String>,
    pub(crate) mode: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ManagedFile {
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) owner: Option<String>,
    pub(crate) mode: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ManagedSymlink {
    pub(crate) source: String,
    pub(crate) target: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Service {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) plane: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) kind: String,
    pub(crate) path: Option<String>,
    pub(crate) unit: Option<String>,
    pub(crate) user: Option<String>,
    pub(crate) host: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) url: Option<String>,
    pub(crate) access: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) containers: Option<Vec<String>>,
    pub(crate) controls: Option<Vec<String>>,
    pub(crate) ports: Option<Vec<String>>,
    pub(crate) optional: Option<bool>,
    pub(crate) repo: Option<String>,
    pub(crate) working_dir: Option<String>,
    pub(crate) command: Option<Vec<String>>,
    pub(crate) env: Option<Vec<String>>,
    pub(crate) env_file: Option<String>,
    pub(crate) unit_source: Option<String>,
}
