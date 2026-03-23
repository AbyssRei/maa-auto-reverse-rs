use crate::domain::{
    AppSettings, PersistedState, PresetCatalog, PresetEntry, StrategyConfig, UiScale,
};
use crate::infra::paths;
use anyhow::{Context, Result};
use ron::ser::PrettyConfig;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::Path;

pub fn load_or_import_state() -> Result<PersistedState> {
    let app_settings_path = paths::file_in_data("app_settings.ron")?;
    let strategy_path = paths::file_in_data("strategy_config.ron")?;
    let presets_path = paths::file_in_data("presets.ron")?;

    if app_settings_path.exists() && strategy_path.exists() && presets_path.exists() {
        return Ok(PersistedState {
            app_settings: read_ron(&app_settings_path)?,
            strategy_config: read_ron(&strategy_path)?,
            presets: read_ron(&presets_path)?,
        });
    }

    let imported = import_local_state()?;
    save_state(&imported)?;
    Ok(imported)
}

pub fn save_state(state: &PersistedState) -> Result<()> {
    write_ron(
        &paths::file_in_data("app_settings.ron")?,
        &state.app_settings,
    )?;
    write_ron(
        &paths::file_in_data("strategy_config.ron")?,
        &state.strategy_config,
    )?;
    write_ron(&paths::file_in_data("presets.ron")?, &state.presets)?;
    Ok(())
}

fn import_local_state() -> Result<PersistedState> {
    let paths = paths::app_paths()?;
    let maa_option_path = paths.config_dir.join("maa_option.json");
    let advanced_config_path = paths.config_dir.join("advanced_config.json");

    let app_settings = if maa_option_path.exists() {
        let json = read_json::<Value>(&maa_option_path)?;
        let ui_scale = read_advanced_ui_scale(&advanced_config_path).unwrap_or(UiScale::Scale90);
        AppSettings {
            selected_window_title: json
                .get("window_title")
                .and_then(Value::as_str)
                .unwrap_or("明日方舟")
                .to_string(),
            controller: crate::domain::ControllerKind::Win32,
            ui_scale,
            recent_windows: Vec::new(),
        }
    } else {
        AppSettings::default()
    };

    let mut strategy_config = StrategyConfig::default();
    strategy_config.item_list = read_string_list(&paths.config_dir.join("buy_items.json"))?;
    strategy_config.operator_list =
        read_string_list(&paths.config_dir.join("buy_sell_operators.json"))?;
    strategy_config.buy_only_operator_list =
        read_string_list(&paths.config_dir.join("buy_only_operators.json"))?;
    strategy_config.six_star_list =
        read_string_list(&paths.config_dir.join("six_star_operators.json"))?;

    if advanced_config_path.exists() {
        let json = read_json::<Value>(&advanced_config_path)?;
        strategy_config.ocr_correction_map = json
            .get("ocr_correction_map")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_else(|| strategy_config.ocr_correction_map.clone());
        strategy_config.change_threshold =
            read_f32(&json, "change_threshold").unwrap_or(strategy_config.change_threshold);
        strategy_config.shop_refresh_change_threshold =
            read_f32(&json, "shop_refresh_change_threshold")
                .unwrap_or(strategy_config.shop_refresh_change_threshold);
        strategy_config.stable_threshold =
            read_f32(&json, "stable_threshold").unwrap_or(strategy_config.stable_threshold);
        strategy_config.stable_timeout =
            read_f32(&json, "stable_timeout").unwrap_or(strategy_config.stable_timeout);
        strategy_config.post_action_refresh_wait = read_f32(&json, "post_action_refresh_wait")
            .unwrap_or(strategy_config.post_action_refresh_wait);
        strategy_config.sell_click_wait =
            read_f32(&json, "sell_click_wait").unwrap_or(strategy_config.sell_click_wait);
        strategy_config.double_click_interval = read_f32(&json, "double_click_interval")
            .unwrap_or(strategy_config.double_click_interval);
        strategy_config.stable_poll_interval =
            read_f32(&json, "stable_poll_interval").unwrap_or(strategy_config.stable_poll_interval);
        strategy_config.action_interval =
            read_f32(&json, "action_interval").unwrap_or(strategy_config.action_interval);
        strategy_config.ui_scale =
            read_advanced_ui_scale(&advanced_config_path).unwrap_or(strategy_config.ui_scale);
    }

    let presets = PresetCatalog {
        predefined_items: read_preset_entries(&paths.config_dir.join("predefined_items.json"))?,
        predefined_buy_only_operators: read_preset_entries(
            &paths.config_dir.join("predefined_buy_only_operators.json"),
        )?,
    };

    Ok(PersistedState {
        app_settings,
        strategy_config,
        presets,
    })
}

fn read_preset_entries(path: &Path) -> Result<Vec<PresetEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let json = read_json::<Value>(path)?;
    let entries = json
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(PresetEntry {
                label: entry.get("label")?.as_str()?.to_string(),
                value: entry.get("value")?.as_str()?.to_string(),
            })
        })
        .collect();
    Ok(entries)
}

fn read_string_list(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let json = read_json::<Value>(path)?;
    Ok(json
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(ToString::to_string))
        .collect())
}

fn read_f32(json: &Value, key: &str) -> Option<f32> {
    json.get(key).and_then(|value| match value {
        Value::Number(number) => number.as_f64().map(|value| value as f32),
        Value::String(text) => text.parse::<f32>().ok(),
        _ => None,
    })
}

fn read_advanced_ui_scale(path: &Path) -> Option<UiScale> {
    let json = read_json::<Value>(path).ok()?;
    match json.get("ui_scale").and_then(Value::as_str) {
        Some("100%") => Some(UiScale::Scale100),
        Some("90%") => Some(UiScale::Scale90),
        _ => None,
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("读取 JSON 失败: {}", path.display()))?;
    Ok(serde_json::from_str(&raw)?)
}

fn read_ron<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("读取 RON 失败: {}", path.display()))?;
    Ok(ron::from_str(&raw)?)
}

fn write_ron<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let pretty = PrettyConfig::new().depth_limit(4);
    let raw = ron::ser::to_string_pretty(value, pretty)?;
    std::fs::write(path, raw).with_context(|| format!("写入 RON 失败: {}", path.display()))?;
    Ok(())
}
