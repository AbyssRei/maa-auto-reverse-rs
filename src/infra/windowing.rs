use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use window_enumerator::WindowEnumerator;

#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
    pub class_name: String,
    pub process_name: String,
    pub width: i32,
    pub height: i32,
}

impl WindowInfo {
    pub fn display_label(&self) -> String {
        if self.process_name.is_empty() {
            self.title.clone()
        } else {
            format!("{} ({})", self.title, self.process_name)
        }
    }
}

pub fn list_windows() -> Result<Vec<WindowInfo>> {
    let mut enumerator = WindowEnumerator::new();
    enumerator.enumerate_all_windows()?;
    let mut seen_titles = HashSet::new();

    let mut windows = enumerator
        .get_windows()
        .iter()
        .filter(|window| {
            let title = window.title.trim();
            !title.is_empty()
                && is_window_candidate(window.hwnd, window.position.width, window.position.height)
                && seen_titles.insert(title.to_string())
        })
        .map(|window| WindowInfo {
            hwnd: window.hwnd,
            title: window.title.clone(),
            class_name: window.class_name.clone(),
            process_name: window.process_name.clone(),
            width: window.position.width,
            height: window.position.height,
        })
        .collect::<Vec<_>>();

    windows.sort_by_key(|window| window.title.clone());
    windows.sort_by_key(|window| {
        if window.title.contains("明日方舟") {
            0
        } else if window.title.contains("模拟器") {
            1
        } else {
            2
        }
    });

    Ok(windows)
}

pub fn find_window_hwnd(title: &str) -> Result<Option<isize>> {
    Ok(list_windows()?
        .into_iter()
        .find(|window| window.title.contains(title))
        .map(|window| window.hwnd))
}

#[cfg(windows)]
fn is_window_candidate(hwnd: isize, width: i32, height: i32) -> bool {
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    if hwnd.0.is_null() {
        return false;
    }

    let visible = unsafe { IsWindowVisible(hwnd).as_bool() };
    if !visible {
        return false;
    }

    let minimized = unsafe { IsIconic(hwnd).as_bool() };
    minimized || (width > 0 && height > 0)
}

#[cfg(not(windows))]
fn is_window_candidate(_hwnd: isize, width: i32, height: i32) -> bool {
    width > 0 && height > 0
}
