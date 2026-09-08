use super::{MessageBoxKind, MessageBoxRequest};
use crate::error::AppError;
use gtk::prelude::*;
use gtk::{ButtonsType, DialogFlags, MessageDialog, MessageType, ResponseType};

pub(super) fn show(
    request: &MessageBoxRequest,
    parent: &gtk::ApplicationWindow,
) -> Result<usize, AppError> {
    // GTK reserves negative response IDs; gtk-rs represents custom IDs as u16.
    if request.buttons.len() > usize::from(u16::MAX) + 1 {
        return Err(AppError::InvalidArgument(
            "too many message box buttons".into(),
        ));
    }

    let kind = match request.kind {
        MessageBoxKind::Warning => MessageType::Warning,
        MessageBoxKind::Error => MessageType::Error,
    };
    let dialog = MessageDialog::new(
        Some(parent),
        DialogFlags::MODAL,
        kind,
        ButtonsType::None,
        &request.message,
    );
    dialog.set_secondary_text(request.detail.as_deref());
    for (index, label) in request.buttons.iter().enumerate() {
        // Literal labels preserve underscores and duplicate labels. Selection
        // is identified by the response ID, never by the displayed text.
        let button = gtk::Button::with_label(label);
        button.set_can_default(true);
        dialog.add_action_widget(&button, ResponseType::Other(index as u16));
    }
    dialog.set_default_response(ResponseType::Other(request.default_id as u16));
    dialog.show_all();
    if let Some(button) = dialog.widget_for_response(ResponseType::Other(request.default_id as u16))
    {
        button.grab_focus();
    }
    let response = dialog.run();
    let selected = selected_index(response, request.buttons.len(), request.cancel_id);
    // SAFETY: run() has returned, and no widget state is accessed after destroy.
    unsafe { dialog.destroy() };
    Ok(selected)
}

fn selected_index(response: ResponseType, button_count: usize, cancel_id: usize) -> usize {
    match response {
        ResponseType::Other(index) if usize::from(index) < button_count => usize::from(index),
        // GTK reports DeleteEvent for window close and usually Escape. Treat
        // every non-button response as cancellation, including external close.
        _ => cancel_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_each_button_index_without_label_matching() {
        for index in 0..5 {
            assert_eq!(
                selected_index(ResponseType::Other(index), 5, 3),
                index as usize
            );
        }
    }

    #[test]
    fn dismissal_and_unknown_responses_use_the_configured_cancel_button() {
        for response in [
            ResponseType::DeleteEvent,
            ResponseType::Cancel,
            ResponseType::Close,
            ResponseType::None,
            ResponseType::Other(5),
        ] {
            assert_eq!(selected_index(response, 5, 3), 3);
        }
    }
}
