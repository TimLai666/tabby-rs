use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use serde_json::json;
use winreg::{
    enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY},
    RegKey,
};

use super::{
    model::{DetectedShell, ShellType},
    unix::slugify_compat,
};

pub fn detect(identification: Option<&str>, warnings: &mut Vec<String>) -> Vec<DetectedShell> {
    let stock = stock_shells(identification);
    let powershell_core = powershell_core_shells(identification);
    let wsl = wsl_shells(warnings);

    let mut result = Vec::new();
    if let Some(candidate) = powershell_core
        .last()
        .or_else(|| wsl.last())
        .or_else(|| stock.last())
    {
        let mut default = candidate.clone();
        default.provider_id = "windows-default".into();
        default.id = "default".into();
        default.name = format!("OS default ({})", candidate.name);
        default.hidden = true;
        result.push(default);
    }

    result.extend(stock);
    result.extend(powershell_core);
    result.extend(cmder_shells());
    result.extend(cygwin32_shells());
    result.extend(cygwin64_shells());
    result.extend(git_bash_shells(identification));
    result.extend(msys2_shells());
    result.extend(wsl);
    result.extend(visual_studio_shells(warnings));
    result
}

fn stock_shells(identification: Option<&str>) -> Vec<DetectedShell> {
    let mut clink = DetectedShell::new("windows-stock", "clink", "CMD (clink)", "cmd.exe");
    clink.args = vec![
        "/k".into(),
        clink_path().to_string_lossy().into_owned(),
        "inject".into(),
    ];
    clink.env.insert("WT_SESSION".into(), "0".into());
    clink.icon = Some("clink".into());
    clink.shell_type = Some(ShellType::Cmd);

    let mut cmd = DetectedShell::new("windows-stock", "cmd", "CMD (stock)", "cmd.exe");
    cmd.icon = Some("cmd".into());
    cmd.shell_type = Some(ShellType::Cmd);

    let mut powershell = DetectedShell::new(
        "windows-stock",
        "powershell",
        "PowerShell",
        powershell_path(),
    );
    powershell.args = vec!["-nologo".into()];
    powershell.env = identification_environment(identification);
    powershell.icon = Some("powershell".into());
    powershell.shell_type = Some(ShellType::Powershell);

    vec![clink, cmd, powershell]
}

fn powershell_core_shells(identification: Option<&str>) -> Vec<DetectedShell> {
    let path = registry_string(
        HKEY_LOCAL_MACHINE,
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\pwsh.exe",
        "",
        KEY_WOW64_64KEY,
    )
    .or_else(|| {
        registry_string(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\pwsh.exe",
            "",
            KEY_WOW64_32KEY,
        )
    })
    .or_else(|| standard_pwsh_paths().into_iter().find(|path| path.is_file()))
    .and_then(path_string);

    let Some(path) = path else {
        return Vec::new();
    };
    let mut shell = DetectedShell::new(
        "powershell-core",
        "powershell-core",
        "PowerShell Core",
        path,
    );
    shell.args = vec!["-nologo".into()];
    shell.env = identification_environment(identification);
    shell.icon = Some("powershell-core".into());
    shell.shell_type = Some(ShellType::Powershell);
    vec![shell]
}

fn cmder_shells() -> Vec<DetectedShell> {
    let Some(root) = env::var_os("CMDER_ROOT").map(PathBuf::from) else {
        return Vec::new();
    };
    let init = root.join("vendor").join("init.bat");
    let profile = root.join("vendor").join("profile.ps1");
    let (Some(init), Some(profile)) = (path_string(init), path_string(profile)) else {
        return Vec::new();
    };

    let mut cmd = DetectedShell::new("cmder", "cmder", "Cmder", "cmd.exe");
    cmd.args = vec!["/k".into(), init];
    cmd.env.insert("TERM".into(), "cygwin".into());
    cmd.icon = Some("cmder".into());
    cmd.shell_type = Some(ShellType::Cmd);

    let mut powershell = DetectedShell::new(
        "cmder",
        "cmderps",
        "Cmder PowerShell",
        "powershell.exe",
    );
    powershell.args = vec![
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-nologo".into(),
        "-noprofile".into(),
        "-noexit".into(),
        "-command".into(),
        format!("Invoke-Expression '. ''{profile}'''"),
    ];
    powershell.icon = Some("cmder-powershell".into());
    powershell.shell_type = Some(ShellType::Powershell);
    vec![cmd, powershell]
}

fn cygwin32_shells() -> Vec<DetectedShell> {
    cygwin_shell(
        "cygwin32",
        "Cygwin (32 bit)",
        r"Software\WOW6432Node\Cygwin\setup",
        KEY_WOW64_32KEY,
    )
    .into_iter()
    .collect()
}

fn cygwin64_shells() -> Vec<DetectedShell> {
    cygwin_shell(
        "cygwin64",
        "Cygwin",
        r"Software\Cygwin\setup",
        KEY_WOW64_64KEY,
    )
    .into_iter()
    .collect()
}

fn cygwin_shell(id: &str, name: &str, key: &str, flags: u32) -> Option<DetectedShell> {
    let root = registry_string(HKEY_LOCAL_MACHINE, key, "rootdir", flags)?;
    let command = path_string(root.join("bin").join("bash.exe"))?;
    let mut shell = DetectedShell::new("cygwin", id, name, command);
    shell.args = vec!["--login".into(), "-i".into()];
    shell.env.insert("TERM".into(), "cygwin".into());
    shell.icon = Some("cygwin".into());
    shell.shell_type = Some(ShellType::Unix);
    Some(shell)
}

fn git_bash_shells(identification: Option<&str>) -> Vec<DetectedShell> {
    let root = registry_string(
        HKEY_LOCAL_MACHINE,
        r"Software\GitForWindows",
        "InstallPath",
        KEY_WOW64_64KEY,
    )
    .or_else(|| {
        registry_string(
            HKEY_CURRENT_USER,
            r"Software\GitForWindows",
            "InstallPath",
            KEY_READ,
        )
    });
    let Some(command) = root
        .map(|root| root.join("bin").join("bash.exe"))
        .and_then(path_string)
    else {
        return Vec::new();
    };
    let mut shell = DetectedShell::new("git-bash", "git-bash", "Git Bash", command);
    shell.args = vec!["--login".into(), "-i".into()];
    shell.env = identification_environment(identification);
    shell.icon = Some("git-bash".into());
    shell.shell_type = Some(ShellType::Unix);
    vec![shell]
}

fn msys2_shells() -> Vec<DetectedShell> {
    let system_root = env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let Some(parent) = system_root.parent() else {
        return Vec::new();
    };
    let root = parent.join("msys64");
    if !root.is_dir() {
        return Vec::new();
    }
    let Some(command) = path_string(root.join("msys2_shell.cmd")) else {
        return Vec::new();
    };
    let cwd = env::var_os("USERNAME")
        .map(|username| root.join("home").join(username))
        .filter(|path| path.is_dir())
        .and_then(path_string);

    ["msys", "mingw64", "clang64", "ucrt64"]
        .into_iter()
        .map(|environment| {
            let mut shell = DetectedShell::new(
                "msys2",
                format!("msys2-{environment}"),
                format!("MSYS2 ({})", environment.to_ascii_uppercase()),
                command.clone(),
            );
            shell.args = vec![
                "-defterm".into(),
                "-here".into(),
                "-no-start".into(),
                format!("-{environment}"),
            ];
            shell.cwd = cwd.clone();
            shell.icon = Some("msys2".into());
            shell.shell_type = Some(ShellType::Unix);
            shell
        })
        .collect()
}

fn wsl_shells(warnings: &mut Vec<String>) -> Vec<DetectedShell> {
    let windows = env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let wsl = windows.join("System32").join("wsl.exe");
    let bash = windows.join("System32").join("bash.exe");
    if !wsl.is_file() && !bash.is_file() {
        return Vec::new();
    }

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(lxss) = hkcu.open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Lxss",
        KEY_READ,
    ) else {
        return bash
            .is_file()
            .then(|| legacy_wsl_shell(&bash))
            .into_iter()
            .collect();
    };

    let default_id = lxss.get_value::<String, _>("DefaultDistribution").ok();
    let mut distributions = Vec::new();
    for subkey in lxss.enum_keys().filter_map(Result::ok) {
        let Ok(key) = lxss.open_subkey_with_flags(&subkey, KEY_READ) else {
            continue;
        };
        let Ok(name) = key.get_value::<String, _>("DistributionName") else {
            continue;
        };
        let base_path = key.get_value::<String, _>("BasePath").ok();
        let flags = key.get_value::<u32, _>("Flags").unwrap_or_default();
        distributions.push((subkey, name, base_path, flags));
    }
    distributions.sort_by(|left, right| left.1.to_lowercase().cmp(&right.1.to_lowercase()));

    let Some(wsl_command) = path_string(&wsl) else {
        warnings.push("The WSL executable path is not valid Unicode.".into());
        return Vec::new();
    };
    let mut result = Vec::new();
    if let Some((_, name, _, _)) = distributions
        .iter()
        .find(|(id, _, _, _)| Some(id) == default_id.as_ref())
    {
        let mut shell = base_wsl_shell("wsl", "WSL / Default distro", &wsl_command, name);
        shell.metadata = json!({ "defaultDistribution": name });
        result.push(shell);
    }

    for (_, name, base_path, flags) in distributions {
        let mut shell = base_wsl_shell(
            format!("wsl-{}", slugify_compat(&name)),
            format!("WSL / {name}"),
            &wsl_command,
            &name,
        );
        shell.args = vec!["-d".into(), name.clone()];
        let version = if flags & 8 != 0 { 2 } else { 1 };
        shell.fs_base = if version == 2 {
            Some(format!(r"\\wsl$\{name}"))
        } else {
            base_path.map(|base| format!(r"{base}\rootfs"))
        };
        shell.metadata = json!({ "distribution": name, "version": version });
        result.push(shell);
    }

    if result.is_empty() && bash.is_file() {
        result.push(legacy_wsl_shell(&bash));
    }
    result
}

fn legacy_wsl_shell(bash: &Path) -> DetectedShell {
    let command = path_string(bash).unwrap_or_else(|| "bash.exe".into());
    let mut shell = base_wsl_shell("wsl", "WSL / Bash on Windows", &command, "Linux");
    shell.metadata = json!({ "legacy": true });
    shell
}

fn base_wsl_shell(
    id: impl Into<String>,
    name: impl Into<String>,
    command: &str,
    distribution: &str,
) -> DetectedShell {
    let mut shell = DetectedShell::new("wsl", id, name, command.to_owned());
    shell.env.insert("TERM".into(), "xterm-color".into());
    shell.env.insert("COLORTERM".into(), "truecolor".into());
    shell.icon = Some(wsl_icon(distribution).into());
    shell.shell_type = Some(ShellType::Unix);
    shell
}

fn visual_studio_shells(warnings: &mut Vec<String>) -> Vec<DetectedShell> {
    let mut roots = Vec::new();
    if let Some(program_files_x86) = env::var_os("ProgramFiles(x86)") {
        roots.push(PathBuf::from(program_files_x86).join("Microsoft Visual Studio"));
    }
    if let Some(program_files) = env::var_os("ProgramFiles") {
        roots.push(PathBuf::from(program_files).join("Microsoft Visual Studio"));
    }

    let mut result = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let Some(version) = entry.file_name().to_str().map(str::to_owned) else {
                warnings.push("A Visual Studio version path is not valid Unicode.".into());
                continue;
            };
            let bat = entry
                .path()
                .join("Community")
                .join("Common7")
                .join("Tools")
                .join("VsDevCmd.bat");
            let Some(bat) = path_string(bat) else {
                continue;
            };
            let mut shell = DetectedShell::new(
                "visual-studio",
                format!("vs-cmd-{version}"),
                format!("Developer Prompt for VS {version}"),
                "cmd.exe",
            );
            shell.args = vec!["/k".into(), bat];
            shell.icon = Some(format!("vs{version}"));
            shell.shell_type = Some(ShellType::Cmd);
            result.push(shell);
        }
    }
    result.sort_by(|left, right| left.id.cmp(&right.id));
    result.dedup_by(|left, right| left.id == right.id);
    result
}

fn identification_environment(identification: Option<&str>) -> BTreeMap<String, String> {
    match identification {
        Some("wt") => BTreeMap::from([("WT_SESSION".into(), "0".into())]),
        Some("cygwin") => BTreeMap::from([("TERM".into(), "cygwin".into())]),
        _ => BTreeMap::new(),
    }
}

fn powershell_path() -> String {
    standard_pwsh_paths()
        .into_iter()
        .find(|path| path.is_file())
        .or_else(|| find_on_path("pwsh.exe"))
        .or_else(|| find_on_path("powershell.exe"))
        .and_then(path_string)
        .unwrap_or_else(|| "powershell.exe".into())
}

fn standard_pwsh_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(profile) = env::var_os("USERPROFILE") {
        paths.push(
            PathBuf::from(profile)
                .join("AppData")
                .join("Local")
                .join("Microsoft")
                .join("WindowsApps")
                .join("pwsh.exe"),
        );
    }
    if let Some(program_files) = env::var_os("ProgramFiles") {
        paths.push(PathBuf::from(program_files).join("PowerShell").join("7").join("pwsh.exe"));
    }
    if let Some(program_files) = env::var_os("ProgramFiles(x86)") {
        paths.push(PathBuf::from(program_files).join("PowerShell").join("7").join("pwsh.exe"));
    }
    if let Some(system_root) = env::var_os("SystemRoot") {
        let system_root = PathBuf::from(system_root);
        paths.push(
            system_root
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe"),
        );
        paths.push(system_root.join("System32").join("powershell.exe"));
    }
    paths
}

fn clink_path() -> PathBuf {
    let executable = env::current_exe().unwrap_or_else(|_| PathBuf::from("tabby-rs.exe"));
    let architecture = match env::consts::ARCH {
        "x86_64" => "x64",
        "x86" => "ia32",
        other => other,
    };
    executable
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("resources")
        .join("extras")
        .join("clink")
        .join(format!("clink_{architecture}.exe"))
}

fn registry_string(root: isize, path: &str, name: &str, flags: u32) -> Option<PathBuf> {
    let root = RegKey::predef(root);
    let key = root.open_subkey_with_flags(path, KEY_READ | flags).ok()?;
    key.get_value::<String, _>(name).ok().map(PathBuf::from)
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn path_string(path: impl AsRef<Path>) -> Option<String> {
    let path = path.as_ref();
    if !path.is_file() {
        return None;
    }
    path.to_str().map(str::to_owned)
}

fn wsl_icon(name: &str) -> &'static str {
    match name {
        "Alpine" => "alpine",
        "Debian" => "debian",
        "kali-linux" => "kali",
        "SLES-12" | "openSUSE-Leap-15-1" => "suse",
        "Ubuntu-16.04" | "Ubuntu-18.04" | "Ubuntu-22.04" | "Ubuntu" => "ubuntu",
        "AlmaLinux-8" => "alma",
        "OracleLinux_7_9" | "OracleLinux_8_5" => "oracle-linux",
        "openEuler" => "open-euler",
        "docker-desktop" | "docker-desktop-data" => "docker",
        _ => "linux",
    }
}
