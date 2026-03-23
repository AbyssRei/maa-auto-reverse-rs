use crate::domain::{
    EditableLists, ImagePreview, PersistedState, PresetEntry, RuntimeMode, StrategyConfig, UiScale,
};
use crate::infra::hotkey::{HotkeyService, HotkeySignal};
use crate::orchestrator::{RuntimeCoordinator, RuntimeStatus};
use anyhow::Result;
use chrono::Local;
use iced::widget::image::Handle;
use iced::widget::text_editor;
use iced::widget::{
    button, checkbox, column, container, image as image_view, pick_list, radio, row, scrollable,
    text, text_editor as text_editor_widget,
};
use iced::{
    Alignment, Element, Length, Size, Subscription, Task, Theme, application, time, window,
};
use image::RgbaImage;
use rfd::FileDialog;
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
    window_width: f32,
}

#[derive(Default)]
struct CachedDebugImages {
    full_frame: Option<Handle>,
    annotated_frame: Option<Handle>,
    recognized_frame: Option<Handle>,
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
    ToggleAutoRefresh(bool),
    SetScale(UiScale),
    WindowResized(Size),
    SaveImage(ImageSaveTarget),
    ImageSaved(Result<String, String>),
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

#[derive(Debug, Clone, Copy)]
enum ImageSaveTarget {
    FullFrame,
    AnnotatedFrame,
    RecognizedFrame,
    SlotPrice(usize),
    SlotName(usize),
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
            window_width: 1280.0,
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
        Message::ToggleAutoRefresh(enabled) => {
            app.state.strategy_config.auto_reverse_auto_refresh = enabled;
            sync_strategy_from_editors(app);
            Task::none()
        }
        Message::SetScale(value) => {
            app.state.app_settings.ui_scale = value;
            app.state.strategy_config.ui_scale = app.state.app_settings.ui_scale;
            let _ = app.coordinator.save_state(app.state.clone());
            Task::none()
        }
        Message::WindowResized(size) => {
            app.window_width = size.width;
            Task::none()
        }
        Message::SaveImage(target) => {
            let Some((preview, filename)) = preview_for_target(app, target) else {
                app.logs.push("[错误] 未找到可导出的图片".to_string());
                return Task::none();
            };

            Task::perform(
                async move { save_preview_with_dialog(preview, filename).map_err(|e| e.to_string()) },
                Message::ImageSaved,
            )
        }
        Message::ImageSaved(result) => {
            match result {
                Ok(path) => app.logs.push(format!("[状态] 图片已保存: {path}")),
                Err(error) if error == "__CANCELLED__" => {
                    app.logs.push("[状态] 已取消保存图片".to_string())
                }
                Err(error) => app.logs.push(format!("[错误] 图片保存失败: {error}")),
            }
            Task::none()
        }
    }
}

fn subscription(_app: &App) -> Subscription<Message> {
    Subscription::batch([
        time::every(Duration::from_millis(250)).map(|_| Message::Tick),
        window::resize_events().map(|(_id, size)| Message::WindowResized(size)),
    ])
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
        checkbox(app.state.strategy_config.auto_reverse_auto_refresh)
            .label("倒转自动刷新")
            .on_toggle(Message::ToggleAutoRefresh),
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
            PresetKind::Item,
            app.window_width,
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
            app.window_width,
        ),
        text("不处理名单"),
        text_editor_widget(&app.editable.six_star)
            .height(100)
            .on_action(|action| Message::ListsEdited(ListField::SixStar, action)),
    ]
    .spacing(8);

    let debug_images = if let Some(debug) = &app.scan_debug {
        let mut content = column![text("图像调试")].spacing(12);
        let mut frame_row = row![].spacing(12);
        if let Some(handle) = &app.cached_debug_images.full_frame {
            frame_row = frame_row.push(
                column![
                    row![
                        text("原图"),
                        button("保存").on_press(Message::SaveImage(ImageSaveTarget::FullFrame))
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    render_cached_handle(handle.clone(), 560)
                ]
                .spacing(6),
            );
        }
        if let Some(handle) = &app.cached_debug_images.annotated_frame {
            frame_row = frame_row.push(
                column![
                    row![
                        text("标注图"),
                        button("保存")
                            .on_press(Message::SaveImage(ImageSaveTarget::AnnotatedFrame))
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    render_cached_handle(handle.clone(), 560)
                ]
                .spacing(6),
            );
        }
        content = content.push(frame_row);

        let mut focus_row = row![].spacing(12);
        if let Some(handle) = &app.cached_debug_images.recognized_frame {
            focus_row = focus_row.push(
                column![
                    row![
                        text("识别命中图"),
                        button("保存")
                            .on_press(Message::SaveImage(ImageSaveTarget::RecognizedFrame))
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    render_cached_handle(handle.clone(), 560)
                ]
                .spacing(6),
            );
        }
        focus_row = focus_row.push(
            container(recognized_summary(debug))
                .padding(10)
                .width(Length::Fill),
        );
        content = content.push(focus_row);
        content
    } else {
        column![
            text("图像调试"),
            text("完成一次识别测试后会在这里显示原图、标注图和命中图")
        ]
        .spacing(8)
    };

    let slot_cards = if let Some(debug) = &app.scan_debug {
        debug.slots.iter().enumerate().fold(
            column![text("槽位明细")].spacing(10),
            |column, (index, slot)| column.push(slot_debug_card(app, index, slot)),
        )
    } else {
        column![text("槽位明细"), text("暂无槽位识别数据")].spacing(8)
    };

    let results_panel = column![result_overview(app), debug_images, slot_cards,].spacing(16);

    let logs = app
        .logs
        .iter()
        .rev()
        .take(200)
        .fold(column![], |column, line| column.push(text(line.clone())));

    let config_panel = container(column![text("策略配置"), editors].spacing(12)).padding(12);
    let results_panel = container(results_panel).padding(12);
    let log_panel = container(column![text("日志"), logs].spacing(4)).padding(12);

    let main_content = if app.window_width >= 1380.0 {
        column![
            row![
                container(config_panel).width(Length::FillPortion(2)),
                container(results_panel).width(Length::FillPortion(3)),
            ]
            .spacing(16),
            log_panel,
        ]
        .spacing(16)
    } else {
        column![config_panel, results_panel, log_panel].spacing(16)
    };

    let body = column![top, controls, main_content].spacing(16).padding(16);

    scrollable(body).into()
}

fn preset_section<'a>(
    title: &'a str,
    entries: &'a [PresetEntry],
    selected: &'a [String],
    kind: PresetKind,
    window_width: f32,
) -> Element<'a, Message> {
    let columns = preset_columns(window_width);
    let mut content = column![text(title)].spacing(8);

    for chunk in entries.chunks(columns) {
        let row_widgets = chunk.iter().fold(row![].spacing(12), |row, entry| {
            let checked = selected.iter().any(|item| item == &entry.value);
            row.push(
                container(checkbox(checked).label(entry.label.clone()).on_toggle({
                    let value = entry.value.clone();
                    move |checked| Message::TogglePreset(kind, value.clone(), checked)
                }))
                .width(Length::FillPortion(1)),
            )
        });
        content = content.push(row_widgets);
    }

    content.into()
}

fn render_cached_handle<'a>(handle: Handle, max_width: u16) -> Element<'a, Message> {
    image_view(handle)
        .width(Length::Fixed(max_width as f32))
        .into()
}

fn result_overview<'a>(app: &'a App) -> Element<'a, Message> {
    let recognized = app
        .scan_debug
        .as_ref()
        .map(|debug| debug.slots.iter().filter(|slot| slot.recognized).count())
        .unwrap_or(0);
    let scanned = app
        .scan_debug
        .as_ref()
        .map(|debug| debug.slots.len())
        .unwrap_or(0);
    let result_lines = app
        .scan_result_lines
        .iter()
        .fold(column![text("识别结果")].spacing(6), |column, line| {
            column.push(text(line.clone()))
        });

    column![
        text("识别概览"),
        row![
            metric_card_owned("窗口", app.state.app_settings.selected_window_title.clone()),
            metric_card_owned("扫描槽位", scanned.to_string()),
            metric_card_owned("命中槽位", recognized.to_string()),
            metric_card_owned("状态", status_text(app.status).to_string()),
        ]
        .spacing(12),
        container(result_lines).padding(10),
    ]
    .spacing(12)
    .into()
}

fn slot_debug_card<'a>(
    app: &'a App,
    index: usize,
    slot: &'a crate::domain::SlotDebugInfo,
) -> Element<'a, Message> {
    let title = format!(
        "槽位 {}{}",
        slot.slot,
        if slot.recognized {
            " · 命中"
        } else {
            " · 未命中"
        }
    );

    let mut image_row = row![].spacing(12);
    if let Some(handle) = app
        .cached_debug_images
        .slots
        .get(index)
        .and_then(|slot| slot.price.clone())
    {
        image_row = image_row.push(
            column![
                row![
                    text("价格 ROI"),
                    button("保存").on_press(Message::SaveImage(ImageSaveTarget::SlotPrice(index)))
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                render_cached_handle(handle, 220)
            ]
            .spacing(6),
        );
    }
    if let Some(handle) = app
        .cached_debug_images
        .slots
        .get(index)
        .and_then(|slot| slot.name.clone())
    {
        image_row = image_row.push(
            column![
                row![
                    text("名称 ROI"),
                    button("保存").on_press(Message::SaveImage(ImageSaveTarget::SlotName(index)))
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                render_cached_handle(handle, 320)
            ]
            .spacing(6),
        );
    }

    container(
        column![
            text(title),
            row![
                metric_card_owned("价格 OCR", display_text(&slot.price_ocr)),
                metric_card_owned("名称 OCR", display_text(&slot.name_ocr)),
            ]
            .spacing(12),
            image_row,
        ]
        .spacing(10),
    )
    .padding(10)
    .into()
}

fn metric_card_owned(label: &'static str, value: String) -> Element<'static, Message> {
    container(column![text(label), text(value)].spacing(4))
        .padding(10)
        .width(Length::FillPortion(1))
        .into()
}

fn display_text(value: &str) -> String {
    if value.trim().is_empty() {
        "(空)".to_string()
    } else {
        value.to_string()
    }
}

fn recognized_summary<'a>(debug: &'a crate::domain::ScanDebugResult) -> Element<'a, Message> {
    let recognized: Vec<_> = debug.slots.iter().filter(|slot| slot.recognized).collect();

    let content = if recognized.is_empty() {
        column![
            text("OCR 摘要"),
            text("图例: 红框=价格, 橙框=名称, 灰框=未命中"),
            text("本次暂无识别命中槽位")
        ]
        .spacing(6)
    } else {
        recognized.into_iter().fold(
            column![
                text("OCR 摘要"),
                text("图例: 红框=价格, 橙框=名称, 灰框=未命中")
            ]
            .spacing(6),
            |column, slot| {
                column.push(text(format!(
                    "槽位 {} | 价格 {} | 名称 {}",
                    slot.slot,
                    if slot.price_ocr.is_empty() {
                        "(空)"
                    } else {
                        &slot.price_ocr
                    },
                    if slot.name_ocr.is_empty() {
                        "(空)"
                    } else {
                        &slot.name_ocr
                    }
                )))
            },
        )
    };

    content.into()
}

fn preset_columns(window_width: f32) -> usize {
    if window_width >= 1500.0 {
        4
    } else if window_width >= 1180.0 {
        3
    } else if window_width >= 860.0 {
        2
    } else {
        1
    }
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
            annotated_frame: debug.annotated_frame.as_ref().map(preview_handle),
            recognized_frame: debug.recognized_frame.as_ref().map(preview_handle),
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

fn preview_for_target(app: &App, target: ImageSaveTarget) -> Option<(ImagePreview, String)> {
    let debug = app.scan_debug.as_ref()?;
    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();

    match target {
        ImageSaveTarget::FullFrame => Some((
            debug.full_frame.clone()?,
            format!("{timestamp}_full_frame.png"),
        )),
        ImageSaveTarget::AnnotatedFrame => Some((
            debug.annotated_frame.clone()?,
            format!("{timestamp}_annotated_frame.png"),
        )),
        ImageSaveTarget::RecognizedFrame => Some((
            debug.recognized_frame.clone()?,
            format!("{timestamp}_recognized_frame.png"),
        )),
        ImageSaveTarget::SlotPrice(index) => {
            let slot = debug.slots.get(index)?;
            Some((
                slot.price_roi.clone()?,
                format!("{timestamp}_slot{}_price.png", slot.slot),
            ))
        }
        ImageSaveTarget::SlotName(index) => {
            let slot = debug.slots.get(index)?;
            Some((
                slot.name_roi.clone()?,
                format!("{timestamp}_slot{}_name.png", slot.slot),
            ))
        }
    }
}

fn save_preview_with_dialog(preview: ImagePreview, filename: String) -> anyhow::Result<String> {
    let Some(path) = FileDialog::new()
        .add_filter("PNG image", &["png"])
        .set_file_name(&filename)
        .save_file()
    else {
        return Err(anyhow::anyhow!("__CANCELLED__"));
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let image = RgbaImage::from_raw(preview.width, preview.height, preview.rgba)
        .ok_or_else(|| anyhow::anyhow!("图片数据无效"))?;
    image.save(&path)?;

    Ok(path.display().to_string())
}
