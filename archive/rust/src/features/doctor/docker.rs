use crate::expected_container_names;
use crate::limit_lines;
use crate::run_command;
use crate::Context;
use crate::DoctorFinding;

pub(crate) fn doctor_docker(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
    doctor_docker_containers(ctx, findings);
    doctor_docker_networks(findings);
    doctor_docker_volumes(findings);
    doctor_docker_reclaimable(findings);
}

pub(crate) fn doctor_docker_containers(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
    let out = run_command(
        "docker",
        &[
            "ps".into(),
            "-a".into(),
            "--format".into(),
            "{{.Names}}\t{{.Status}}\t{{.Label \"com.docker.compose.project\"}}".into(),
        ],
        None,
        &[],
    );
    if out.code != 0 {
        findings.push(DoctorFinding::info(
            "docker",
            "could not inspect Docker containers",
            out.text,
        ));
        return;
    }

    let expected = expected_container_names(ctx);
    let mut stopped = Vec::new();
    let mut unmanaged = Vec::new();
    for line in out.text.lines().filter(|line| !line.trim().is_empty()) {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        let name = parts.first().copied().unwrap_or("");
        let status = parts.get(1).copied().unwrap_or("");
        let project = parts.get(2).copied().unwrap_or("");
        if !status.starts_with("Up ") {
            stopped.push(format!("{name}: {status}"));
        } else if !expected.is_empty() && !expected.contains(name) {
            unmanaged.push(format!("{name}: {status}, project={project}"));
        }
    }

    if stopped.is_empty() {
        findings.push(DoctorFinding::ok(
            "docker",
            "no stopped Docker containers detected",
        ));
    } else {
        findings.push(DoctorFinding::warn(
            "docker",
            format!("{} stopped Docker container(s) detected", stopped.len()),
            limit_lines(&stopped, 12),
        ));
    }

    if !unmanaged.is_empty() {
        findings.push(DoctorFinding::info(
            "docker",
            format!(
                "{} running Docker container(s) are not declared in Plugroot",
                unmanaged.len()
            ),
            limit_lines(&unmanaged, 12),
        ));
    }
}

pub(crate) fn doctor_docker_networks(findings: &mut Vec<DoctorFinding>) {
    let out = run_command(
        "docker",
        &[
            "network".into(),
            "ls".into(),
            "--format".into(),
            "{{.Name}}\t{{.Driver}}".into(),
        ],
        None,
        &[],
    );
    if out.code != 0 {
        findings.push(DoctorFinding::info(
            "docker-networks",
            "could not inspect Docker networks",
            out.text,
        ));
        return;
    }

    let mut empty_custom = Vec::new();
    for line in out.text.lines().filter(|line| !line.trim().is_empty()) {
        let mut parts = line.split('\t');
        let name = parts.next().unwrap_or("");
        let driver = parts.next().unwrap_or("");
        if matches!(name, "bridge" | "host" | "none") || driver != "bridge" {
            continue;
        }
        let inspected = run_command(
            "docker",
            &[
                "network".into(),
                "inspect".into(),
                "-f".into(),
                "{{len .Containers}}".into(),
                name.into(),
            ],
            None,
            &[],
        );
        if inspected.code == 0 && inspected.text.trim() == "0" {
            empty_custom.push(name.to_string());
        }
    }

    if empty_custom.is_empty() {
        findings.push(DoctorFinding::ok(
            "docker-networks",
            "no empty custom Docker networks detected",
        ));
    } else {
        findings.push(DoctorFinding::warn(
            "docker-networks",
            format!(
                "{} empty custom Docker network(s) detected",
                empty_custom.len()
            ),
            limit_lines(&empty_custom, 12),
        ));
    }
}

pub(crate) fn doctor_docker_volumes(findings: &mut Vec<DoctorFinding>) {
    let out = run_command(
        "docker",
        &[
            "volume".into(),
            "ls".into(),
            "-qf".into(),
            "dangling=true".into(),
        ],
        None,
        &[],
    );
    if out.code != 0 {
        findings.push(DoctorFinding::info(
            "docker-volumes",
            "could not inspect unused Docker volumes",
            out.text,
        ));
        return;
    }
    let volumes: Vec<String> = out
        .text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    if volumes.is_empty() {
        findings.push(DoctorFinding::ok(
            "docker-volumes",
            "no unused Docker volumes detected",
        ));
    } else {
        findings.push(DoctorFinding::warn(
            "docker-volumes",
            format!("{} unused Docker volume(s) detected", volumes.len()),
            limit_lines(&volumes, 12),
        ));
    }
}

pub(crate) fn doctor_docker_reclaimable(findings: &mut Vec<DoctorFinding>) {
    let out = run_command(
        "docker",
        &[
            "system".into(),
            "df".into(),
            "--format".into(),
            "{{json .}}".into(),
        ],
        None,
        &[],
    );
    if out.code != 0 {
        findings.push(DoctorFinding::info(
            "docker-cache",
            "could not inspect Docker reclaimable data",
            out.text,
        ));
        return;
    }

    let mut reclaimable = Vec::new();
    for line in out.text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let item_type = value
            .get("Type")
            .and_then(|value| value.as_str())
            .unwrap_or("Docker data");
        let amount = value
            .get("Reclaimable")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if !amount.starts_with("0B") {
            reclaimable.push(format!("{item_type}: {amount} reclaimable"));
        }
    }

    if reclaimable.is_empty() {
        findings.push(DoctorFinding::ok(
            "docker-cache",
            "Docker reports no reclaimable data",
        ));
    } else {
        findings.push(DoctorFinding::info(
            "docker-cache",
            "Docker has reclaimable data",
            limit_lines(&reclaimable, 8),
        ));
    }
}
