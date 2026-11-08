use crate::limit_lines;
use crate::nearest_git_root;
use crate::run_command;
use crate::Path;
use std::io;

pub(crate) fn git_dirty_details(path: &Path) -> io::Result<Option<String>> {
    if nearest_git_root(path).is_none() {
        return Ok(None);
    }
    let out = run_command(
        "git",
        &[
            "-C".into(),
            path.display().to_string(),
            "status".into(),
            "--porcelain=v1".into(),
        ],
        None,
        &[],
    );
    if out.code != 0 {
        return Err(io::Error::other(out.text));
    }
    let lines = out
        .text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        Ok(None)
    } else {
        Ok(Some(limit_lines(&lines, 20)))
    }
}
