use crate::{
    error::AppError,
    transfer::{
        manager::{TransferDescriptor, TransferManager},
        safe_path::safe_file_name,
    },
};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tempfile::TempDir;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableFile {
    pub id: String,
    pub path: String,
    pub transfer: TransferDescriptor,
}
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEditEvent {
    pub id: String,
    pub event: String,
    pub message: Option<String>,
}
struct EditContext {
    directory: Option<TempDir>,
    path: PathBuf,
    ready: bool,
    active: Arc<AtomicBool>,
    watcher: Option<RecommendedWatcher>,
}
impl Drop for EditContext {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
    }
}
#[derive(Default)]
pub struct FileEditManager {
    contexts: Mutex<HashMap<String, EditContext>>,
}
impl FileEditManager {
    pub fn prepare(
        &self,
        name: &str,
        mode: u32,
        size: u64,
        transfers: &TransferManager,
    ) -> Result<EditableFile, AppError> {
        let directory = tempfile::Builder::new().prefix("tabby-edit-").tempdir()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        let root = directory.path().canonicalize()?;
        let path = root.join(safe_file_name(name));
        let id = root.file_name().unwrap().to_string_lossy().into_owned();
        let path_text = path
            .to_str()
            .ok_or_else(|| AppError::InvalidArgument("editor path is not UTF-8".into()))?
            .to_owned();
        let transfer = transfers.open_download(name, mode, Some(size), &path_text, None, None)?;
        self.contexts.lock().unwrap().insert(
            id.clone(),
            EditContext {
                directory: Some(directory),
                path,
                ready: false,
                active: Arc::new(AtomicBool::new(true)),
                watcher: None,
            },
        );
        Ok(EditableFile {
            id,
            path: path_text,
            transfer,
        })
    }
    pub fn ready(&self, id: &str) -> Result<(), AppError> {
        let mut contexts = self.contexts.lock().unwrap();
        let context = contexts
            .get_mut(id)
            .ok_or_else(|| AppError::NotFound("editor workspace".into()))?;
        if !std::fs::symlink_metadata(&context.path)?.is_file() {
            return Err(AppError::InvalidArgument(
                "editor copy is not a regular file".into(),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&context.path, std::fs::Permissions::from_mode(0o700))?;
        }
        context.ready = true;
        if let Some(directory) = context.directory.take() {
            let _ = directory.keep();
        }
        Ok(())
    }
    pub fn watch(
        &self,
        id: &str,
        callback: impl Fn(FileEditEvent) + Send + Sync + 'static,
    ) -> Result<(), AppError> {
        let mut contexts = self.contexts.lock().unwrap();
        let context = contexts
            .get_mut(id)
            .ok_or_else(|| AppError::NotFound("editor workspace".into()))?;
        if !context.ready {
            return Err(AppError::InvalidArgument("editor copy is not ready".into()));
        }
        if context.watcher.is_some() {
            return Ok(());
        }
        let target = context.path.clone();
        let identity = file_identity(&target);
        let active = context.active.clone();
        let event_id = id.to_owned();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                if !active.load(Ordering::SeqCst) {
                    return;
                }
                let (event, message) = match result {
                    Ok(event) => {
                        if !event.paths.iter().any(|path| path == &target) {
                            return;
                        }
                        let kind = match event.kind {
                            EventKind::Modify(_)
                            | EventKind::Create(_)
                            | EventKind::Remove(_)
                            | EventKind::Any => {
                                if file_identity(&target) == identity {
                                    "change"
                                } else {
                                    "rename"
                                }
                            }
                            _ => return,
                        };
                        (kind, None)
                    }
                    Err(error) => ("error", Some(error.to_string())),
                };
                callback(FileEditEvent {
                    id: event_id.clone(),
                    event: event.into(),
                    message,
                });
            })
            .map_err(|error| AppError::Io(error.to_string()))?;
        // Editors can replace the inode; parent watching still reports the original file's rename.
        watcher
            .watch(context.path.parent().unwrap(), RecursiveMode::NonRecursive)
            .map_err(|error| AppError::Io(error.to_string()))?;
        context.watcher = Some(watcher);
        Ok(())
    }
    pub fn stop(&self, id: &str) -> Result<(), AppError> {
        let removed = self.contexts.lock().unwrap().remove(id);
        drop(removed);
        Ok(())
    }
}
#[cfg(unix)]
fn file_identity(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path)
        .ok()
        .map(|metadata| (metadata.dev(), metadata.ino()))
}
#[cfg(windows)]
fn file_identity(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::windows::fs::MetadataExt;
    std::fs::metadata(path)
        .ok()
        .map(|metadata| (metadata.creation_time(), 0))
}
#[cfg(test)]
#[path = "file_edit_tests.rs"]
mod tests;
