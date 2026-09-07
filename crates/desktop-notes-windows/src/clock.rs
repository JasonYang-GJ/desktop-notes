use desktop_notes_core::{ErrorCode, FoundationError};
use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::GetLocalTime};

pub fn local_date_today() -> Result<String, FoundationError> {
    let mut local = SYSTEMTIME::default();
    // SAFETY: GetLocalTime always writes one caller-owned SYSTEMTIME and has no failure return.
    unsafe { GetLocalTime(&mut local) };
    if local.wYear < 1970 || !(1..=12).contains(&local.wMonth) || !(1..=31).contains(&local.wDay) {
        return Err(FoundationError::new(
            ErrorCode::InternalError,
            "The current local backup date is unavailable.",
        ));
    }
    Ok(format!(
        "{:04}-{:02}-{:02}",
        local.wYear, local.wMonth, local.wDay
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_day_has_the_frozen_iso_shape() {
        let day = local_date_today().unwrap();
        assert_eq!(day.len(), 10);
        assert_eq!(&day[4..5], "-");
        assert_eq!(&day[7..8], "-");
    }
}
