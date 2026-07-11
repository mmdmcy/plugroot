use crate::command_text;

pub(crate) fn user_systemd_prefix(user: &str) -> Option<Vec<String>> {
    let uid = command_text(&["id", "-u", user])?;
    let home = user_home(user).unwrap_or_else(|| format!("/home/{user}"));
    let env_args = vec![
        "env".into(),
        format!("XDG_RUNTIME_DIR=/run/user/{uid}"),
        format!("DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{uid}/bus"),
        format!("HOME={home}"),
    ];
    if command_text(&["id", "-u"]).as_deref() == Some(uid.as_str()) {
        return Some(env_args);
    }
    let mut args = vec!["runuser".into(), "-u".into(), user.into(), "--".into()];
    args.extend(env_args);
    Some(args)
}

pub(crate) fn user_home(user: &str) -> Option<String> {
    command_text(&["getent", "passwd", user]).and_then(|line| {
        line.split(':')
            .nth(5)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

pub(crate) fn is_root() -> bool {
    command_text(&["id", "-u"]).as_deref() == Some("0")
}
