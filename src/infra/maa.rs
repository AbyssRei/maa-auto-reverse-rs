use crate::domain::{AutoReverseEngine, RuntimeMode, ScanDebugResult, StrategyConfig};
use crate::infra::{logging, paths, windowing};
use anyhow::{Result, anyhow};
use maa_framework::controller::Controller;
use maa_framework::custom::CustomAction;
use maa_framework::resource::Resource;
use maa_framework::tasker::Tasker;
use maa_framework::toolkit::Toolkit;
use maa_framework::{common, context::Context};
use once_cell::sync::{Lazy, OnceCell};
use serde_json::json;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ScanOutcome {
    pub cards: Vec<crate::domain::RecognizedCard>,
    pub debug: ScanDebugResult,
}

#[derive(Debug, Clone, Default)]
struct SharedScanState {
    cards: Vec<crate::domain::RecognizedCard>,
    debug: ScanDebugResult,
    done: bool,
}

struct RuntimeBridge {
    engine: AutoReverseEngine,
    controller: Controller,
    mode: RuntimeMode,
    scan_state: Arc<(Mutex<SharedScanState>, Condvar)>,
}

static BRIDGE: Lazy<Mutex<Option<RuntimeBridge>>> = Lazy::new(|| Mutex::new(None));
static LIB_LOADED: OnceCell<()> = OnceCell::new();

pub fn ensure_library_loaded() -> Result<()> {
    LIB_LOADED.get_or_try_init(|| {
        let dll = framework_dll_path()?;
        maa_framework::load_library(&dll).map_err(anyhow::Error::msg)?;
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}

pub struct MaaRuntimeSession {
    tasker: Tasker,
    _resource: Resource,
    _controller: Controller,
}

impl MaaRuntimeSession {
    pub fn start(window_title: &str, config: StrategyConfig, mode: RuntimeMode) -> Result<Self> {
        ensure_library_loaded()?;
        let paths = paths::app_paths()?;
        Toolkit::init_option(
            paths.project_root.to_str().unwrap_or("."),
            &json!({ "log_dir": paths.logs_dir }).to_string(),
        )?;

        let resource = Resource::new()?;
        resource.register_custom_action("AutoReverseTick", Box::new(TickAction))?;
        resource.register_custom_action("AutoReverseScanOnce", Box::new(ScanOnceAction))?;
        resource
            .post_bundle(paths.legacy_bundle_dir.to_str().unwrap_or("."))?
            .wait();

        let controller = build_win32_controller(window_title)?;
        controller.wait(controller.post_connection()?);

        let tasker = Tasker::new()?;
        tasker.bind_resource(&resource)?;
        tasker.bind_controller(&controller)?;
        if !tasker.inited() {
            return Err(anyhow!("Tasker init failed"));
        }

        let logger = logging::app_logger()?;
        let engine = AutoReverseEngine::new(config, logger);
        let scan_state = Arc::new((Mutex::new(SharedScanState::default()), Condvar::new()));

        {
            let mut bridge = BRIDGE.lock().expect("bridge lock");
            *bridge = Some(RuntimeBridge {
                engine,
                controller: controller.clone(),
                mode,
                scan_state,
            });
        }

        tasker.post_task("AutoReverseEntry", "{}")?;

        Ok(Self {
            tasker,
            _resource: resource,
            _controller: controller,
        })
    }

    pub fn stop(&self) -> Result<()> {
        self.tasker.post_stop()?;
        while self.tasker.running() {
            thread::sleep(Duration::from_millis(50));
        }

        if let Ok(mut bridge) = BRIDGE.lock() {
            *bridge = None;
        }

        Ok(())
    }

    pub fn update_config(&self, config: StrategyConfig, mode: RuntimeMode) {
        if let Ok(mut bridge) = BRIDGE.lock() {
            if let Some(bridge) = bridge.as_mut() {
                bridge.mode = mode;
                bridge.engine.update_config(config);
            }
        }
    }

    pub fn running(&self) -> bool {
        self.tasker.running()
    }

    pub fn scan_once(window_title: &str, config: StrategyConfig) -> Result<ScanOutcome> {
        ensure_library_loaded()?;
        let paths = paths::app_paths()?;
        Toolkit::init_option(
            paths.project_root.to_str().unwrap_or("."),
            &json!({ "log_dir": paths.logs_dir }).to_string(),
        )?;

        let resource = Resource::new()?;
        resource.register_custom_action("AutoReverseTick", Box::new(TickAction))?;
        resource.register_custom_action("AutoReverseScanOnce", Box::new(ScanOnceAction))?;
        resource
            .post_bundle(paths.legacy_bundle_dir.to_str().unwrap_or("."))?
            .wait();

        let controller = build_win32_controller(window_title)?;
        controller.wait(controller.post_connection()?);

        let tasker = Tasker::new()?;
        tasker.bind_resource(&resource)?;
        tasker.bind_controller(&controller)?;

        let scan_state = Arc::new((Mutex::new(SharedScanState::default()), Condvar::new()));
        {
            let logger = logging::app_logger()?;
            let mut bridge = BRIDGE.lock().expect("bridge lock");
            *bridge = Some(RuntimeBridge {
                engine: AutoReverseEngine::new(config, logger),
                controller,
                mode: RuntimeMode::AutoReverse,
                scan_state: scan_state.clone(),
            });
        }

        tasker.post_task("AutoReverseScanEntry", "{}")?;

        let (lock, cv) = &*scan_state;
        let mut state = lock.lock().expect("scan lock");
        while !state.done {
            state = cv.wait(state).expect("scan wait");
        }

        let result = ScanOutcome {
            cards: state.cards.clone(),
            debug: state.debug.clone(),
        };

        tasker.post_stop()?;
        if let Ok(mut bridge) = BRIDGE.lock() {
            *bridge = None;
        }

        Ok(result)
    }
}

struct TickAction;

impl CustomAction for TickAction {
    fn run(
        &self,
        context: &Context,
        _task_id: common::MaaId,
        _node_name: &str,
        _custom_action_name: &str,
        _custom_action_param: &str,
        _reco_id: common::MaaId,
        _box_rect: &common::Rect,
    ) -> bool {
        let mut bridge = match BRIDGE.lock() {
            Ok(bridge) => bridge,
            Err(_) => return false,
        };

        let Some(bridge) = bridge.as_mut() else {
            return false;
        };

        bridge
            .engine
            .tick(context, &bridge.controller, bridge.mode)
            .unwrap_or(false)
    }
}

struct ScanOnceAction;

impl CustomAction for ScanOnceAction {
    fn run(
        &self,
        context: &Context,
        _task_id: common::MaaId,
        _node_name: &str,
        _custom_action_name: &str,
        _custom_action_param: &str,
        _reco_id: common::MaaId,
        _box_rect: &common::Rect,
    ) -> bool {
        let mut bridge = match BRIDGE.lock() {
            Ok(bridge) => bridge,
            Err(_) => return false,
        };

        let Some(bridge) = bridge.as_mut() else {
            return false;
        };

        let result = bridge.engine.scan_once_debug(context, &bridge.controller);
        let (lock, cv) = &*bridge.scan_state;
        let mut state = lock.lock().expect("scan state");
        match result {
            Ok((cards, debug)) => {
                state.cards = cards;
                state.debug = debug;
                state.done = true;
            }
            Err(error) => {
                (logging::app_logger().expect("logger"))(format!("单次扫描失败: {error}"));
                state.done = true;
            }
        }
        cv.notify_all();
        false
    }
}

fn build_win32_controller(window_title: &str) -> Result<Controller> {
    let hwnd = windowing::find_window_hwnd(window_title)?
        .ok_or_else(|| anyhow!("未找到目标窗口: {window_title}"))?;

    Ok(Controller::new_win32(
        hwnd as *mut c_void,
        maa_framework::common::Win32ScreencapMethod::FRAME_POOL.bits(),
        maa_framework::common::Win32InputMethod::SEIZE.bits(),
        maa_framework::common::Win32InputMethod::SEIZE.bits(),
    )?)
}

fn framework_dll_path() -> Result<PathBuf> {
    Ok(paths::app_paths()?
        .legacy_runtime_dir
        .join("MaaFramework.dll"))
}
