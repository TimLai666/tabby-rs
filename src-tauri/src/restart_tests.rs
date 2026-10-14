use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
use tauri::Manager;

fn sample_env() -> tauri::Env {
    let mut env = tauri::Env::default();
    env.args_os = vec![
        OsString::from("tabby"),
        OsString::from("--command"),
        OsString::from("ssh"),
        OsString::from("--deep-link"),
        OsString::from("tabby://open"),
        OsString::from("--opaque"),
    ];
    #[cfg(unix)]
    env.args_os.push(OsString::from_vec(vec![0xff, 0xfe]));
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        env.args_os.push(OsString::from_wide(&[0xd800]));
    }
    #[cfg(target_os = "linux")]
    {
        env.appimage = Some(OsString::from("/opt/tabby.AppImage"));
        env.appdir = Some(OsString::from("/opt"));
    }
    env
}

#[test]
fn restart_environment_keeps_only_argv0_and_preserves_other_fields() {
    let original = sample_env();
    let clean = super::restart_environment(original.clone());
    assert_eq!(clean.args_os, vec![OsString::from("tabby")]);
    #[cfg(target_os = "linux")]
    {
        assert_eq!(clean.appimage, original.appimage);
        assert_eq!(clean.appdir, original.appdir);
    }
}

#[test]
fn builder_manages_the_clean_environment_used_for_restart() {
    let clean = super::restart_environment(sample_env());
    let app = tauri::test::mock_builder()
        .manage(clean.clone())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("build mock app");
    assert_eq!(app.env().args_os, clean.args_os);
}
