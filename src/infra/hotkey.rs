use anyhow::Result;
use global_hotkey::hotkey::{Code, HotKey};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeySignal {
    ToggleAutoReverse,
    ToggleRefreshKeep,
}

pub struct HotkeyService {
    _manager: GlobalHotKeyManager,
    auto_reverse: HotKey,
    refresh_keep: HotKey,
}

impl HotkeyService {
    pub fn register() -> Result<Self> {
        let manager = GlobalHotKeyManager::new()?;
        let auto_reverse = HotKey::new(None, Code::F8);
        let refresh_keep = HotKey::new(None, Code::F9);
        manager.register_all(&[auto_reverse, refresh_keep])?;

        Ok(Self {
            _manager: manager,
            auto_reverse,
            refresh_keep,
        })
    }

    pub fn poll(&self) -> Vec<HotkeySignal> {
        let mut signals = Vec::new();
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == HotKeyState::Pressed {
                if event.id == self.auto_reverse.id() {
                    signals.push(HotkeySignal::ToggleAutoReverse);
                } else if event.id == self.refresh_keep.id() {
                    signals.push(HotkeySignal::ToggleRefreshKeep);
                }
            }
        }
        signals
    }
}
