use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use strsim::normalized_levenshtein;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecognizedCard {
    pub slot: usize,
    pub name: String,
    pub price: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlannedActionKind {
    BuyItem,
    BuyOnlyOperator,
    BuySellOperator,
    BuySellCheapOperator,
}

impl Display for PlannedActionKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BuyItem => f.write_str("购买道具"),
            Self::BuyOnlyOperator => f.write_str("保留干员"),
            Self::BuySellOperator => f.write_str("倒转干员"),
            Self::BuySellCheapOperator => f.write_str("低费倒转"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedAction {
    pub kind: PlannedActionKind,
    pub slot: usize,
    pub name: String,
    pub price: i32,
}

pub fn normalize_text(
    text: &str,
    correction_map: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut cleaned = text.trim().to_string();
    if cleaned.is_empty() {
        return cleaned;
    }

    if let Some(full) = correction_map.get(&cleaned) {
        cleaned = full.clone();
    }

    for (wrong, right) in correction_map {
        if cleaned.contains(wrong) {
            cleaned = cleaned.replace(wrong, right);
        }
    }

    cleaned
}

pub fn is_list_match(
    ocr_text: &str,
    target_list: &[String],
    correction_map: &std::collections::BTreeMap<String, String>,
) -> bool {
    let normalized = normalize_text(ocr_text, correction_map);
    if normalized.is_empty() {
        return false;
    }

    let normalized_nospace = normalized.replace(' ', "");

    target_list.iter().any(|target| {
        if target.is_empty() {
            return false;
        }

        let target_nospace = target.replace(' ', "");

        if target_nospace == normalized_nospace {
            return true;
        }

        if (target_nospace.len() as isize - normalized_nospace.len() as isize).abs() <= 1 {
            if normalized.contains(target) || normalized_nospace.contains(&target_nospace) {
                return true;
            }

            if target_nospace.len() >= 2 {
                return normalized_levenshtein(&target_nospace, &normalized_nospace) >= 0.6;
            }
        }

        false
    })
}

pub fn classify_action(
    card: &RecognizedCard,
    item_list: &[String],
    operator_list: &[String],
    buy_only_operator_list: &[String],
    six_star_list: &[String],
    correction_map: &std::collections::BTreeMap<String, String>,
) -> Option<PlannedActionKind> {
    if is_list_match(&card.name, item_list, correction_map) {
        return Some(PlannedActionKind::BuyItem);
    }

    if card.slot == 6 {
        return None;
    }

    if is_list_match(&card.name, buy_only_operator_list, correction_map) {
        return Some(PlannedActionKind::BuyOnlyOperator);
    }

    if is_list_match(&card.name, operator_list, correction_map) {
        return Some(PlannedActionKind::BuySellOperator);
    }

    if is_list_match(&card.name, six_star_list, correction_map) {
        return None;
    }

    if matches!(card.price, 0 | 1) {
        return Some(PlannedActionKind::BuySellCheapOperator);
    }

    None
}

pub fn plan_actions(
    cards: &[RecognizedCard],
    item_list: &[String],
    operator_list: &[String],
    buy_only_operator_list: &[String],
    six_star_list: &[String],
    correction_map: &std::collections::BTreeMap<String, String>,
) -> Vec<PlannedAction> {
    let mut actions = cards
        .iter()
        .filter_map(|card| {
            classify_action(
                card,
                item_list,
                operator_list,
                buy_only_operator_list,
                six_star_list,
                correction_map,
            )
            .map(|kind| PlannedAction {
                kind,
                slot: card.slot,
                name: normalize_text(&card.name, correction_map),
                price: card.price,
            })
        })
        .collect::<Vec<_>>();

    actions.sort_by_key(|action| {
        let rank = match action.kind {
            PlannedActionKind::BuyItem => 0,
            PlannedActionKind::BuyOnlyOperator => 1,
            PlannedActionKind::BuySellOperator => 2,
            PlannedActionKind::BuySellCheapOperator => 3,
        };
        (rank, action.price, action.slot)
    });

    actions
}
