//! Reports a panel main's working/idle/blocked state to herdr (OSC 7501).
//!
//! herdr only draws a terminal as an agent when it receives this sequence from
//! the program's tty. Panel mains run inside tmux, so the sequence is written
//! straight to each attached tmux client tty, and the last state is kept in a
//! pane option so a re-attached panel can replay it.
use std::{
    fs,
    io::Write,
    os::unix::fs::{FileTypeExt, OpenOptionsExt},
    process::{Command, Stdio},
};

/// herdr `app` name for a panel harness; `None` for harnesses herdr cannot name.
pub fn app_for(harness: &str) -> Option<&'static str> {
    match harness {
        "claude" => Some("claude-code"),
        "codex" => Some("codex"),
        "agy" => Some("antigravity"),
        "pi" => Some("pi"),
        _ => None,
    }
}

/// OSC 7501 payload for `state` (`working`, `idle`, `blocked`) and `app`.
pub fn sequence(state: &str, app: &str) -> Vec<u8> {
    format!("\x1b]7501;state={state}:app={app}\x1b\\").into_bytes()
}

fn tmux(args: &[&str]) -> Option<String> {
    let out = Command::new("tmux")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Write `data` to every tty in `ttys`; only `/dev/` character devices qualify.
pub fn write_ttys<'a>(ttys: impl IntoIterator<Item = &'a str>, data: &[u8]) {
    for tty in ttys {
        if !tty.starts_with("/dev/") {
            continue;
        }
        if let Ok(mut f) = fs::OpenOptions::new()
            .write(true)
            .custom_flags(
                rustix::fs::OFlags::NOFOLLOW.bits() as i32
                    | rustix::fs::OFlags::NONBLOCK.bits() as i32,
            )
            .open(tty)
            && f.metadata().is_ok_and(|m| m.file_type().is_char_device())
        {
            let _ = f.write_all(data);
        }
    }
}

/// Record and broadcast `state` for the calling panel main. A no-op outside a
/// panel main pane, so hook callers need no gate of their own.
pub fn report(state: &str) {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let Some(app) = app_for(&env("OMS_PANEL_HARNESS")) else {
        return;
    };
    let (session, pane) = (env("OMS_PANEL_SESSION"), env("TMUX_PANE"));
    if env("OMS_HARNESS_CHILD") == "1" || session.is_empty() || pane.is_empty() {
        return;
    }
    let _ = tmux(&[
        "set-option",
        "-p",
        "-t",
        &pane,
        "@oms_agent_state",
        &format!("{state}:{app}"),
    ]);
    let target = format!("={session}");
    if let Some(list) = tmux(&["list-clients", "-t", &target, "-F", "#{client_tty}"]) {
        write_ttys(list.lines(), &sequence(state, app));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_names_map_to_herdr_apps() {
        assert_eq!(app_for("claude"), Some("claude-code"));
        assert_eq!(app_for("codex"), Some("codex"));
        assert_eq!(app_for("agy"), Some("antigravity"));
        assert_eq!(app_for("pi"), Some("pi"));
        assert_eq!(app_for("unknown"), None);
    }

    #[test]
    fn sequence_is_osc_7501() {
        assert_eq!(
            sequence("working", "pi"),
            b"\x1b]7501;state=working:app=pi\x1b\\"
        );
    }

    #[test]
    fn write_ttys_ignores_non_device_paths() {
        let dir = std::env::temp_dir().join(format!("oms-herdr-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("not-a-tty");
        fs::write(&file, b"keep").unwrap();
        write_ttys([file.to_str().unwrap(), "relative", "/dev/null"], b"x");
        assert_eq!(fs::read(&file).unwrap(), b"keep");
        fs::remove_dir_all(&dir).unwrap();
    }
}
