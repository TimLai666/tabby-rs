//! Geometry and pointer behavior for the fixed Electron Windows caption overlay.
//! Coordinates are physical client pixels; scale includes the display text scale.
use crate::error::AppError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptionButton {
    Minimize,
    Maximize,
    Close,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionButtonLayout {
    pub button: CaptionButton,
    pub bounds: CaptionRect,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionLayout {
    pub clip: CaptionRect,
    pub buttons: Vec<CaptionButtonLayout>,
}

#[derive(Clone, Copy, Debug)]
pub struct CaptionMetrics {
    pub client_width: u32,
    pub scale: f64,
    pub caption_height_dip: i32,
    pub border_thickness_dip: i32,
    pub maximized: bool,
    pub fullscreen: bool,
    pub minimizable: bool,
    pub maximizable: bool,
    pub closable: bool,
}

pub fn layout(metrics: CaptionMetrics) -> Result<CaptionLayout, AppError> {
    if !metrics.scale.is_finite()
        || metrics.scale <= 0.0
        || metrics.caption_height_dip < 0
        || metrics.border_thickness_dip < 0
    {
        return Err(AppError::InvalidArgument(
            "invalid Windows caption metrics".into(),
        ));
    }
    if metrics.client_width == 0 || (metrics.fullscreen && !metrics.maximized) {
        return Ok(CaptionLayout::default());
    }
    let scale = metrics.scale;
    let top = if metrics.maximized { 0.0 } else { 1.0 };
    let border = if metrics.maximized {
        0.0
    } else {
        f64::from(metrics.border_thickness_dip)
    };
    let height = if metrics.caption_height_dip < 32 {
        32.0 - top
    } else {
        f64::from(metrics.caption_height_dip) + border - top
    };
    let buttons = [
        (CaptionButton::Minimize, metrics.minimizable, 45.0, true),
        (CaptionButton::Maximize, metrics.maximizable, 46.0, true),
        (CaptionButton::Close, true, 46.0, metrics.closable),
    ];
    let preferred_width: f64 = buttons
        .iter()
        .filter(|(_, visible, _, _)| *visible)
        .map(|(_, _, width, _)| width)
        .sum();
    let right = f64::from(metrics.client_width) - top * scale;
    let left = f64::from(metrics.client_width) - preferred_width * scale;
    let clip_left = left.max(0.0).min(right.max(0.0));
    let mut x = right - preferred_width * scale;
    let buttons = buttons
        .into_iter()
        .filter(|(_, visible, _, _)| *visible)
        .map(|(button, _, width, enabled)| {
            let bounds = CaptionRect {
                x,
                y: top * scale,
                width: width * scale,
                height: height * scale,
            };
            x += bounds.width;
            CaptionButtonLayout {
                button,
                bounds,
                enabled,
            }
        })
        .collect();
    Ok(CaptionLayout {
        clip: CaptionRect {
            x: clip_left,
            y: top * scale,
            width: (right - clip_left).max(0.0),
            height: height * scale,
        },
        buttons,
    })
}

impl CaptionLayout {
    /// Hit regions include disabled Close; callers suppress its activation.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<CaptionButton> {
        let contains = |rect: CaptionRect| {
            f64::from(x) >= rect.x
                && f64::from(x) < rect.x + rect.width
                && f64::from(y) >= rect.y
                && f64::from(y) < rect.y + rect.height
        };
        if !contains(self.clip) {
            return None;
        }
        self.buttons
            .iter()
            .find(|item| contains(item.bounds))
            .map(|item| item.button)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionPointerState {
    pub hovered: Option<CaptionButton>,
    pub pressed: Option<CaptionButton>,
}

#[derive(Clone, Copy, Debug)]
pub enum CaptionPointerEvent {
    Move(Option<CaptionButton>),
    Press(CaptionButton),
    Release(Option<CaptionButton>),
    Cancel,
}

impl CaptionPointerState {
    /// A release activates only the button that owned the matching press.
    pub fn update(&mut self, event: CaptionPointerEvent) -> Option<CaptionButton> {
        match event {
            CaptionPointerEvent::Move(button) => self.hovered = button,
            CaptionPointerEvent::Press(button) => {
                self.hovered = Some(button);
                if self.pressed.is_none() {
                    self.pressed = Some(button);
                }
            }
            CaptionPointerEvent::Release(button) => {
                self.hovered = button;
                let pressed = self.pressed.take();
                return pressed.filter(|pressed| Some(*pressed) == button);
            }
            CaptionPointerEvent::Cancel => *self = Self::default(),
        }
        None
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsCaptionSnapshot {
    pub revision: u64,
    pub visible: bool,
    pub scale: f64,
    pub layout: CaptionLayout,
    pub pointer: CaptionPointerState,
    pub maximized: bool,
    pub active: bool,
}

#[cfg(any(windows, test))]
#[path = "windows_caption_events.rs"]
mod events;

#[cfg(windows)]
#[path = "windows_caption_native.rs"]
mod native;
#[cfg(windows)]
pub use native::{caption_state, prepare_caption_controls, set_caption_frame};

#[cfg(test)]
#[path = "windows_caption_tests.rs"]
mod tests;
