use std::ffi::c_void;

use serde::Serialize;
use windows::UI::ViewManagement::UISettings;

const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
const DWMSBT_NONE: i32 = 1;
const DWMSBT_MAINWINDOW: i32 = 2;
const DWMSBT_TRANSIENTWINDOW: i32 = 3;
const SPI_GETHIGHCONTRAST: u32 = 0x0042;
const HCF_HIGHCONTRASTON: u32 = 0x0000_0001;
const SM_REMOTESESSION: i32 = 0x1000;

#[repr(C)]
struct HighContrastW {
    cb_size: u32,
    flags: u32,
    default_scheme: *mut u16,
}

#[repr(C)]
#[derive(Default)]
struct SystemPowerStatus {
    ac_line_status: u8,
    battery_flag: u8,
    battery_life_percent: u8,
    system_status_flag: u8,
    battery_life_time: u32,
    battery_full_life_time: u32,
}

#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: isize,
        attribute: u32,
        value: *const c_void,
        value_size: u32,
    ) -> i32;
    fn DwmGetWindowAttribute(
        hwnd: isize,
        attribute: u32,
        value: *mut c_void,
        value_size: u32,
    ) -> i32;
    fn DwmIsCompositionEnabled(enabled: *mut i32) -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SystemParametersInfoW(action: u32, parameter: u32, output: *mut c_void, flags: u32) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Mica,
    DesktopAcrylic,
    Solid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialStatus {
    pub selected: MaterialKind,
    pub reason: String,
    pub api_roundtrip_verified: bool,
}

pub fn apply_window_material(hwnd: isize) -> MaterialStatus {
    if let Some(reason) = restricted_environment_reason() {
        disable_backdrop(hwnd);
        return MaterialStatus {
            selected: MaterialKind::Solid,
            reason: reason.to_owned(),
            api_roundtrip_verified: true,
        };
    }

    if set_and_verify(hwnd, DWMSBT_MAINWINDOW) {
        return MaterialStatus {
            selected: MaterialKind::Mica,
            reason: "mica_api_roundtrip_verified".to_owned(),
            api_roundtrip_verified: true,
        };
    }
    if set_and_verify(hwnd, DWMSBT_TRANSIENTWINDOW) {
        return MaterialStatus {
            selected: MaterialKind::DesktopAcrylic,
            reason: "acrylic_api_roundtrip_verified".to_owned(),
            api_roundtrip_verified: true,
        };
    }
    disable_backdrop(hwnd);
    MaterialStatus {
        selected: MaterialKind::Solid,
        reason: "advanced_material_api_roundtrip_failed".to_owned(),
        api_roundtrip_verified: false,
    }
}

#[cfg(test)]
fn select_from_availability(
    restricted_reason: Option<&str>,
    mica_available: bool,
    acrylic_available: bool,
) -> MaterialKind {
    if restricted_reason.is_some() {
        MaterialKind::Solid
    } else if mica_available {
        MaterialKind::Mica
    } else if acrylic_available {
        MaterialKind::DesktopAcrylic
    } else {
        MaterialKind::Solid
    }
}

fn restricted_environment_reason() -> Option<&'static str> {
    let mut contrast = HighContrastW {
        cb_size: std::mem::size_of::<HighContrastW>() as u32,
        flags: 0,
        default_scheme: std::ptr::null_mut(),
    };
    let contrast_ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cb_size,
            &mut contrast as *mut _ as *mut c_void,
            0,
        )
    } != 0;
    if !contrast_ok {
        return Some("high_contrast_probe_unavailable");
    }
    if contrast.flags & HCF_HIGHCONTRASTON != 0 {
        return Some("high_contrast_on");
    }

    match UISettings::new().and_then(|settings| settings.AdvancedEffectsEnabled()) {
        Ok(true) => {}
        Ok(false) => return Some("advanced_effects_disabled"),
        Err(_) => return Some("advanced_effects_probe_unavailable"),
    }

    let mut power = SystemPowerStatus::default();
    if unsafe { GetSystemPowerStatus(&mut power) } == 0 {
        return Some("battery_saver_probe_unavailable");
    }
    if power.system_status_flag == 1 {
        return Some("battery_saver_on");
    }
    if unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0 {
        return Some("remote_session");
    }
    let mut composition = 0i32;
    if unsafe { DwmIsCompositionEnabled(&mut composition) } < 0 || composition == 0 {
        return Some("dwm_composition_unavailable");
    }
    None
}

fn set_and_verify(hwnd: isize, requested: i32) -> bool {
    let set_result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &requested as *const _ as *const c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };
    let mut readback = -1i32;
    let get_result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &mut readback as *mut _ as *mut c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };
    set_result >= 0 && get_result >= 0 && readback == requested
}

fn disable_backdrop(hwnd: isize) {
    let none = DWMSBT_NONE;
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &none as *const _ as *const c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_environments_always_choose_readable_solid() {
        for reason in [
            "high_contrast_on",
            "advanced_effects_disabled",
            "battery_saver_on",
            "remote_session",
        ] {
            assert_eq!(
                select_from_availability(Some(reason), true, true),
                MaterialKind::Solid
            );
        }
    }

    #[test]
    fn material_order_is_mica_then_acrylic_then_solid() {
        assert_eq!(
            select_from_availability(None, true, true),
            MaterialKind::Mica
        );
        assert_eq!(
            select_from_availability(None, false, true),
            MaterialKind::DesktopAcrylic
        );
        assert_eq!(
            select_from_availability(None, false, false),
            MaterialKind::Solid
        );
    }
}
