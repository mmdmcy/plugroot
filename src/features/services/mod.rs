use crate::load_env_file;
use crate::print_cmd_output;
use crate::resolve_path;
use crate::run_command;
use crate::run_command_with_env_values;
use crate::run_prefixed_command;
use crate::user_systemd_prefix;
use crate::CmdOutput;
use crate::Context;
use crate::HashMap;
use crate::Service;
use std::io;

pub(crate) fn cmd_action(ctx: &Context, action: &str, args: &[String]) -> io::Result<u8> {
    let Some(target) = args.first() else {
        eprintln!("missing service target");
        return Ok(2);
    };
    let services = ctx.services_for_target(target);
    if services.is_empty() {
        eprintln!("unknown service: {target}");
        return Ok(2);
    }

    let mut final_code = 0u8;
    for service in services {
        if !has_control(&service, action) {
            eprintln!("{} does not allow `{action}`", service.id);
            final_code = 2;
            continue;
        }
        println!("==> {} {}", service.id, action);
        let out = if action == "logs" {
            logs_for(ctx, &service)
        } else {
            run_fixed_action(ctx, &service, action)
        };
        print_cmd_output(&out);
        if out.code != 0 {
            final_code = out.code.min(255) as u8;
        }
    }
    Ok(final_code)
}

pub(crate) fn has_control(service: &Service, action: &str) -> bool {
    service
        .controls
        .as_ref()
        .map(|controls| controls.iter().any(|control| control == action))
        .unwrap_or(false)
}

pub(crate) fn compose_args(ctx: &Context, service: &Service) -> Option<Vec<String>> {
    let path = service.path.as_ref()?;
    let dir = resolve_path(&ctx.root, path);
    let mut args = vec!["compose".into()];
    args.extend([
        "--project-directory".into(),
        dir.display().to_string(),
        "-f".into(),
        dir.join("compose.yaml").display().to_string(),
    ]);
    Some(args)
}

pub(crate) fn compose_env_values(ctx: &Context, service: &Service) -> HashMap<String, String> {
    let mut values = ctx.env_values.clone();
    if let Some(env_file) = service
        .env_file
        .as_deref()
        .map(|path| resolve_path(&ctx.root, path))
    {
        if let Ok(extra) = load_env_file(&env_file) {
            for (key, value) in extra {
                values.insert(key, value);
            }
        }
    }
    values
}

pub(crate) fn logs_for(ctx: &Context, service: &Service) -> CmdOutput {
    match service.kind.as_str() {
        "compose" => {
            let Some(mut args) = compose_args(ctx, service) else {
                return CmdOutput {
                    code: 2,
                    text: "compose service is missing path".into(),
                };
            };
            args.extend([
                "logs".into(),
                "--tail".into(),
                "200".into(),
                "--no-color".into(),
            ]);
            let env_values = compose_env_values(ctx, service);
            run_command_with_env_values("docker", &args, None, &env_values)
        }
        "systemd" => run_command(
            "journalctl",
            &[
                "-u".into(),
                service.unit.clone().unwrap_or_default(),
                "-n".into(),
                "200".into(),
                "--no-pager".into(),
            ],
            None,
            &[],
        ),
        "user-systemd" => {
            let Some(prefix) = user_systemd_prefix(service.user.as_deref().unwrap_or("")) else {
                return CmdOutput {
                    code: 2,
                    text: "user not found".into(),
                };
            };
            run_prefixed_command(
                &prefix,
                "journalctl",
                &[
                    "--user".into(),
                    "-u".into(),
                    service.unit.clone().unwrap_or_default(),
                    "-n".into(),
                    "200".into(),
                    "--no-pager".into(),
                ],
            )
        }
        _ => CmdOutput {
            code: 2,
            text: "logs unavailable for this service type".into(),
        },
    }
}

pub(crate) fn run_fixed_action(ctx: &Context, service: &Service, action: &str) -> CmdOutput {
    match service.kind.as_str() {
        "compose" => {
            let Some(mut args) = compose_args(ctx, service) else {
                return CmdOutput {
                    code: 2,
                    text: "compose service is missing path".into(),
                };
            };
            let compose_action = match action {
                "start" => vec!["up".into(), "-d".into()],
                "stop" => vec!["down".into()],
                "restart" => vec!["restart".into()],
                _ => {
                    return CmdOutput {
                        code: 2,
                        text: format!("unsupported action {action}"),
                    }
                }
            };
            args.extend(compose_action);
            let env_values = compose_env_values(ctx, service);
            run_command_with_env_values("docker", &args, None, &env_values)
        }
        "systemd" => {
            let systemd_action = match action {
                "start" => "start",
                "stop" => "stop",
                "restart" => "restart",
                _ => {
                    return CmdOutput {
                        code: 2,
                        text: format!("unsupported action {action}"),
                    }
                }
            };
            run_command(
                "systemctl",
                &[
                    systemd_action.into(),
                    service.unit.clone().unwrap_or_default(),
                ],
                None,
                &[],
            )
        }
        "user-systemd" => {
            let Some(prefix) = user_systemd_prefix(service.user.as_deref().unwrap_or("")) else {
                return CmdOutput {
                    code: 2,
                    text: "user not found".into(),
                };
            };
            let systemd_action = match action {
                "start" => "start",
                "stop" => "stop",
                "restart" => "restart",
                _ => {
                    return CmdOutput {
                        code: 2,
                        text: format!("unsupported action {action}"),
                    }
                }
            };
            run_prefixed_command(
                &prefix,
                "systemctl",
                &[
                    "--user".into(),
                    systemd_action.into(),
                    service.unit.clone().unwrap_or_default(),
                ],
            )
        }
        _ => CmdOutput {
            code: 2,
            text: "this service type has no fixed action".into(),
        },
    }
}
