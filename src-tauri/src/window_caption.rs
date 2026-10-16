#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WindowCaptionButtonKind {
    Minimize,
    Maximize,
    Close,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowCaptionButton {
    pub kind: WindowCaptionButtonKind,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub hovered: bool,
    pub pressed: bool,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowCaptionState {
    pub scale_factor: f64,
    pub maximized: bool,
    pub hover_color: String,
    pub buttons: Vec<WindowCaptionButton>,
}
