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
use std::sync::atomic::{AtomicBool, Ordering};
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
    pipeline_watch_stop: Option<Arc<AtomicBool>>,
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
        let bundle_job = resource.post_bundle(paths.bundle_dir.to_str().unwrap_or("."))?;
        bundle_job.wait();
        if !bundle_job.succeeded() {
            return Err(anyhow!(
                "加载 MAA bundle 失败: {}",
                paths.bundle_dir.display()
            ));
        }

        let hwnd = target_hwnd(window_title)?;
        let controller = build_win32_controller(hwnd)?;
        controller.wait(controller.post_connection()?);
        let pipeline_override = load_pipeline_override_file().unwrap_or_default();

        let tasker = Tasker::new()?;
        tasker.bind_resource(&resource)?;
        tasker.bind_controller(&controller)?;
        if !tasker.inited() {
            return Err(anyhow!("Tasker init failed"));
        }
        let logger = logging::app_logger()?;
        log_pipeline_override_status(&logger, &pipeline_override);
        let engine = AutoReverseEngine::new(config, logger, hwnd);
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

        let task_job = tasker.post_task("AutoReverseEntry", &pipeline_override)?;
        let watch_flag =
            start_pipeline_override_watch(tasker.clone(), task_job.id, logging::app_logger()?);

        Ok(Self {
            tasker,
            _resource: resource,
            _controller: controller,
            pipeline_watch_stop: watch_flag,
        })
    }

    pub fn stop(&self) -> Result<()> {
        if let Some(flag) = &self.pipeline_watch_stop {
            flag.store(true, Ordering::Relaxed);
        }
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
        let bundle_job = resource.post_bundle(paths.bundle_dir.to_str().unwrap_or("."))?;
        bundle_job.wait();
        if !bundle_job.succeeded() {
            return Err(anyhow!(
                "加载 MAA bundle 失败: {}",
                paths.bundle_dir.display()
            ));
        }

        let hwnd = target_hwnd(window_title)?;
        let controller = build_win32_controller(hwnd)?;
        controller.wait(controller.post_connection()?);
        let pipeline_override = load_pipeline_override_file().unwrap_or_default();

        let tasker = Tasker::new()?;
        tasker.bind_resource(&resource)?;
        tasker.bind_controller(&controller)?;
        if !tasker.inited() {
            return Err(anyhow!("Tasker init failed"));
        }

        let scan_state = Arc::new((Mutex::new(SharedScanState::default()), Condvar::new()));
        {
            let logger = logging::app_logger()?;
            log_pipeline_override_status(&logger, &pipeline_override);
            let mut bridge = BRIDGE.lock().expect("bridge lock");
            *bridge = Some(RuntimeBridge {
                engine: AutoReverseEngine::new(config, logger, hwnd),
                controller,
                mode: RuntimeMode::AutoReverse,
                scan_state: scan_state.clone(),
            });
        }

        tasker.post_task("AutoReverseScanEntry", &pipeline_override)?;

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
        while tasker.running() {
            thread::sleep(Duration::from_millis(50));
        }
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
        _context: &Context,
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
            .tick(_context, &bridge.controller, bridge.mode)
            .unwrap_or(false)
    }
}

struct ScanOnceAction;

impl CustomAction for ScanOnceAction {
    fn run(
        &self,
        _context: &Context,
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

        let result = bridge.engine.scan_once_debug(_context, &bridge.controller);
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
        true
    }
}

fn target_hwnd(window_title: &str) -> Result<isize> {
    windowing::find_window_hwnd(window_title)?
        .ok_or_else(|| anyhow!("未找到目标窗口: {window_title}"))
}

fn build_win32_controller(hwnd: isize) -> Result<Controller> {
    Ok(Controller::new_win32(
        hwnd as *mut c_void,
        maa_framework::common::Win32ScreencapMethod::FRAME_POOL.bits(),
        maa_framework::common::Win32InputMethod::SEND_MESSAGE_WITH_CURSOR_POS.bits(),
        maa_framework::common::Win32InputMethod::SEND_MESSAGE.bits(),
    )?)
}

fn framework_dll_path() -> Result<PathBuf> {
    Ok(paths::app_paths()?.runtime_dir.join("MaaFramework.dll"))
}

fn pipeline_override_path() -> Result<PathBuf> {
    paths::file_in_config("pipeline_override.json")
}

fn load_pipeline_override_file() -> Result<String> {
    let path = pipeline_override_path()?;
    if !path.exists() {
        return Ok("{}".to_string());
    }

    let raw = std::fs::read_to_string(&path)?;
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|error| anyhow!("pipeline override JSON 无效: {error}"))?;
    Ok(parsed.to_string())
}

fn start_pipeline_override_watch(
    tasker: Tasker,
    task_id: common::MaaId,
    logger: crate::domain::engine::SharedLogger,
) -> Option<Arc<AtomicBool>> {
    let path = pipeline_override_path().ok()?;
    if !path.exists() {
        return None;
    }

    let stop_flag = Arc::new(AtomicBool::new(false));
    let thread_stop = stop_flag.clone();
    logger(format!(
        "[AutoReverse] 已启用 pipeline override 热重载: {}",
        path.display()
    ));
    thread::spawn(move || {
        let mut last_modified = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok();

        while !thread_stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(1));
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let modified = meta.modified().ok();
            if modified.is_none() || modified == last_modified {
                continue;
            }
            last_modified = modified;

            match std::fs::read_to_string(&path)
                .map_err(anyhow::Error::from)
                .and_then(|raw| {
                    let value: serde_json::Value = serde_json::from_str(&raw)?;
                    Ok(value.to_string())
                })
                .and_then(|override_json| {
                    tasker
                        .override_pipeline(task_id, &override_json)
                        .map_err(Into::into)
                }) {
                Ok(true) => logger(format!(
                    "[AutoReverse] pipeline override reloaded: {}",
                    path.display()
                )),
                Ok(false) => logger(format!(
                    "[AutoReverse] pipeline override ignored: {}",
                    path.display()
                )),
                Err(error) => logger(format!("[AutoReverse] override reload failed: {error}")),
            }
        }
    });

    Some(stop_flag)
}

fn log_pipeline_override_status(logger: &crate::domain::engine::SharedLogger, override_json: &str) {
    let Ok(path) = pipeline_override_path() else {
        return;
    };

    if !path.exists() {
        return;
    }

    if override_json.trim() == "{}" {
        logger(format!(
            "[AutoReverse] 检测到 pipeline override 文件，但当前内容为空对象: {}",
            path.display()
        ));
    } else {
        logger(format!(
            "[AutoReverse] 检测到 pipeline override 文件，启动时将应用覆盖配置: {}",
            path.display()
        ));
    }
}
