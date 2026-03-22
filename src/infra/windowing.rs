use anyhow::Result;
use serde::{Deserialize, Serialize};
use window_enumerator::WindowEnumerator;

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

    let mut windows = enumerator
        .get_windows()
        .iter()
        .filter(|window| {
            !window.title.trim().is_empty()
                && window.position.width > 0
                && window.position.height > 0
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
