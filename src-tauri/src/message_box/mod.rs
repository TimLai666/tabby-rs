use crate::error::AppError;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageBoxKind {
    Warning,
    Error,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageBoxOptions {
    #[serde(rename = "type")]
    kind: MessageBoxKind,
    message: String,
    detail: Option<String>,
    buttons: Vec<String>,
    default_id: Option<usize>,
    cancel_id: Option<usize>,
}

pub struct MessageBoxRequest {
    kind: MessageBoxKind,
    message: String,
    detail: Option<String>,
    buttons: Vec<String>,
    default_id: usize,
    cancel_id: usize,
}

#[derive(serde::Serialize)]
pub struct MessageBoxResult {
    response: usize,
}

impl MessageBoxOptions {
    fn into_request(self) -> Result<MessageBoxRequest, AppError> {
        let buttons = if self.buttons.is_empty() {
            vec!["OK".into()]
        } else {
            self.buttons
        };
        let default_id = self.default_id.unwrap_or(0);
        // Electron uses the first Cancel/No label, otherwise button zero.
        let cancel_id = self.cancel_id.unwrap_or_else(|| {
            buttons
                .iter()
                .position(|label| matches!(label.to_lowercase().as_str(), "cancel" | "no"))
                .unwrap_or(0)
        });
        if default_id >= buttons.len() || cancel_id >= buttons.len() {
            return Err(AppError::InvalidArgument(
                "message box button index is out of range".into(),
            ));
        }
        if std::iter::once(self.message.as_str())
            .chain(self.detail.as_deref())
            .chain(buttons.iter().map(String::as_str))
            .any(|value| value.contains('\0'))
        {
            return Err(AppError::InvalidArgument(
                "message box text contains a NUL character".into(),
            ));
        }
        Ok(MessageBoxRequest {
            kind: self.kind,
            message: self.message,
            detail: self.detail,
            buttons,
            default_id,
            cancel_id,
        })
    }
}

#[tauri::command]
pub async fn dialog_message(
    window: tauri::WebviewWindow,
    request: MessageBoxOptions,
) -> Result<MessageBoxResult, AppError> {
    let request = request.into_request()?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window
        .clone()
        .run_on_main_thread(move || {
            #[cfg(target_os = "macos")]
            {
                match window.ns_window() {
                    Ok(parent) => {
                        // SAFETY: Tauri owns this NSWindow and this closure runs on
                        // its main thread. show retains it for the sheet lifetime.
                        let parent = unsafe { &*parent.cast::<objc2_app_kit::NSWindow>() };
                        macos::show(&request, parent, move |result| {
                            let _ = sender.send(result);
                        });
                    }
                    Err(error) => {
                        let _ = sender.send(Err(AppError::Io(error.to_string())));
                    }
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                #[cfg(target_os = "linux")]
                let result = window
                    .gtk_window()
                    .map_err(|error| AppError::Io(error.to_string()))
                    .and_then(|parent| linux::show(&request, &parent));
                #[cfg(windows)]
                let result = window
                    .hwnd()
                    .map_err(|error| AppError::Io(error.to_string()))
                    .and_then(|parent| windows::show(&request, parent.0 as _));
                #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
                let result: Result<usize, AppError> =
                    Err(AppError::Unsupported("native message box".into()));
                let _ = sender.send(result);
            }
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    let response = receiver
        .await
        .map_err(|error| AppError::Io(error.to_string()))??;
    Ok(MessageBoxResult { response })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: serde_json::Value) -> Result<MessageBoxRequest, AppError> {
        serde_json::from_value::<MessageBoxOptions>(value)?.into_request()
    }

    #[test]
    fn preserves_choices_and_distinguishes_default_from_cancel() {
        let value = request(serde_json::json!({
            "type": "warning", "message": "Delete?", "detail": "台灣",
            "buttons": ["Delete", "Keep", "Other"], "defaultId": 1, "cancelId": 2
        }))
        .unwrap();
        assert_eq!(value.buttons, ["Delete", "Keep", "Other"]);
        assert_eq!(value.default_id, 1);
        assert_eq!(value.cancel_id, 2);
        assert_eq!(value.detail.as_deref(), Some("台灣"));
    }

    #[test]
    fn infers_electron_cancel_button_and_uses_zero_otherwise() {
        for (buttons, expected) in [
            (vec!["Retry", "Cancel", "Quit"], 1),
            (vec!["Yes", "No"], 1),
            (vec!["Retry", "Erase", "Quit"], 0),
        ] {
            let value = request(serde_json::json!({
                "type": "error", "message": "Test", "buttons": buttons
            }))
            .unwrap();
            assert_eq!(value.cancel_id, expected);
            assert_eq!(value.default_id, 0);
        }
    }

    #[test]
    fn supports_empty_and_duplicate_button_labels() {
        let empty = request(serde_json::json!({
            "type": "warning", "message": "Notice", "buttons": []
        }))
        .unwrap();
        assert_eq!(empty.buttons, ["OK"]);
        let duplicate = request(serde_json::json!({
            "type": "warning", "message": "Notice", "buttons": ["Same", "Same"], "defaultId": 1
        }))
        .unwrap();
        assert_eq!(duplicate.buttons, ["Same", "Same"]);
        assert_eq!(duplicate.default_id, 1);
    }

    #[test]
    fn rejects_invalid_indices_and_native_string_truncation() {
        for patch in [
            serde_json::json!({ "defaultId": 3 }),
            serde_json::json!({ "cancelId": 3 }),
            serde_json::json!({ "defaultId": -1 }),
            serde_json::json!({ "message": "hidden\0suffix" }),
            serde_json::json!({ "buttons": ["hidden\0suffix"] }),
        ] {
            let mut value = serde_json::json!({
                "type": "warning", "message": "Test", "buttons": ["OK"]
            });
            value
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(request(value).is_err());
        }
    }
}
