use crate::error::AppError;
use std::collections::HashSet;

#[cfg(target_os = "macos")]
mod macos;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum MenuItemKind {
    #[default]
    Normal,
    Separator,
    Submenu,
    Checkbox,
    Radio,
}

#[derive(Debug, serde::Deserialize)]
pub struct MenuItemRequest {
    id: u32,
    #[serde(rename = "type", default)]
    kind: MenuItemKind,
    #[serde(default)]
    label: String,
    sublabel: Option<String>,
    enabled: bool,
    #[serde(default)]
    checked: bool,
    submenu: Option<Vec<MenuItemRequest>>,
}

#[derive(serde::Deserialize)]
pub struct ContextMenuRequest {
    items: Vec<MenuItemRequest>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextMenuResult {
    selected_id: Option<u32>,
}

fn validate(items: &[MenuItemRequest]) -> Result<(), AppError> {
    fn visit(items: &[MenuItemRequest], ids: &mut HashSet<u32>) -> Result<(), AppError> {
        for item in items {
            if item.id == 0 || !ids.insert(item.id) {
                return Err(AppError::InvalidArgument(
                    "menu IDs must be unique and nonzero".into(),
                ));
            }
            if item.label.contains('\0')
                || item
                    .sublabel
                    .as_deref()
                    .is_some_and(|value| value.contains('\0'))
            {
                return Err(AppError::InvalidArgument(
                    "menu text contains a NUL character".into(),
                ));
            }
            if let Some(children) = &item.submenu {
                visit(children, ids)?;
            }
        }
        Ok(())
    }
    visit(items, &mut HashSet::new())
}

fn validate_selection(items: &[MenuItemRequest], selected_id: Option<u32>) -> Result<(), AppError> {
    fn selectable(items: &[MenuItemRequest], id: u32) -> bool {
        items.iter().any(|item| {
            item.enabled
                && if let Some(children) = &item.submenu {
                    selectable(children, id)
                } else {
                    item.id == id
                        && !matches!(item.kind, MenuItemKind::Separator | MenuItemKind::Submenu)
                }
        })
    }
    match selected_id {
        None => Ok(()),
        Some(id) if selectable(items, id) => Ok(()),
        Some(_) => Err(AppError::Io(
            "native menu returned an unavailable action".into(),
        )),
    }
}

fn normalize(items: &mut Vec<MenuItemRequest>) {
    // Electron removes leading, trailing, and adjacent separators before popup.
    let mut previous_separator = true;
    items.retain(|item| {
        let separator = item.kind == MenuItemKind::Separator;
        let keep = !separator || !previous_separator;
        previous_separator = separator;
        keep
    });
    if items
        .last()
        .is_some_and(|item| item.kind == MenuItemKind::Separator)
    {
        items.pop();
    }
    for group in items.split_mut(|item| item.kind == MenuItemKind::Separator) {
        if !group
            .iter()
            .any(|item| item.kind == MenuItemKind::Radio && item.checked)
        {
            if let Some(first) = group
                .iter_mut()
                .find(|item| item.kind == MenuItemKind::Radio)
            {
                first.checked = true;
            }
        }
        for item in group {
            if let Some(children) = &mut item.submenu {
                normalize(children);
            }
        }
    }
}

#[tauri::command]
pub async fn menu_popup(
    window: tauri::WebviewWindow,
    mut request: ContextMenuRequest,
) -> Result<ContextMenuResult, AppError> {
    validate(&request.items)?;
    normalize(&mut request.items);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window
        .clone()
        .run_on_main_thread(move || {
            #[cfg(target_os = "macos")]
            let result = window
                .ns_window()
                .map_err(|error| AppError::Io(error.to_string()))
                .and_then(|parent| {
                    // SAFETY: Tauri owns this NSWindow; the popup is synchronous
                    // and this closure executes on its main thread.
                    macos::show(&request.items, unsafe {
                        &*parent.cast::<objc2_app_kit::NSWindow>()
                    })
                });
            #[cfg(not(target_os = "macos"))]
            let result: Result<Option<u32>, AppError> =
                Err(AppError::Unsupported("native context menu".into()));
            let result = result.and_then(|selected_id| {
                validate_selection(&request.items, selected_id)?;
                Ok(ContextMenuResult { selected_id })
            });
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    receiver
        .await
        .map_err(|error| AppError::Io(error.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<MenuItemRequest> {
        serde_json::from_value(serde_json::json!([
            {"id":1,"label":"Rename","enabled":true},
            {"id":2,"type":"separator","enabled":true},
            {"id":3,"type":"submenu","label":"Disabled","enabled":false,"submenu":[
                {"id":4,"label":"Child","enabled":true}
            ]},
            {"id":5,"type":"submenu","label":"Color","sublabel":"Blue","enabled":true,"submenu":[
                {"id":6,"type":"radio","label":"Blue","enabled":true,"checked":true}
            ]}
        ]))
        .unwrap()
    }

    #[test]
    fn accepts_menu_labels_and_eligible_actions() {
        let items = items();
        validate(&items).unwrap();
        for selected in [None, Some(1), Some(6)] {
            validate_selection(&items, selected).unwrap();
        }
    }

    #[test]
    fn rejects_disabled_descendants_separators_parents_and_unknown_ids() {
        let items = items();
        for id in [0, 2, 3, 4, 5, 100] {
            assert!(validate_selection(&items, Some(id)).is_err());
        }
    }

    #[test]
    fn rejects_ambiguous_ids_and_native_text_truncation() {
        let mut menu = items();
        menu[0].id = 0;
        assert!(validate(&menu).is_err());
        menu[0].id = 6;
        assert!(validate(&menu).is_err());
        menu[0].id = 1;
        menu[0].label = "visible\0hidden".into();
        assert!(validate(&menu).is_err());
        menu[0].label = "Visible".into();
        menu[0].sublabel = Some("visible\0hidden".into());
        assert!(validate(&menu).is_err());
    }

    #[test]
    fn normalizes_separator_edges_and_unselected_radio_groups() {
        let mut menu: Vec<MenuItemRequest> = serde_json::from_value(serde_json::json!([
            {"id":1,"type":"separator","enabled":true},
            {"id":2,"type":"radio","enabled":true},
            {"id":3,"type":"normal","enabled":true},
            {"id":4,"type":"radio","enabled":true},
            {"id":5,"type":"separator","enabled":true},
            {"id":6,"type":"separator","enabled":true},
            {"id":7,"type":"radio","enabled":true},
            {"id":8,"type":"radio","enabled":true,"checked":true},
            {"id":9,"type":"separator","enabled":true}
        ]))
        .unwrap();
        normalize(&mut menu);
        assert_eq!(
            menu.iter().map(|item| item.id).collect::<Vec<_>>(),
            [2, 3, 4, 5, 7, 8]
        );
        assert_eq!(
            menu.iter()
                .filter(|item| item.checked)
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [2, 8]
        );
        let mut nested = items();
        nested[3].submenu.as_mut().unwrap()[0].checked = false;
        normalize(&mut nested);
        assert!(nested[3].submenu.as_ref().unwrap()[0].checked);
    }
}
