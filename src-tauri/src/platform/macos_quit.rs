use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, ProtocolObject, Sel};
use objc2::{sel, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSApplicationTerminateReply};

use crate::error::AppError;

struct QuitAdapter {
    delegate: Retained<ProtocolObject<dyn NSApplicationDelegate>>,
    class: &'static AnyClass,
    app: tauri::AppHandle,
}

thread_local! {
    static ADAPTER: RefCell<Option<QuitAdapter>> = const { RefCell::new(None) };
}

extern "C" fn should_terminate(
    _delegate: &AnyObject,
    _selector: Sel,
    _application: &NSApplication,
) -> NSApplicationTerminateReply {
    // Release the borrow before posting the ordinary Tauri exit request.
    let app = ADAPTER.with(|slot| slot.borrow().as_ref().map(|adapter| adapter.app.clone()));
    if let Some(app) = app {
        app.exit(0);
    }
    NSApplicationTerminateReply::TerminateCancel
}

pub(crate) fn install(app: &tauri::AppHandle) -> Result<(), AppError> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| AppError::Io("quit adapter requires the main thread".into()))?;
    let application = NSApplication::sharedApplication(mtm);
    let delegate = application
        .delegate()
        .ok_or_else(|| AppError::Io("quit adapter requires an application delegate".into()))?;
    let object: &AnyObject = (&*delegate).as_ref();
    let current_class = object.class();

    if let Some(result) = ADAPTER.with(|slot| {
        slot.borrow().as_ref().map(|adapter| {
            let installed: &AnyObject = (&*adapter.delegate).as_ref();
            if std::ptr::eq(installed, object) && std::ptr::eq(adapter.class, current_class) {
                Ok(())
            } else {
                Err(AppError::Io(
                    "application delegate changed after quit adapter installation".into(),
                ))
            }
        })
    }) {
        return result;
    }
    let selector = sel!(applicationShouldTerminate:);
    if current_class.instance_method(selector).is_some() {
        return Err(AppError::Io(
            "application delegate already handles termination".into(),
        ));
    }
    let mut builder = ClassBuilder::new(c"TabbyRSApplicationDelegate", current_class)
        .ok_or_else(|| AppError::Io("quit adapter class is already registered".into()))?;
    // SAFETY: AppKit's selector takes NSApplication and returns NSUInteger via
    // NSApplicationTerminateReply. The original class has no implementation.
    unsafe {
        builder.add_method(selector, should_terminate as extern "C" fn(_, _, _) -> _);
    }
    let class = builder.register();
    if class.instance_size() != current_class.instance_size() {
        return Err(AppError::Io(
            "quit adapter changed the delegate layout".into(),
        ));
    }
    // SAFETY: This is a direct subclass with no new ivars and only the ABI-checked
    // termination method. Installation runs on the main thread without yielding;
    // the original delegate, ivars and all inherited Tao callbacks stay intact.
    let previous = unsafe { AnyObject::set_class(object, class) };
    assert!(std::ptr::eq(previous, current_class));
    ADAPTER.with(|slot| {
        *slot.borrow_mut() = Some(QuitAdapter {
            delegate: delegate.clone(),
            class,
            app: app.clone(),
        });
    });
    // Refresh AppKit's optional-method cache. Keep the weak delegate strongly
    // retained across the reset, then restore the exact same object.
    application.setDelegate(None);
    application.setDelegate(Some(&delegate));
    Ok(())
}
