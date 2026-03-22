use crate::domain::{
    AppSettings, PersistedState, PresetCatalog, PresetEntry, StrategyConfig, UiScale,
};
use crate::infra::paths;
use anyhow::{Context, Result};
use ron::ser::PrettyConfig;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeMap;
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

    let imported = import_legacy_state()?;
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

fn import_legacy_state() -> Result<PersistedState> {
    let paths = paths::app_paths()?;

    let app_settings = if paths.legacy_config_dir.join("maa_option.json").exists() {
        let json = read_json::<Value>(&paths.legacy_config_dir.join("maa_option.json"))?;
        AppSettings {
            selected_window_title: json
                .get("window_title")
                .and_then(Value::as_str)
                .unwrap_or("明日方舟")
                .to_string(),
            controller: crate::domain::ControllerKind::Win32,
            ui_scale: UiScale::Scale90,
            recent_windows: Vec::new(),
        }
    } else {
        AppSettings::default()
    };

    let default_json = if paths
        .legacy_root
        .join("autoreverse")
        .join("config.default.json")
        .exists()
    {
        read_json::<Value>(
            &paths
                .legacy_root
                .join("autoreverse")
                .join("config.default.json"),
        )?
    } else {
        Value::Null
    };

    let strategy_config = StrategyConfig {
        item_list: read_string_list(&paths.legacy_config_dir.join("buy_items.json"))?,
        operator_list: read_string_list(&paths.legacy_config_dir.join("buy_sell_operators.json"))?,
        buy_only_operator_list: read_string_list(
            &paths.legacy_config_dir.join("buy_only_operators.json"),
        )?,
        six_star_list: read_string_list(&paths.legacy_config_dir.join("six_star_operators.json"))?,
        ocr_correction_map: read_string_map(&default_json, "ocr_correction_map")
            .unwrap_or_else(|| StrategyConfig::default().ocr_correction_map),
        change_threshold: read_f32(&default_json, "change_threshold", 5.0),
        shop_refresh_change_threshold: read_f32(
            &default_json,
            "shop_refresh_change_threshold",
            15.0,
        ),
        stable_threshold: read_f32(&default_json, "stable_threshold", 2.0),
        stable_timeout: read_f32(&default_json, "stable_timeout", 2.0),
        post_action_refresh_wait: read_f32(&default_json, "post_action_refresh_wait", 0.4),
        sell_click_wait: read_f32(&default_json, "sell_click_wait", 0.03),
        refresh_keep_mode: false,
        ui_scale: UiScale::Scale90,
    };

    let presets = PresetCatalog {
        predefined_items: read_preset_entries(
            &paths.legacy_config_dir.join("predefined_items.json"),
        )?,
        predefined_buy_only_operators: read_preset_entries(
            &paths
                .legacy_config_dir
                .join("predefined_buy_only_operators.json"),
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
    read_json(path)
}

fn read_string_map(json: &Value, key: &str) -> Option<BTreeMap<String, String>> {
    let obj = json.get(key)?.as_object()?;
    Some(
        obj.iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect(),
    )
}

fn read_f32(json: &Value, key: &str, default: f32) -> f32 {
    json.get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .unwrap_or(default)
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
