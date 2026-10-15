use std::path::{Path, PathBuf};

use crate::error::AppError;

pub(crate) fn data_directory(portable_root: Option<&Path>) -> Option<PathBuf> {
    portable_root.map(|root| root.join("data").join("webview"))
}

pub(crate) fn prepare(
    config: &mut tauri::Config,
    portable_root: Option<&Path>,
) -> Result<Option<PathBuf>, AppError> {
    let Some(directory) = data_directory(portable_root) else {
        return Ok(None);
    };
    std::fs::create_dir_all(&directory)?;
    for window in &mut config.app.windows {
        window.data_directory = Some(directory.clone());
    }
    Ok(Some(directory))
}

#[cfg(target_os = "macos")]
pub(crate) fn install_macos(directory: &Path) -> Result<(), AppError> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    extern "C" {
        fn tabby_rs_install_portable_webview_storage(
            directory: *const std::ffi::c_char,
            error: *mut std::ffi::c_char,
            capacity: usize,
        ) -> std::ffi::c_int;
    }

    let directory = CString::new(directory.as_os_str().as_bytes())
        .map_err(|_| AppError::Io("portable WebView storage path contains a null byte".into()))?;
    let mut error = [0_u8; 1024];
    // Called on the startup main thread, before Tauri creates any WebView.
    let installed = unsafe {
        tabby_rs_install_portable_webview_storage(
            directory.as_ptr(),
            error.as_mut_ptr().cast(),
            error.len(),
        )
    };
    if installed == 1 {
        Ok(())
    } else {
        let length = error
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(error.len());
        Err(AppError::Io(format!(
            "unable to prepare portable WebView storage: {}",
            String::from_utf8_lossy(&error[..length])
        )))
    }
}

#[cfg(test)]
#[path = "webview_storage_tests.rs"]
mod tests;
