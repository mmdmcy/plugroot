//! Narrow helpers shared across multiple capabilities.

mod paths;

pub(crate) use paths::{
    clean_path, default_state_root, nearest_git_root, resolve_path, same_or_inside,
    state_root_for_manifest,
};
