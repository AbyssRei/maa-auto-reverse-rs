use crate::domain::{
    EditableLists, ImagePreview, PersistedState, PresetEntry, RuntimeMode, StrategyConfig, UiScale,
};
use crate::infra::hotkey::{HotkeyService, HotkeySignal};
use crate::orchestrator::{RuntimeCoordinator, RuntimeStatus};
use anyhow::Result;
use iced::widget::image::Handle;
use iced::widget::text_editor;
use iced::widget::{
    button, checkbox, column, container, image as image_view, pick_list, radio, row, scrollable,
    text, text_editor as text_editor_widget,
};
use iced::{Alignment, Element, Length, Subscription, Task, Theme, application, time};
use std::time::Duration;

pub fn run_gui() -> iced::Result {
    application(initialize, update, view)
        .title(app_title)
        .theme(app_theme)
        .subscription(subscription)
        .run()
}

struct App {
    coordinator: RuntimeCoordinator,
    state: PersistedState,
    editable: EditableEditors,
    windows: Vec<String>,
    status: RuntimeStatus,
    logs: Vec<String>,
    scan_result_lines: Vec<String>,
    scan_debug: Option<crate::domain::ScanDebugResult>,
    cached_debug_images: CachedDebugImages,
    hotkeys: Option<HotkeyService>,
}

#[derive(Default)]
struct CachedDebugImages {
    full_frame: Option<Handle>,
    slots: Vec<CachedSlotImages>,
}

#[derive(Default)]
struct CachedSlotImages {
    price: Option<Handle>,
    name: Option<Handle>,
}

#[derive(Default)]
struct EditableEditors {
    items: text_editor::Content,
    operators: text_editor::Content,
    buy_only: text_editor::Content,
    six_star: text_editor::Content,
}

impl EditableEditors {
    fn from_lists(lists: &EditableLists) -> Self {
        Self {
            items: text_editor::Content::with_text(&lists.items),
            operators: text_editor::Content::with_text(&lists.operators),
            buy_only: text_editor::Content::with_text(&lists.buy_only_operators),
            six_star: text_editor::Content::with_text(&lists.six_star_operators),
        }
    }

    fn to_lists(&self) -> EditableLists {
        EditableLists {
            items: self.items.text(),
            operators: self.operators.text(),
            buy_only_operators: self.buy_only.text(),
            six_star_operators: self.six_star.text(),
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    WindowsLoaded(Result<Vec<String>, String>),
    WindowSelected(String),
    RefreshWindows,
    ToggleAutoReverse,
    ToggleRefreshKeep,
    Stopped(Result<(), String>),
    Started(Result<(), String>, RuntimeMode),
    ScanOnce,
    ScanCompleted(Result<ScanPayload, String>),
    Tick,
    ListsEdited(ListField, text_editor::Action),
    TogglePreset(PresetKind, String, bool),
    SetScale(UiScale),
}

#[derive(Debug, Clone, Copy)]
enum ListField {
    Items,
    Operators,
    BuyOnly,
    SixStar,
}

#[derive(Debug, Clone, Copy)]
enum PresetKind {
    Item,
    BuyOnlyOperator,
}

#[derive(Debug, Clone)]
struct ScanPayload {
    lines: Vec<String>,
    debug: crate::domain::ScanDebugResult,
}

fn app_title(_app: &App) -> String {
    "卫戍协议-倒转小助手 (Rust)".to_string()
}

fn app_theme(_app: &App) -> Theme {
    Theme::Light
}

fn initialize() -> (App, Task<Message>) {
    let coordinator = RuntimeCoordinator::new().expect("failed to initialize coordinator");
    let state = coordinator.state();
    let editable = EditableEditors::from_lists(&coordinator.editable_lists());

    (
        App {
            coordinator,
            state,
            editable,
            windows: Vec::new(),
            status: RuntimeStatus::Idle,
            logs: vec!["[启动] 应用已初始化".to_string()],
            scan_result_lines: vec!["暂无识别结果".to_string()],
            scan_debug: None,
            cached_debug_images: CachedDebugImages::default(),
            hotkeys: HotkeyService::register().ok(),
        },
        Task::perform(async { load_windows_task() }, Message::WindowsLoaded),
    )
}

fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::WindowsLoaded(result) => {
            match result {
                Ok(windows) => {
                    if app.state.app_settings.selected_window_title.is_empty()
                        && !windows.is_empty()
                    {
                        app.state.app_settings.selected_window_title = windows[0].clone();
                    }
                    app.windows = windows;
                }
                Err(error) => app.logs.push(format!("[错误] {error}")),
            }
            Task::none()
        }
        Message::WindowSelected(title) => {
            app.state.app_settings.selected_window_title = title;
            let _ = app.coordinator.save_state(app.state.clone());
            Task::none()
        }
        Message::RefreshWindows => {
            Task::perform(async { load_windows_task() }, Message::WindowsLoaded)
        }
        Message::ToggleAutoReverse => {
            if matches!(app.status, RuntimeStatus::Running(RuntimeMode::AutoReverse)) {
                app.status = RuntimeStatus::Stopping;
                let coordinator = app.coordinator.clone();
                return Task::perform(
                    async move { coordinator.stop().map_err(|e| e.to_string()) },
                    Message::Stopped,
                );
            }

            if matches!(app.status, RuntimeStatus::Idle | RuntimeStatus::Error) {
                app.status = RuntimeStatus::Starting;
                let coordinator = app.coordinator.clone();
                let window = app.state.app_settings.selected_window_title.clone();
                let strategy = current_strategy(app);
                let _ = coordinator.update_strategy(strategy);
                return Task::perform(
                    async move {
                        coordinator
                            .start(RuntimeMode::AutoReverse, window)
                            .map_err(|e| e.to_string())
                    },
                    |result| Message::Started(result, RuntimeMode::AutoReverse),
                );
            }

            Task::none()
        }
        Message::ToggleRefreshKeep => {
            if matches!(app.status, RuntimeStatus::Running(RuntimeMode::RefreshKeep)) {
                app.status = RuntimeStatus::Stopping;
                let coordinator = app.coordinator.clone();
                return Task::perform(
                    async move { coordinator.stop().map_err(|e| e.to_string()) },
                    Message::Stopped,
                );
            }

            if matches!(app.status, RuntimeStatus::Idle | RuntimeStatus::Error) {
                app.status = RuntimeStatus::Starting;
                let coordinator = app.coordinator.clone();
                let window = app.state.app_settings.selected_window_title.clone();
                let mut strategy = current_strategy(app);
                strategy.refresh_keep_mode = true;
                let _ = coordinator.update_strategy(strategy);
                return Task::perform(
                    async move {
                        coordinator
                            .start(RuntimeMode::RefreshKeep, window)
                            .map_err(|e| e.to_string())
                    },
                    |result| Message::Started(result, RuntimeMode::RefreshKeep),
                );
            }

            Task::none()
        }
        Message::Started(result, mode) => {
            match result {
                Ok(()) => {
                    app.status = RuntimeStatus::Running(mode);
                    app.logs.push(format!("[状态] {} 已启动", mode));
                }
                Err(error) => {
                    app.status = RuntimeStatus::Error;
                    app.logs.push(format!("[错误] 启动失败: {error}"));
                }
            }
            Task::none()
        }
        Message::Stopped(result) => {
            match result {
                Ok(()) => {
                    app.status = RuntimeStatus::Idle;
                    app.logs.push("[状态] 自动倒转已停止".to_string());
                }
                Err(error) => {
                    app.status = RuntimeStatus::Error;
                    app.logs.push(format!("[错误] 停止失败: {error}"));
                }
            }
            Task::none()
        }
        Message::ScanOnce => {
            if !matches!(app.status, RuntimeStatus::Idle | RuntimeStatus::Error) {
                return Task::none();
            }

            app.status = RuntimeStatus::ScanDebugging;
            let coordinator = app.coordinator.clone();
            let window = app.state.app_settings.selected_window_title.clone();
            let strategy = current_strategy(app);
            let _ = coordinator.update_strategy(strategy);
            Task::perform(
                async move {
                    coordinator
                        .scan_once(window)
                        .map(|result| ScanPayload {
                            lines: result
                                .cards
                                .iter()
                                .map(|card| {
                                    format!(
                                        "槽位{} | 费用: {} | 名称: {}",
                                        card.slot, card.price, card.name
                                    )
                                })
                                .collect(),
                            debug: result.debug,
                        })
                        .map_err(|e| e.to_string())
                },
                Message::ScanCompleted,
            )
        }
        Message::ScanCompleted(result) => {
            match result {
                Ok(payload) => {
                    app.scan_result_lines = if payload.lines.is_empty() {
                        vec!["本次未识别到有效结果".to_string()]
                    } else {
                        payload.lines
                    };
                    app.cached_debug_images = CachedDebugImages::from_debug(&payload.debug);
                    app.scan_debug = Some(payload.debug);
                    app.logs.push("[状态] 单次扫描完成".to_string());
                    app.status = RuntimeStatus::Idle;
                }
                Err(error) => {
                    app.logs.push(format!("[错误] 扫描失败: {error}"));
                    app.status = RuntimeStatus::Error;
                }
            }
            Task::none()
        }
        Message::Tick => {
            app.logs.extend(app.coordinator.drain_logs());
            if let Some(mode) = app.coordinator.running_mode() {
                app.status = RuntimeStatus::Running(mode);
            } else if matches!(app.status, RuntimeStatus::Running(_)) {
                app.status = RuntimeStatus::Idle;
            }

            if let Some(hotkeys) = &app.hotkeys {
                for signal in hotkeys.poll() {
                    match signal {
                        HotkeySignal::ToggleAutoReverse => {
                            return update(app, Message::ToggleAutoReverse);
                        }
                        HotkeySignal::ToggleRefreshKeep => {
                            return update(app, Message::ToggleRefreshKeep);
                        }
                    }
                }
            }

            Task::none()
        }
        Message::ListsEdited(field, action) => {
            let editor = match field {
                ListField::Items => &mut app.editable.items,
                ListField::Operators => &mut app.editable.operators,
                ListField::BuyOnly => &mut app.editable.buy_only,
                ListField::SixStar => &mut app.editable.six_star,
            };
            editor.perform(action);
            sync_strategy_from_editors(app);
            Task::none()
        }
        Message::TogglePreset(kind, value, checked) => {
            apply_preset_toggle(app, kind, value, checked);
            sync_strategy_from_editors(app);
            Task::none()
        }
        Message::SetScale(value) => {
            app.state.app_settings.ui_scale = value;
            app.state.strategy_config.ui_scale = app.state.app_settings.ui_scale;
            let _ = app.coordinator.save_state(app.state.clone());
            Task::none()
        }
    }
}

fn subscription(_app: &App) -> Subscription<Message> {
    time::every(Duration::from_millis(250)).map(|_| Message::Tick)
}

fn view(app: &App) -> Element<'_, Message> {
    let top = row![
        text("选择窗口:"),
        pick_list(
            app.windows.clone(),
            Some(app.state.app_settings.selected_window_title.clone()),
            Message::WindowSelected
        )
        .width(Length::FillPortion(3)),
        button("刷新").on_press(Message::RefreshWindows),
        text("界面比例:"),
        radio(
            "90%",
            UiScale::Scale90,
            Some(app.state.app_settings.ui_scale),
            Message::SetScale
        ),
        radio(
            "100%",
            UiScale::Scale100,
            Some(app.state.app_settings.ui_scale),
            Message::SetScale
        ),
        text(status_text(app.status)),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let controls = row![
        button(button_text(app.status, RuntimeMode::AutoReverse))
            .on_press_maybe(button_action(app.status, RuntimeMode::AutoReverse)),
        button(button_text(app.status, RuntimeMode::RefreshKeep))
            .on_press_maybe(button_action(app.status, RuntimeMode::RefreshKeep)),
        button("扫描识别（测试用）").on_press_maybe(
            if matches!(app.status, RuntimeStatus::Idle | RuntimeStatus::Error) {
                Some(Message::ScanOnce)
            } else {
                None
            }
        ),
    ]
    .spacing(12);

    let editors = column![
        text("保留道具"),
        text_editor_widget(&app.editable.items)
            .height(100)
            .on_action(|action| Message::ListsEdited(ListField::Items, action)),
        preset_section(
            "预设道具选择",
            &app.state.presets.predefined_items,
            &app.state.strategy_config.item_list,
            PresetKind::Item
        ),
        text("倒转干员"),
        text_editor_widget(&app.editable.operators)
            .height(100)
            .on_action(|action| Message::ListsEdited(ListField::Operators, action)),
        text("保留干员"),
        text_editor_widget(&app.editable.buy_only)
            .height(100)
            .on_action(|action| Message::ListsEdited(ListField::BuyOnly, action)),
        preset_section(
            "预设保留干员选择",
            &app.state.presets.predefined_buy_only_operators,
            &app.state.strategy_config.buy_only_operator_list,
            PresetKind::BuyOnlyOperator,
        ),
        text("不处理名单"),
        text_editor_widget(&app.editable.six_star)
            .height(100)
            .on_action(|action| Message::ListsEdited(ListField::SixStar, action)),
    ]
    .spacing(8);

    let scan_lines = app
        .scan_result_lines
        .iter()
        .fold(column![], |column, line| column.push(text(line.clone())));

    let debug_images = if let Some(debug) = &app.scan_debug {
        let mut content = column![text("整张截图")].spacing(8);
        if let Some(handle) = &app.cached_debug_images.full_frame {
            content = content.push(render_cached_handle(handle.clone(), 900));
        }
        for (index, slot) in debug.slots.iter().enumerate() {
            content = content.push(text(format!(
                "槽位 {} | 价格OCR: {} | 名称OCR: {}",
                slot.slot, slot.price_ocr, slot.name_ocr
            )));
            let mut row_widgets = row![].spacing(8);
            if let Some(handle) = app
                .cached_debug_images
                .slots
                .get(index)
                .and_then(|slot| slot.price.clone())
            {
                row_widgets = row_widgets.push(render_cached_handle(handle, 220));
            }
            if let Some(handle) = app
                .cached_debug_images
                .slots
                .get(index)
                .and_then(|slot| slot.name.clone())
            {
                row_widgets = row_widgets.push(render_cached_handle(handle, 360));
            }
            content = content.push(row_widgets);
        }
        content
    } else {
        column![]
    };

    let logs = app
        .logs
        .iter()
        .rev()
        .take(200)
        .fold(column![], |column, line| column.push(text(line.clone())));

    let body = column![
        top,
        controls,
        container(editors).padding(12),
        container(column![text("识别结果"), scan_lines, debug_images].spacing(8)).padding(12),
        container(column![text("日志"), logs].spacing(4)).padding(12),
    ]
    .spacing(16)
    .padding(16);

    scrollable(body).into()
}

fn preset_section<'a>(
    title: &'a str,
    entries: &'a [PresetEntry],
    selected: &'a [String],
    kind: PresetKind,
) -> Element<'a, Message> {
    let content = entries
        .iter()
        .fold(column![text(title)].spacing(4), |column, entry| {
            let checked = selected.iter().any(|item| item == &entry.value);
            column.push(checkbox(checked).label(entry.label.clone()).on_toggle({
                let value = entry.value.clone();
                move |checked| Message::TogglePreset(kind, value.clone(), checked)
            }))
        });
    content.into()
}

fn render_cached_handle<'a>(handle: Handle, max_width: u16) -> Element<'a, Message> {
    image_view(handle)
        .width(Length::Fixed(max_width as f32))
        .into()
}

fn status_text(status: RuntimeStatus) -> &'static str {
    match status {
        RuntimeStatus::Idle => "空闲",
        RuntimeStatus::Starting => "启动中",
        RuntimeStatus::Running(RuntimeMode::AutoReverse) => "自动倒转运行中",
        RuntimeStatus::Running(RuntimeMode::RefreshKeep) => "刷新保留运行中",
        RuntimeStatus::Stopping => "停止中",
        RuntimeStatus::ScanDebugging => "扫描中",
        RuntimeStatus::Error => "错误",
    }
}

fn button_text(status: RuntimeStatus, mode: RuntimeMode) -> &'static str {
    match (status, mode) {
        (RuntimeStatus::Running(RuntimeMode::AutoReverse), RuntimeMode::AutoReverse) => {
            "停止自动倒转 (F8)"
        }
        (RuntimeStatus::Running(RuntimeMode::RefreshKeep), RuntimeMode::RefreshKeep) => {
            "停止刷新保留 (F9)"
        }
        (_, RuntimeMode::AutoReverse) => "启动自动倒转 (F8)",
        (_, RuntimeMode::RefreshKeep) => "干员道具刷新保留 (F9)",
    }
}

fn button_action(status: RuntimeStatus, mode: RuntimeMode) -> Option<Message> {
    match (status, mode) {
        (RuntimeStatus::Running(RuntimeMode::AutoReverse), RuntimeMode::AutoReverse) => {
            Some(Message::ToggleAutoReverse)
        }
        (RuntimeStatus::Running(RuntimeMode::RefreshKeep), RuntimeMode::RefreshKeep) => {
            Some(Message::ToggleRefreshKeep)
        }
        (RuntimeStatus::Idle | RuntimeStatus::Error, RuntimeMode::AutoReverse) => {
            Some(Message::ToggleAutoReverse)
        }
        (RuntimeStatus::Idle | RuntimeStatus::Error, RuntimeMode::RefreshKeep) => {
            Some(Message::ToggleRefreshKeep)
        }
        _ => None,
    }
}

fn load_windows_task() -> Result<Vec<String>, String> {
    let coordinator = RuntimeCoordinator::new().map_err(|e| e.to_string())?;
    coordinator
        .refresh_windows()
        .map(|windows| windows.into_iter().map(|window| window.title).collect())
        .map_err(|e| e.to_string())
}

fn current_strategy(app: &App) -> StrategyConfig {
    let mut strategy = app.state.strategy_config.clone();
    app.editable.to_lists().apply_to(&mut strategy);
    strategy.ui_scale = app.state.app_settings.ui_scale;
    strategy
}

fn sync_strategy_from_editors(app: &mut App) {
    let strategy = current_strategy(app);
    app.state.strategy_config = strategy.clone();
    let _ = app.coordinator.update_strategy(strategy);
}

fn apply_preset_toggle(app: &mut App, kind: PresetKind, value: String, checked: bool) {
    let mut strategy = current_strategy(app);
    let list = match kind {
        PresetKind::Item => &mut strategy.item_list,
        PresetKind::BuyOnlyOperator => &mut strategy.buy_only_operator_list,
    };

    if checked {
        if !list.iter().any(|item| item == &value) {
            list.push(value);
        }
    } else {
        list.retain(|item| item != &value);
    }

    app.state.strategy_config = strategy.clone();
    app.editable = EditableEditors::from_lists(&EditableLists::from_strategy(&strategy));
    let _ = app.coordinator.update_strategy(strategy);
}

fn preview_handle(preview: &ImagePreview) -> Handle {
    Handle::from_rgba(preview.width, preview.height, preview.rgba.clone())
}

impl CachedDebugImages {
    fn from_debug(debug: &crate::domain::ScanDebugResult) -> Self {
        Self {
            full_frame: debug.full_frame.as_ref().map(preview_handle),
            slots: debug
                .slots
                .iter()
                .map(|slot| CachedSlotImages {
                    price: slot.price_roi.as_ref().map(preview_handle),
                    name: slot.name_roi.as_ref().map(preview_handle),
                })
                .collect(),
        }
    }
}
