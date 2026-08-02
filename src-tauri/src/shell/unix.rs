use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
};

use serde_json::json;

use super::model::{DetectedShell, ShellType};

pub fn detect(warnings: &mut Vec<String>) -> Vec<DetectedShell> {
    let mut shells = Vec::new();

    #[cfg(target_os = "macos")]
    shells.push(default_macos_shell(warnings));

    #[cfg(target_os = "linux")]
    shells.push(default_linux_shell(warnings));

    shells.extend(posix_shells(warnings));
    shells
}

#[cfg(target_os = "macos")]
fn default_macos_shell(warnings: &mut Vec<String>) -> DetectedShell {
    let command = env::var("SHELL")
        .ok()
        .filter(|candidate| is_file(candidate))
        .or_else(|| is_file("/bin/zsh").then(|| "/bin/zsh".into()))
        .unwrap_or_else(|| {
            warnings.push("Could not detect the macOS login shell; using /bin/sh.".into());
            "/bin/sh".into()
        });
    let mut shell = DetectedShell::new("macos-default", "default", "OS default", command);
    shell.args = vec!["--login".into()];
    shell.hidden = true;
    shell.shell_type = Some(ShellType::Unix);
    shell.icon = Some("terminal".into());
    shell.metadata = json!({ "source": "loginShell" });
    shell
}

#[cfg(target_os = "linux")]
fn default_linux_shell(warnings: &mut Vec<String>) -> DetectedShell {
    let username = env::var("LOGNAME").or_else(|_| env::var("USER")).ok();
    let detected = username.as_deref().and_then(login_shell_from_passwd);
    let command = detected.clone().unwrap_or_else(|| "/bin/sh".into());
    if detected.is_none() {
        warnings.push("Could not detect the Linux login shell; using /bin/sh.".into());
    }

    let mut shell = DetectedShell::new("linux-default", "default", "User default", command);
    shell.hidden = detected.is_some();
    if detected.is_some() {
        shell.args = vec!["--login".into()];
    }
    shell.shell_type = Some(ShellType::Unix);
    shell.icon = Some("terminal".into());
    shell.metadata = json!({ "source": "/etc/passwd" });
    shell
}

#[cfg(target_os = "linux")]
fn login_shell_from_passwd(username: &str) -> Option<String> {
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        if name != username {
            return None;
        }
        let fields = line.split(':').collect::<Vec<_>>();
        let shell = fields.get(6)?.trim();
        (!shell.is_empty()).then(|| shell.to_owned())
    })
}

fn posix_shells(warnings: &mut Vec<String>) -> Vec<DetectedShell> {
    let shell_list = [
        PathBuf::from("/etc/shells"),
        PathBuf::from("/usr/share/defaults/etc/shells"),
    ]
    .into_iter()
    .find(|path| path.is_file());
    let Some(shell_list) = shell_list else {
        warnings.push("No POSIX shell list was found.".into());
        return Vec::new();
    };
    let contents = match fs::read_to_string(&shell_list) {
        Ok(contents) => contents,
        Err(_) => {
            warnings.push("The POSIX shell list could not be read.".into());
            return Vec::new();
        }
    };

    let mut seen = BTreeSet::new();
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| seen.insert((*line).to_owned()))
        .map(|command| {
            let name = Path::new(command)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(command)
                .to_owned();
            let mut shell =
                DetectedShell::new("posix", slugify_compat(command), name, command.to_owned());
            shell.args = vec!["-l".into()];
            shell.shell_type = Some(ShellType::Unix);
            shell.icon = Some("terminal".into());
            shell.metadata = json!({ "source": shell_list });
            shell
        })
        .collect()
}

fn is_file(path: &str) -> bool {
    Path::new(path).is_file()
}

pub fn slugify_compat(value: &str) -> String {
    let mut output = String::new();
    let mut pending_dash = false;
    for character in value.chars() {
        if character.is_alphanumeric() {
            if pending_dash && !output.is_empty() {
                output.push('-');
            }
            pending_dash = false;
            for lowered in character.to_lowercase() {
                output.push(lowered);
            }
        } else if character.is_whitespace() || character == '-' || character == '_' {
            pending_dash = true;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::slugify_compat;

    #[test]
    fn creates_stable_ids_for_posix_and_wsl_names() {
        assert_eq!(slugify_compat("/bin/bash"), "binbash");
        assert_eq!(slugify_compat("Ubuntu 22.04"), "ubuntu-2204");
        assert_eq!(slugify_compat("openSUSE-Leap-15.1"), "opensuse-leap-151");
    }
}
