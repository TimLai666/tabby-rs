use super::{MessageBoxKind, MessageBoxRequest};
use crate::error::AppError;
use block2::RcBlock;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSAlert, NSAlertStyle, NSEvent, NSEventMask, NSEventModifierFlags, NSWindow};
use objc2_foundation::NSString;
use std::{cell::RefCell, ptr::NonNull};

pub(super) fn show(
    request: &MessageBoxRequest,
    parent: &NSWindow,
    complete: impl FnOnce(Result<usize, AppError>) + 'static,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        complete(Err(AppError::Io(
            "native message box requires the main thread".into(),
        )));
        return;
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&request.message));
    alert.setInformativeText(&NSString::from_str(request.detail.as_deref().unwrap_or("")));
    alert.setAlertStyle(match request.kind {
        MessageBoxKind::Warning => NSAlertStyle::Warning,
        MessageBoxKind::Error => NSAlertStyle::Critical,
    });
    let buttons: Vec<_> = request
        .buttons
        .iter()
        .map(|label| alert.addButtonWithTitle(&NSString::from_str(label)))
        .collect();
    for (index, button) in buttons.iter().enumerate() {
        button.setKeyEquivalent(&NSString::from_str(if index == request.default_id {
            "\r"
        } else {
            ""
        }));
        button.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
    }
    let alert_window = alert.window();
    alert_window.makeFirstResponder(Some(&buttons[request.default_id]));

    // A separate cancellation handler also covers default_id == cancel_id.
    // NSButton has only one key equivalent, which is reserved for Return above.
    let owner = parent.retain();
    let cancel_code = 1000 + request.cancel_id as isize;
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        // SAFETY: AppKit supplies a live event for the duration of this callback.
        let key = unsafe { event.as_ref() };
        let characters = key
            .charactersIgnoringModifiers()
            .map(|text| text.to_string());
        let is_cancel = is_cancel_key(
            key.keyCode(),
            key.modifierFlags().contains(NSEventModifierFlags::Command),
            characters.as_deref(),
        );
        if is_cancel
            && key
                .window(mtm)
                .is_some_and(|window| std::ptr::eq(&*window, &*alert_window))
            && owner
                .attachedSheet()
                .is_some_and(|window| std::ptr::eq(&*window, &*alert_window))
        {
            owner.endSheet_returnCode(&alert_window, cancel_code);
            std::ptr::null_mut()
        } else {
            event.as_ptr()
        }
    });
    // SAFETY: the block returns the original live event or null, and is retained
    // until the sheet ends. Its monitor is removed by the completion callback.
    let Some(monitor) = (unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler)
    }) else {
        complete(Err(AppError::Io(
            "could not register message box cancellation".into(),
        )));
        return;
    };
    let count = request.buttons.len();
    let cancel_id = request.cancel_id;
    let complete = RefCell::new(Some(complete));
    let retained_alert = alert.clone();
    let completion = RcBlock::new(move |response| {
        // Keep the alert alive until AppKit finishes the sheet.
        let _ = &retained_alert;
        // SAFETY: monitor is the token returned by addLocalMonitor above.
        unsafe { NSEvent::removeMonitor(&monitor) };
        if let Some(complete) = complete.borrow_mut().take() {
            complete(selected_index(response, count, cancel_id));
        }
    });
    alert.beginSheetModalForWindow_completionHandler(parent, Some(&completion));
}

fn selected_index(response: isize, count: usize, cancel_id: usize) -> Result<usize, AppError> {
    if response < 1000 {
        return Ok(cancel_id);
    }
    usize::try_from(response - 1000)
        .ok()
        .filter(|index| *index < count)
        .ok_or_else(|| AppError::Io("native message box returned an unknown button".into()))
}

fn is_cancel_key(code: u16, command: bool, characters: Option<&str>) -> bool {
    // Zhuyin can report ㄡ for the physical period key even with Command held.
    // Preserve character-based matching for alternate Latin keyboard layouts.
    code == 53
        || command
            && (characters == Some(".")
                || code == 47 && characters.is_some_and(|text| !text.is_ascii()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_supports_input_methods_without_overriding_latin_layouts() {
        assert!(is_cancel_key(53, false, Some("\x1b")));
        assert!(is_cancel_key(14, true, Some(".")));
        assert!(is_cancel_key(47, true, Some("ㄡ")));
        assert!(!is_cancel_key(47, true, Some("v")));
        assert!(!is_cancel_key(47, false, Some("ㄡ")));
    }
    #[test]
    fn maps_all_buttons_without_using_the_default_or_label() {
        for index in 0..5 {
            assert_eq!(selected_index(1000 + index, 5, 2).unwrap(), index as usize);
        }
        assert_eq!(selected_index(-1000, 5, 2).unwrap(), 2);
        assert!(selected_index(1005, 5, 2).is_err());
    }
}
