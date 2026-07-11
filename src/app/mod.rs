//! Configuration loading and application context support.

mod manifest;
mod types;

#[cfg(test)]
pub(crate) use manifest::expand_vars;
pub(crate) use manifest::{load_env_file, load_manifest_file, merge_manifest};
#[cfg(test)]
pub(crate) use types::PlugrootConfig;
pub(crate) use types::{Context, ManagedDir, ManagedFile, Manifest, Repo, Service};
