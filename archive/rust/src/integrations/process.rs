use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub(crate) struct CmdOutput {
    pub(crate) code: i32,
    pub(crate) text: String,
}

pub(crate) fn ensure_success(out: CmdOutput) -> io::Result<()> {
    if out.code == 0 {
        return Ok(());
    }
    Err(io::Error::other(out.text))
}

pub(crate) fn run_command(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    envs: &[(&str, &str)],
) -> CmdOutput {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    for (key, value) in envs {
        cmd.env(key, value);
    }
    collect_output(cmd)
}

pub(crate) fn run_command_with_env_values(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    envs: &HashMap<String, String>,
) -> CmdOutput {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.envs(envs);
    collect_output(cmd)
}

pub(crate) fn run_prefixed_command(prefix: &[String], program: &str, args: &[String]) -> CmdOutput {
    if prefix.is_empty() {
        return run_command(program, args, None, &[]);
    }
    let mut full_args = prefix[1..].to_vec();
    full_args.push(program.into());
    full_args.extend(args.iter().cloned());
    run_command(&prefix[0], &full_args, None, &[])
}

pub(crate) fn print_cmd_output(out: &CmdOutput) {
    if !out.text.is_empty() {
        println!("{}", out.text);
    }
}

pub(crate) fn command_text(args: &[&str]) -> Option<String> {
    let output = Command::new(args[0]).args(&args[1..]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn collect_output(mut cmd: Command) -> CmdOutput {
    match cmd.output() {
        Ok(output) => {
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            CmdOutput {
                code: output.status.code().unwrap_or(1),
                text: text.trim().to_string(),
            }
        }
        Err(err) => CmdOutput {
            code: 127,
            text: err.to_string(),
        },
    }
}
