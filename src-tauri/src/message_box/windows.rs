use std::{mem::size_of, ptr};

use windows_sys::Win32::UI::{
    Controls::{
        TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOG_BUTTON,
        TASKDIALOG_COMMON_BUTTON_FLAGS, TDCBF_CANCEL_BUTTON, TDCBF_CLOSE_BUTTON, TDCBF_NO_BUTTON,
        TDCBF_OK_BUTTON, TDCBF_RETRY_BUTTON, TDCBF_YES_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION,
        TDF_POSITION_RELATIVE_TO_WINDOW, TDF_SIZE_TO_CONTENT, TDF_USE_COMMAND_LINKS, TD_ERROR_ICON,
        TD_WARNING_ICON,
    },
    WindowsAndMessaging::{IDCANCEL, IDCLOSE, IDNO, IDOK, IDRETRY, IDYES},
};

use super::{MessageBoxKind, MessageBoxRequest};
use crate::error::AppError;

const FIRST_BUTTON_ID: i32 = 100;

fn button_id(index: usize) -> Result<i32, AppError> {
    i32::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(FIRST_BUTTON_ID))
        .ok_or_else(|| AppError::InvalidArgument("too many message box buttons".into()))
}

fn common_button(label: &str) -> Option<(i32, TASKDIALOG_COMMON_BUTTON_FLAGS)> {
    match label.to_ascii_lowercase().as_str() {
        "ok" => Some((IDOK, TDCBF_OK_BUTTON)),
        "yes" => Some((IDYES, TDCBF_YES_BUTTON)),
        "no" => Some((IDNO, TDCBF_NO_BUTTON)),
        "cancel" => Some((IDCANCEL, TDCBF_CANCEL_BUTTON)),
        "retry" => Some((IDRETRY, TDCBF_RETRY_BUTTON)),
        "close" => Some((IDCLOSE, TDCBF_CLOSE_BUTTON)),
        _ => None,
    }
}

fn map_buttons(
    labels: &[String],
    cancel_id: usize,
) -> Result<(Vec<i32>, TASKDIALOG_COMMON_BUTTON_FLAGS), AppError> {
    let common: Vec<_> = labels.iter().map(|label| common_button(label)).collect();
    let mut flags = 0;
    let ids = common
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            if let Some((id, flag)) = candidate {
                // Native common buttons cannot represent duplicate labels. Preserve
                // each original choice as a custom button in that case. IDCANCEL
                // also represents Esc, so use it only for the designated cancel choice.
                if common.iter().filter(|other| *other == candidate).count() == 1
                    && (*id != IDCANCEL || index == cancel_id)
                {
                    flags |= flag;
                    return Ok(*id);
                }
            }
            button_id(index)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((ids, flags))
}

fn selected_index(selected: i32, ids: &[i32], cancel_id: usize) -> Result<usize, AppError> {
    if selected == IDCANCEL {
        return Ok(cancel_id);
    }
    ids.iter()
        .position(|id| *id == selected)
        .ok_or_else(|| AppError::Io("native message box returned an unknown button".into()))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

pub(super) fn show(
    request: &MessageBoxRequest,
    parent: windows_sys::Win32::Foundation::HWND,
) -> Result<usize, AppError> {
    let title = wide("Tabby RS");
    let message = wide(&request.message);
    let detail = request
        .detail
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(wide);
    let (ids, common_flags) = map_buttons(&request.buttons, request.cancel_id)?;
    // Keep every UTF-16 allocation alive until the synchronous native call returns.
    let labels: Vec<_> = request.buttons.iter().map(|label| wide(label)).collect();
    let buttons: Vec<_> = labels
        .iter()
        .enumerate()
        .filter(|(index, _)| ids[*index] >= FIRST_BUTTON_ID)
        .map(|(index, label)| TASKDIALOG_BUTTON {
            nButtonID: ids[index],
            pszButtonText: label.as_ptr(),
        })
        .collect();
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: parent,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION
            | TDF_SIZE_TO_CONTENT
            | TDF_POSITION_RELATIVE_TO_WINDOW
            | if buttons.is_empty() {
                0
            } else {
                TDF_USE_COMMAND_LINKS
            },
        dwCommonButtons: common_flags,
        pszWindowTitle: title.as_ptr(),
        Anonymous1: TASKDIALOGCONFIG_0 {
            pszMainIcon: match request.kind {
                MessageBoxKind::Warning => TD_WARNING_ICON,
                MessageBoxKind::Error => TD_ERROR_ICON,
            },
        },
        pszMainInstruction: if detail.is_some() {
            message.as_ptr()
        } else {
            ptr::null()
        },
        pszContent: detail
            .as_ref()
            .map_or(message.as_ptr(), |value| value.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: if buttons.is_empty() {
            ptr::null()
        } else {
            buttons.as_ptr()
        },
        nDefaultButton: ids[request.default_id],
        // SAFETY: all remaining fields accept zero or null for unused options.
        ..unsafe { std::mem::zeroed() }
    };
    let mut selected = 0;
    // SAFETY: config and all strings/button arrays remain valid for the entire
    // synchronous call. Optional radio/verification outputs are not requested.
    let result =
        unsafe { TaskDialogIndirect(&config, &mut selected, ptr::null_mut(), ptr::null_mut()) };
    if result < 0 {
        return Err(AppError::Io(format!(
            "native message box failed (HRESULT 0x{:08X})",
            result as u32
        )));
    }
    selected_index(selected, &ids, request.cancel_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn maps_common_and_custom_buttons_to_original_indices() {
        let (ids, flags) = map_buttons(
            &labels(&["Yes", "取消", "OK", "Cancel", "Retry", "Close", "No"]),
            3,
        )
        .unwrap();
        assert_eq!(
            ids,
            vec![IDYES, 101, IDOK, IDCANCEL, IDRETRY, IDCLOSE, IDNO]
        );
        assert_eq!(
            flags,
            TDCBF_YES_BUTTON
                | TDCBF_OK_BUTTON
                | TDCBF_CANCEL_BUTTON
                | TDCBF_RETRY_BUTTON
                | TDCBF_CLOSE_BUTTON
                | TDCBF_NO_BUTTON
        );
        for (index, id) in ids.iter().enumerate() {
            assert_eq!(selected_index(*id, &ids, 3).unwrap(), index);
        }
        assert!(selected_index(0, &ids, 3).is_err());
        assert!(button_id(usize::MAX).is_err());
    }

    #[test]
    fn duplicate_common_labels_remain_independent_choices() {
        let (ids, flags) = map_buttons(&labels(&["OK", "ok", "Cancel", "cancel"]), 2).unwrap();
        assert_eq!(ids, vec![100, 101, 102, 103]);
        assert_eq!(flags, 0);
        assert_eq!(selected_index(101, &ids, 2).unwrap(), 1);
        assert_eq!(selected_index(103, &ids, 2).unwrap(), 3);
        assert_eq!(selected_index(IDCANCEL, &ids, 2).unwrap(), 2);
    }

    #[test]
    fn cancel_label_does_not_override_explicit_cancel_choice() {
        let (ids, _) = map_buttons(&labels(&["Cancel", "Keep"]), 1).unwrap();
        assert_eq!(ids, vec![100, 101]);
        assert_eq!(selected_index(100, &ids, 1).unwrap(), 0);
        assert_eq!(selected_index(IDCANCEL, &ids, 1).unwrap(), 1);
    }
}
