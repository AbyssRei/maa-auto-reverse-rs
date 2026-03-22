use crate::domain::{EditableLists, PersistedState, RuntimeMode, StrategyConfig};
use crate::infra::{logging, maa, persistence, windowing};
use anyhow::Result;
use crossbeam_channel::Receiver;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuntimeStatus {
    #[default]
    Idle,
    Starting,
    Running(RuntimeMode),
    Stopping,
    ScanDebugging,
    Error,
}

#[derive(Clone)]
pub struct RuntimeCoordinator {
    state: Arc<Mutex<PersistedState>>,
    session: Arc<Mutex<Option<maa::MaaRuntimeSession>>>,
    logs: Receiver<String>,
}

impl RuntimeCoordinator {
    pub fn new() -> Result<Self> {
        Ok(Self {
            state: Arc::new(Mutex::new(persistence::load_or_import_state()?)),
            session: Arc::new(Mutex::new(None)),
            logs: logging::subscribe_logs(),
        })
    }

    pub fn state(&self) -> PersistedState {
        self.state.lock().expect("state lock").clone()
    }

    pub fn editable_lists(&self) -> EditableLists {
        EditableLists::from_strategy(&self.state().strategy_config)
    }

    pub fn save_state(&self, next: PersistedState) -> Result<()> {
        persistence::save_state(&next)?;
        *self.state.lock().expect("state lock") = next;
        Ok(())
    }

    pub fn update_strategy(&self, strategy: StrategyConfig) -> Result<()> {
        let mut state = self.state();
        state.strategy_config = strategy.clone();
        persistence::save_state(&state)?;
        *self.state.lock().expect("state lock") = state;

        if let Some(session) = self.session.lock().expect("session lock").as_ref() {
            let mode = if strategy.refresh_keep_mode {
                RuntimeMode::RefreshKeep
            } else {
                RuntimeMode::AutoReverse
            };
            session.update_config(strategy, mode);
        }

        Ok(())
    }

    pub fn refresh_windows(&self) -> Result<Vec<windowing::WindowInfo>> {
        windowing::list_windows()
    }

    pub fn start(&self, mode: RuntimeMode, window_title: String) -> Result<()> {
        let mut state = self.state();
        state.app_settings.selected_window_title = window_title.clone();
        state.strategy_config.refresh_keep_mode = mode == RuntimeMode::RefreshKeep;
        persistence::save_state(&state)?;
        *self.state.lock().expect("state lock") = state.clone();

        let session =
            maa::MaaRuntimeSession::start(&window_title, state.strategy_config.clone(), mode)?;
        *self.session.lock().expect("session lock") = Some(session);
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        if let Some(session) = self.session.lock().expect("session lock").take() {
            session.stop()?;
        }
        Ok(())
    }

    pub fn scan_once(&self, window_title: String) -> Result<maa::ScanOutcome> {
        let state = self.state();
        maa::MaaRuntimeSession::scan_once(&window_title, state.strategy_config.clone())
    }

    pub fn running_mode(&self) -> Option<RuntimeMode> {
        self.session
            .lock()
            .expect("session lock")
            .as_ref()
            .and_then(|session| {
                if session.running() {
                    Some(if self.state().strategy_config.refresh_keep_mode {
                        RuntimeMode::RefreshKeep
                    } else {
                        RuntimeMode::AutoReverse
                    })
                } else {
                    None
                }
            })
    }

    pub fn drain_logs(&self) -> Vec<String> {
        let mut logs = Vec::new();
        while let Ok(line) = self.logs.try_recv() {
            logs.push(line);
        }
        logs
    }
}

pub fn run_scan_once_cli(window: Option<String>) -> Result<String> {
    let coordinator = RuntimeCoordinator::new()?;
    let window = window.unwrap_or_else(|| coordinator.state().app_settings.selected_window_title);
    let result = coordinator.scan_once(window.clone())?;

    if result.cards.is_empty() {
        return Ok(format!("窗口 `{window}` 未识别到可用卡片"));
    }

    let lines = result
        .cards
        .iter()
        .map(|card| {
            format!(
                "槽位{} | 费用: {} | 名称: {}",
                card.slot, card.price, card.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    Ok(lines)
}
