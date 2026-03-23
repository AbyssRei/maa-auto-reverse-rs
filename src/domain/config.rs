use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ControllerKind {
    #[default]
    Win32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UiScale {
    #[default]
    Scale90,
    Scale100,
}

impl UiScale {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scale90 => "90%",
            Self::Scale100 => "100%",
        }
    }
}

impl Display for UiScale {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RuntimeMode {
    #[default]
    AutoReverse,
    RefreshKeep,
}

impl Display for RuntimeMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AutoReverse => f.write_str("自动倒转"),
            Self::RefreshKeep => f.write_str("刷新保留"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub controller: ControllerKind,
    pub selected_window_title: String,
    pub ui_scale: UiScale,
    pub recent_windows: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            controller: ControllerKind::Win32,
            selected_window_title: "明日方舟".to_string(),
            ui_scale: UiScale::Scale90,
            recent_windows: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StrategyConfig {
    pub item_list: Vec<String>,
    pub operator_list: Vec<String>,
    pub buy_only_operator_list: Vec<String>,
    pub six_star_list: Vec<String>,
    pub ocr_correction_map: BTreeMap<String, String>,
    pub change_threshold: f32,
    pub shop_refresh_change_threshold: f32,
    pub stable_threshold: f32,
    pub stable_timeout: f32,
    pub post_action_refresh_wait: f32,
    pub sell_click_wait: f32,
    pub refresh_keep_mode: bool,
    pub auto_reverse_auto_refresh: bool,
    pub ui_scale: UiScale,
    pub double_click_interval: f32,
    pub stable_poll_interval: f32,
    pub action_interval: f32,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            item_list: vec!["人事部文档".to_string()],
            operator_list: Vec::new(),
            buy_only_operator_list: Vec::new(),
            six_star_list: Vec::new(),
            ocr_correction_map: BTreeMap::from([
                ("铜".to_string(), "锏".to_string()),
                ("湖".to_string(), "溯".to_string()),
            ]),
            change_threshold: 5.0,
            shop_refresh_change_threshold: 8.0,
            stable_threshold: 2.0,
            stable_timeout: 2.0,
            post_action_refresh_wait: 0.4,
            sell_click_wait: 0.03,
            refresh_keep_mode: false,
            auto_reverse_auto_refresh: false,
            ui_scale: UiScale::Scale90,
            double_click_interval: 0.01,
            stable_poll_interval: 0.1,
            action_interval: 0.1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PresetEntry {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PresetCatalog {
    pub predefined_items: Vec<PresetEntry>,
    pub predefined_buy_only_operators: Vec<PresetEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PersistedState {
    pub app_settings: AppSettings,
    pub strategy_config: StrategyConfig,
    pub presets: PresetCatalog,
}

#[derive(Debug, Clone, Default)]
pub struct EditableLists {
    pub items: String,
    pub operators: String,
    pub buy_only_operators: String,
    pub six_star_operators: String,
}

impl EditableLists {
    pub fn from_strategy(strategy: &StrategyConfig) -> Self {
        Self {
            items: strategy.item_list.join("、"),
            operators: strategy.operator_list.join("、"),
            buy_only_operators: strategy.buy_only_operator_list.join("、"),
            six_star_operators: strategy.six_star_list.join("、"),
        }
    }

    pub fn apply_to(&self, strategy: &mut StrategyConfig) {
        strategy.item_list = parse_name_list(&self.items);
        strategy.operator_list = parse_name_list(&self.operators);
        strategy.buy_only_operator_list = parse_name_list(&self.buy_only_operators);
        strategy.six_star_list = parse_name_list(&self.six_star_operators);
    }
}

pub fn parse_name_list(text: &str) -> Vec<String> {
    static SPLITTER: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let splitter = SPLITTER.get_or_init(|| Regex::new(r"[，,；;、\n\r]+").expect("regex"));

    splitter
        .split(text.trim())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_name_list_with_multiple_delimiters() {
        assert_eq!(
            parse_name_list("德克萨斯， 宴; Sharp、\n锏"),
            vec!["德克萨斯", "宴", "Sharp", "锏"]
        );
    }
}
