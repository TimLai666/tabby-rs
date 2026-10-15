//! A diagnostic of GDI text, not a production tooltip or a parity acceptance.
use super::{check, native::Raster, paint, tooltip_pixels, Check};
use serde_json::{json, Value};
use std::{fs::OpenOptions, path::Path, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{RECT, SIZE},
    Graphics::Gdi::*,
    UI::{HiDpi::GetDpiForSystem, WindowsAndMessaging::*},
};

struct Drawing {
    screen: HDC,
    memory: HDC,
    objects: Vec<HGDIOBJ>,
}

impl Drawing {
    fn new() -> Result<Self, String> {
        let mut drawing = Self {
            screen: null_mut(),
            memory: null_mut(),
            objects: Vec::new(),
        };
        drawing.screen = unsafe { GetDC(null_mut()) };
        if drawing.screen.is_null() {
            return Err(last_error("GetDC"));
        }
        drawing.memory = unsafe { CreateCompatibleDC(drawing.screen) };
        if drawing.memory.is_null() {
            return Err(last_error("CreateCompatibleDC"));
        }
        Ok(drawing)
    }

    fn own(&mut self, object: HGDIOBJ, operation: &str) -> Result<HGDIOBJ, String> {
        if object.is_null() {
            return Err(last_error(operation));
        }
        self.objects.push(object);
        Ok(object)
    }

    fn select(&self, object: HGDIOBJ) -> Result<HGDIOBJ, String> {
        let previous = unsafe { SelectObject(self.memory, object) };
        if previous.is_null() || previous as isize == -1 {
            return Err(last_error("SelectObject"));
        }
        Ok(previous)
    }
}

impl Drop for Drawing {
    fn drop(&mut self) {
        // Delete the memory DC first, releasing every selected object even on error.
        if !self.memory.is_null() {
            unsafe { DeleteDC(self.memory) };
        }
        for object in self.objects.drain(..).rev() {
            unsafe { DeleteObject(object) };
        }
        if !self.screen.is_null() {
            unsafe { ReleaseDC(null_mut(), self.screen) };
        }
    }
}

fn last_error(operation: &str) -> String {
    format!("{operation}: {}", std::io::Error::last_os_error())
}

fn render(text: &str) -> Result<(Raster, Value), String> {
    let mut metrics: NONCLIENTMETRICSW = unsafe { std::mem::zeroed() };
    metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
    if unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&mut metrics as *mut NONCLIENTMETRICSW).cast(),
            0,
        )
    } == 0
    {
        return Err(last_error("SystemParametersInfoW(SPI_GETNONCLIENTMETRICS)"));
    }
    let logfont = metrics.lfMessageFont;
    let mut drawing = Drawing::new()?;
    let bitmap = drawing.own(
        unsafe { CreateCompatibleBitmap(drawing.screen, 1024, 128) },
        "CreateCompatibleBitmap",
    )?;
    let previous_bitmap = drawing.select(bitmap)?;
    let font = drawing.own(
        unsafe { CreateFontIndirectW(&logfont) },
        "CreateFontIndirectW",
    )?;
    drawing.select(font)?;
    let mut mapped_family = [0u16; 128];
    if unsafe {
        GetTextFaceW(
            drawing.memory,
            mapped_family.len() as i32,
            mapped_family.as_mut_ptr(),
        )
    } == 0
    {
        return Err(last_error("GetTextFaceW(message font)"));
    }
    let mut font_metrics: TEXTMETRICW = unsafe { std::mem::zeroed() };
    let text = text.encode_utf16().collect::<Vec<_>>();
    let mut extent = SIZE { cx: 0, cy: 0 };
    if unsafe { GetTextMetricsW(drawing.memory, &mut font_metrics) } == 0
        || unsafe {
            GetTextExtentPoint32W(
                drawing.memory,
                text.as_ptr(),
                text.len() as i32,
                &mut extent,
            )
        } == 0
    {
        return Err(last_error("measure message font"));
    }
    let width = extent.cx.checked_add(16).ok_or("tooltip width overflow")?;
    let height = font_metrics
        .tmHeight
        .checked_add(9)
        .ok_or("tooltip height overflow")?;
    if !(18..=800).contains(&width) || !(10..=96).contains(&height) {
        return Err(format!(
            "diagnostic text exceeds bitmap bounds: {width}x{height}"
        ));
    }
    let background = drawing.own(
        unsafe { CreateSolidBrush(0x1f1f1f) },
        "CreateSolidBrush(background)",
    )?;
    let foreground = drawing.own(
        unsafe { CreateSolidBrush(0xcacaca) },
        "CreateSolidBrush(foreground)",
    )?;
    let bounds = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    if unsafe { FillRect(drawing.memory, &bounds, background) } == 0
        || unsafe { FrameRect(drawing.memory, &bounds, foreground) } == 0
        || unsafe { SetTextColor(drawing.memory, 0xcacaca) } == CLR_INVALID
        || unsafe { SetBkColor(drawing.memory, 0x1f1f1f) } == CLR_INVALID
        || unsafe { SetBkMode(drawing.memory, TRANSPARENT as i32) } == 0
        || unsafe { TextOutW(drawing.memory, 8, 4, text.as_ptr(), text.len() as i32) } == 0
    {
        return Err(last_error("paint GDI diagnostic tooltip"));
    }
    drawing.select(previous_bitmap)?;
    if unsafe { GdiFlush() } == 0 {
        return Err(last_error("GdiFlush(GDI tooltip)"));
    }
    let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
    info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = 1024;
    info.bmiHeader.biHeight = -128;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut bytes = vec![0u8; 1024 * 128 * 4];
    if unsafe {
        GetDIBits(
            drawing.screen,
            bitmap,
            0,
            128,
            bytes.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        )
    } != 128
    {
        return Err(last_error("GetDIBits(GDI tooltip)"));
    }
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let offset = (y * 1024 + x) * 4;
            rgba.extend_from_slice(&[bytes[offset + 2], bytes[offset + 1], bytes[offset], 255]);
        }
    }
    let family_end = logfont
        .lfFaceName
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(logfont.lfFaceName.len());
    let mapped_end = mapped_family
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(mapped_family.len());
    let font = json!({"family":String::from_utf16_lossy(&mapped_family[..mapped_end]),
        "requestedFamily":String::from_utf16_lossy(&logfont.lfFaceName[..family_end]),
        "logicalHeight":logfont.lfHeight,"weight":logfont.lfWeight,"italic":logfont.lfItalic,
        "quality":logfont.lfQuality,"systemDpi":unsafe{GetDpiForSystem()},
        "textExtent":[extent.cx,extent.cy],"height":font_metrics.tmHeight,
        "ascent":font_metrics.tmAscent,"descent":font_metrics.tmDescent,"internalLeading":font_metrics.tmInternalLeading});
    Ok((
        Raster {
            width: width as u32,
            height: height as u32,
            rgba,
        },
        font,
    ))
}

fn probe_one(output: &Path, label: &str, reference_name: &str) -> Result<Value, String> {
    let original = paint::read_png(&output.join(reference_name))?;
    let [x, y, width, height] =
        tooltip_pixels::find(original.width, original.height, &original.rgba)?;
    let (candidate, font) = render(label)?;
    let candidate_name = format!("gdi-tooltip-{}.png", label.to_ascii_lowercase());
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join(&candidate_name))
        .map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(file, candidate.width, candidate.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&candidate.rgba)
        .map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    let comparison = paint::compare(
        &candidate,
        paint::Rect {
            x: 0,
            y: 0,
            width: candidate.width,
            height: candidate.height,
        },
        &original,
        paint::Rect {
            x,
            y,
            width,
            height,
        },
    );
    let matches = comparison
        .as_ref()
        .is_ok_and(|result| result["differentPixels"] == 0);
    Ok(
        json!({"diagnosticOnly":true,"productionTooltipVerified":false,"label":label,"font":font,
        "reference":reference_name,"referenceTooltip":[x,y,width,height],"candidate":candidate_name,
        "candidateDimensions":[candidate.width,candidate.height],"pixelsMatch":matches,"comparison":comparison}),
    )
}

pub fn probe(output: &Path) -> Vec<Check> {
    [
        ("Maximize", "electron-thin-hover-max-tooltip.png"),
        ("Restore", "electron-thin-hover-restore-tooltip.png"),
    ]
    .into_iter()
    .map(|(label, reference)| {
        let result = probe_one(output, label, reference);
        let matched = result
            .as_ref()
            .is_ok_and(|detail| detail["pixelsMatch"] == true);
        check(
            &format!("gdi-tooltip-{}-pixels", label.to_ascii_lowercase()),
            matched,
            json!(result),
        )
    })
    .collect()
}
