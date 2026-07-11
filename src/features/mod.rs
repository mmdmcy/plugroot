//! Product capability inventory.

pub(crate) mod apply;
pub(crate) mod backup;
pub(crate) mod doctor;
pub(crate) mod restore;
pub(crate) mod services;
pub(crate) mod status;

pub(crate) use apply::{cmd_apply, cmd_repos};
pub(crate) use backup::{
    absolute_clean_path, cmd_backup, cmd_emergency, emergency_metadata_commands,
    emergency_timestamp, program_available, EmergencyCommandSpec, EmergencyExportManifest,
    EmergencySource,
};
pub(crate) use doctor::{
    broad_ufw_allow_rules, cmd_doctor, cmd_release_check, expected_container_names,
    expected_manifest_tcp_ports, limit_lines, meminfo_kb, run_read_command,
    tailscale_text_has_public_funnel, unexpected_wildcard_tcp_listeners, DoctorFinding,
};
pub(crate) use restore::cmd_restore;
pub(crate) use services::{cmd_action, has_control, logs_for, run_fixed_action};
pub(crate) use status::{cmd_list, cmd_status, print_status_table, status_rows, StatusRow};
