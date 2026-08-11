use serialport::{available_ports, SerialPortType};

use crate::error::AppError;

use super::model::SerialPortInfo;

pub fn list_ports() -> Result<Vec<SerialPortInfo>, AppError> {
    available_ports()
        .map(|ports| ports.into_iter().map(port_info).collect())
        .map_err(|error| AppError::Io(error.to_string()))
}

pub fn stable_id(info: &SerialPortInfo) -> String {
    if let (Some(vendor_id), Some(product_id), Some(serial_number)) = (
        info.vendor_id,
        info.product_id,
        info.serial_number.as_deref(),
    ) {
        if !serial_number.is_empty() {
            return format!("usb:{vendor_id:04x}:{product_id:04x}:{serial_number}");
        }
    }
    format!("path:{}", info.path)
}

fn port_info(port: serialport::SerialPortInfo) -> SerialPortInfo {
    let (port_type, vendor_id, product_id, serial_number, manufacturer, product) =
        match port.port_type {
            SerialPortType::UsbPort(usb) => (
                "usb".into(),
                Some(usb.vid),
                Some(usb.pid),
                usb.serial_number,
                usb.manufacturer,
                usb.product,
            ),
            SerialPortType::BluetoothPort => ("bluetooth".into(), None, None, None, None, None),
            SerialPortType::PciPort => ("pci".into(), None, None, None, None, None),
            SerialPortType::Unknown => ("unknown".into(), None, None, None, None, None),
        };
    let display_name = [manufacturer.as_deref(), product.as_deref()]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut info = SerialPortInfo {
        id: String::new(),
        display_name: if display_name.is_empty() {
            port.port_name.clone()
        } else {
            display_name
        },
        path: port.port_name,
        port_type,
        vendor_id,
        product_id,
        serial_number,
        manufacturer,
    };
    info.id = stable_id(&info);
    info
}

pub fn path_for_stable_id(stable: &str) -> Option<String> {
    list_ports()
        .ok()?
        .into_iter()
        .find(|port| port.id == stable)
        .map(|port| port.path)
}
