use std::cell::Cell;

use objc2::{
    define_class, msg_send, rc::Retained, sel, DefinedClass, MainThreadMarker, MainThreadOnly,
    Message,
};
use objc2_app_kit::{NSMenu, NSMenuItem, NSWindow};
use objc2_foundation::{NSLocale, NSObject, NSObjectProtocol, NSString};

#[path = "labels.rs"]
mod labels;

use super::{MenuItemKind, MenuItemRequest};
use crate::error::AppError;

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "TabbyRSContextMenuTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = Cell<Option<u32>>]
    struct SelectionTarget;

    impl SelectionTarget {
        #[unsafe(method(selectItem:))]
        fn select_item(&self, sender: &NSMenuItem) {
            if sender.isEnabled() {
                self.ivars().set(u32::try_from(sender.tag()).ok().filter(|id| *id != 0));
            }
        }
    }
);

impl SelectionTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let allocated = Self::alloc(mtm).set_ivars(Cell::new(None));
        // SAFETY: NSObject's initializer is called on our allocated subclass.
        unsafe { msg_send![super(allocated), init] }
    }
}

// Electron 38 uses Chromium's FixUpWindowsStyleLabel for both title and subtitle.
fn native_label(label: &str) -> String {
    let chars: Vec<char> = label.chars().collect();
    let mut result = String::with_capacity(label.len());
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '(' if chars.get(index + 1) == Some(&'&')
                && chars.get(index + 3) == Some(&')')
                && chars[index + 2].len_utf16() == 1 =>
            {
                index += 4;
            }
            '&' => {
                if chars.get(index + 1) == Some(&'&') {
                    result.push('&');
                    index += 1;
                }
                index += 1;
            }
            '.' if chars.get(index + 1) == Some(&'.') && chars.get(index + 2) == Some(&'.') => {
                result.push('…');
                index += 3;
            }
            character => {
                result.push(character);
                index += 1;
            }
        }
    }
    result
}

fn build_menu(
    items: &[MenuItemRequest],
    target: &SelectionTarget,
    mtm: MainThreadMarker,
) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::new());
    menu.setAutoenablesItems(false);
    for item in items {
        if item.kind == MenuItemKind::Separator {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        }
        let title = NSString::from_str(&native_label(&item.label));
        // SAFETY: The selector matches SelectionTarget's selectItem: signature.
        let native = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &title,
                Some(sel!(selectItem:)),
                &NSString::new(),
            )
        };
        native.setEnabled(item.enabled);
        native.setTag(item.id as isize);
        // Electron's Cocoa controller uses the same NSControlStateValue for radio
        // and checkbox items. Application callbacks update their persistent state.
        native.setState(
            if matches!(item.kind, MenuItemKind::Checkbox | MenuItemKind::Radio) && item.checked {
                1
            } else {
                0
            },
        );
        if let Some(subtitle) = item.sublabel.as_deref().filter(|text| !text.is_empty()) {
            // subtitle was introduced in macOS 14.4; Electron omits it before then.
            if native.respondsToSelector(sel!(setSubtitle:)) {
                native.setSubtitle(Some(&NSString::from_str(&native_label(subtitle))));
            }
        }
        if let Some(children) = &item.submenu {
            let submenu = build_menu(children, target, mtm);
            if children.is_empty() {
                let locales = NSLocale::preferredLanguages()
                    .iter()
                    .map(|locale| locale.to_string())
                    .collect::<Vec<_>>();
                // Electron adds a disabled localized item to otherwise empty submenus.
                let empty = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(labels::empty_submenu_label(&locales)),
                        None,
                        &NSString::new(),
                    )
                };
                empty.setEnabled(false);
                submenu.addItem(&empty);
            }
            submenu.setTitle(&title);
            native.setSubmenu(Some(&submenu));
            // SAFETY: A submenu has no selectable action or target.
            unsafe {
                native.setAction(None);
            }
        } else {
            // SAFETY: show retains target until tracking and all actions finish.
            unsafe {
                native.setTarget(Some(target));
            }
        }
        menu.addItem(&native);
    }
    menu
}

pub(super) fn show(items: &[MenuItemRequest], parent: &NSWindow) -> Result<Option<u32>, AppError> {
    if items.is_empty() {
        return Ok(None);
    }
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| AppError::Io("context menu requires the main thread".into()))?;
    // The nested menu event loop may process a request to close the window.
    let parent = parent.retain();
    let target = SelectionTarget::new(mtm);
    let menu = build_menu(items, &target, mtm);
    let view = parent
        .contentView()
        .ok_or_else(|| AppError::Io("context menu window has no content view".into()))?;
    let mut position = view.convertPoint_fromView(parent.mouseLocationOutsideOfEventStream(), None);
    // Match Electron 38 MenuMac::PopupOnUI's cursor positioning before AppKit
    // performs its own screen-edge and submenu adjustments.
    if let Some(screen) = parent.screen() {
        let frame = parent.frame();
        let visible = screen.visibleFrame();
        let size = menu.size();
        let distance_from_bottom = frame.origin.y + position.y - size.height - visible.origin.y;
        if distance_from_bottom < 0.0 {
            position.y = position.y - distance_from_bottom + 4.0;
        }
        if frame.origin.x + position.x + size.width > visible.origin.x + visible.size.width {
            position.x -= size.width;
        }
    }
    menu.popUpMenuPositioningItem_atLocation_inView(None, position, Some(&view));
    Ok(target.ivars().get())
}

#[cfg(test)]
mod tests {
    use super::native_label;

    #[test]
    fn converts_electron_macos_menu_labels() {
        assert_eq!(native_label("Save && &Close..."), "Save & Close…");
        assert_eq!(native_label("顏色(&C)"), "顏色");
        assert_eq!(native_label("SSH → 主機"), "SSH → 主機");
        assert_eq!(native_label("(&) &"), "() ");
        assert_eq!(native_label("(&🐈)"), "(🐈)");
    }
}
