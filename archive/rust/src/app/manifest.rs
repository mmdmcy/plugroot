use std::{collections::HashMap, fs, io, path::Path};

use crate::Manifest;

pub(crate) fn load_env_file(path: &Path) -> io::Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    let Ok(raw) = fs::read_to_string(path) else {
        return Ok(values);
    };

    for raw_line in raw.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        values.insert(key.trim().to_string(), value);
    }
    Ok(values)
}

pub(crate) fn load_manifest_file(
    path: &Path,
    env_values: &HashMap<String, String>,
) -> io::Result<Manifest> {
    let raw = fs::read_to_string(path)?;
    let expanded = expand_vars(&raw, env_values);
    toml::from_str(&expanded).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub(crate) fn merge_manifest(mut base: Manifest, overlay: Manifest) -> Manifest {
    if overlay.plugroot.is_some() {
        base.plugroot = overlay.plugroot;
    }
    if overlay.host.is_some() {
        base.host = overlay.host;
    }
    merge_by_id(&mut base.repo, overlay.repo, |repo| &repo.id);
    merge_by_id(&mut base.directory, overlay.directory, |directory| {
        &directory.path
    });
    merge_by_id(&mut base.file, overlay.file, |file| &file.target);
    merge_by_id(&mut base.symlink, overlay.symlink, |symlink| {
        &symlink.target
    });
    merge_by_id(&mut base.service, overlay.service, |service| &service.id);
    base
}

pub(crate) fn merge_by_id<T, F>(base: &mut Vec<T>, overlay: Vec<T>, id: F)
where
    F: Fn(&T) -> &str,
{
    for item in overlay {
        if let Some(index) = base.iter().position(|existing| id(existing) == id(&item)) {
            base[index] = item;
        } else {
            base.push(item);
        }
    }
}

pub(crate) fn expand_vars(input: &str, values: &HashMap<String, String>) -> String {
    let mut output = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            if let Some(end) = chars[index + 2..].iter().position(|ch| *ch == '}') {
                let body: String = chars[index + 2..index + 2 + end].iter().collect();
                let replacement =
                    expand_var_body(&body, values).unwrap_or_else(|| format!("${{{body}}}"));
                output.push_str(&replacement);
                index += end + 3;
                continue;
            }
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}

pub(crate) fn expand_var_body(body: &str, values: &HashMap<String, String>) -> Option<String> {
    if let Some((key, default)) = body.split_once(":-") {
        return values
            .get(key)
            .filter(|value| !value.is_empty())
            .cloned()
            .or_else(|| Some(default.to_string()));
    }
    values.get(body).cloned()
}
