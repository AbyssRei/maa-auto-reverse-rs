use anyhow::{Result, anyhow};

#[cfg(windows)]
fn directinput_scancode(keycode: i32) -> Option<u16> {
    match keycode {
        0x58 | 0x78 => Some(0x2D), // X / x
        0x44 | 0x64 => Some(0x20), // D / d
        _ => None,
    }
}

#[cfg(windows)]
const PYDIRECTINPUT_KEY_HOLD_MS: u64 = 50;

#[cfg(windows)]
const PYDIRECTINPUT_KEY_POST_PAUSE_MS: u64 = 50;

#[cfg(windows)]
pub fn press_key(hwnd: isize, keycode: i32) -> Result<()> {
    use std::thread;
    use std::time::Duration;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, SW_RESTORE,
        SetForegroundWindow, ShowWindow, WA_ACTIVE, WM_ACTIVATE,
    };

    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    if hwnd.0.is_null() {
        return Err(anyhow!("无效窗口句柄"));
    }

    unsafe {
        let foreground = GetForegroundWindow();
        let current_thread = GetCurrentThreadId();
        let target_thread = GetWindowThreadProcessId(hwnd, None);
        let foreground_thread = if foreground.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(foreground, None)
        };
        let mut attached = false;

        if foreground_thread != 0 && foreground_thread != current_thread {
            attached = AttachThreadInput(foreground_thread, current_thread, true).as_bool();
        }

        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if target_thread != 0 && target_thread != current_thread {
            let _ = AttachThreadInput(target_thread, current_thread, true);
        }
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageA(
            hwnd,
            WM_ACTIVATE,
            WPARAM(WA_ACTIVE as usize),
            LPARAM(0),
        );
        if target_thread != 0 && target_thread != current_thread {
            let _ = AttachThreadInput(target_thread, current_thread, false);
        }
        if attached {
            let _ = AttachThreadInput(foreground_thread, current_thread, false);
        }
    }

    thread::sleep(Duration::from_millis(60));

    let scan = directinput_scancode(keycode)
        .unwrap_or_else(|| unsafe { MapVirtualKeyW(keycode as u32, MAPVK_VK_TO_VSC) } as u16);
    if scan == 0 {
        return Err(anyhow!("无法将按键码映射为扫描码: {keycode}"));
    }

    let down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: Default::default(),
                wScan: scan,
                dwFlags: KEYEVENTF_SCANCODE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: Default::default(),
                wScan: scan,
                dwFlags: KEYBD_EVENT_FLAGS(KEYEVENTF_SCANCODE.0 | KEYEVENTF_KEYUP.0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };

    let sent_down = unsafe { SendInput(&[down], core::mem::size_of::<INPUT>() as i32) };
    if sent_down != 1 {
        return Err(anyhow!("SendInput 发送按下事件失败"));
    }

    thread::sleep(Duration::from_millis(PYDIRECTINPUT_KEY_HOLD_MS));

    let sent_up = unsafe { SendInput(&[up], core::mem::size_of::<INPUT>() as i32) };
    if sent_up != 1 {
        return Err(anyhow!("SendInput 发送抬起事件失败"));
    }

    thread::sleep(Duration::from_millis(PYDIRECTINPUT_KEY_POST_PAUSE_MS));

    Ok(())
}

#[cfg(windows)]
pub fn is_key_pressed(keycode: i32) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    unsafe { (GetAsyncKeyState(keycode) as u16 & 0x8000) != 0 }
}

#[cfg(not(windows))]
pub fn press_key(_hwnd: isize, _keycode: i32) -> Result<()> {
    Err(anyhow!("仅支持 Windows"))
}

#[cfg(not(windows))]
pub fn is_key_pressed(_keycode: i32) -> bool {
    false
}
