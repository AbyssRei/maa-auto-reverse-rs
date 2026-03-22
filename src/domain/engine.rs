use crate::domain::config::{RuntimeMode, StrategyConfig, UiScale};
use crate::domain::image_ops::{
    ImagePreview, ScanDebugResult, SlotDebugInfo, center_of_roi, crop_relative,
    find_hand_change_center, has_image_changed, preprocess_roi, roi_set,
};
use crate::domain::strategy::{PlannedActionKind, RecognizedCard, plan_actions};
use anyhow::Result;
use image::{DynamicImage, GenericImageView};
use maa_framework::buffer::MaaImageBuffer;
use maa_framework::common::RecognitionDetail;
use maa_framework::context::Context;
use maa_framework::controller::Controller;
use serde_json::json;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

pub type SharedLogger = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct EngineConfigSnapshot {
    pub strategy: StrategyConfig,
    pub mode: RuntimeMode,
}

pub struct AutoReverseEngine {
    config: Arc<RwLock<StrategyConfig>>,
    logger: SharedLogger,
    last_shop_image: Option<DynamicImage>,
}

impl AutoReverseEngine {
    pub fn new(config: StrategyConfig, logger: SharedLogger) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            logger,
            last_shop_image: None,
        }
    }

    pub fn update_config(&mut self, config: StrategyConfig) {
        if let Ok(mut current) = self.config.write() {
            *current = config;
        }
        self.log("配置已更新");
    }

    pub fn snapshot(&self, mode: RuntimeMode) -> EngineConfigSnapshot {
        EngineConfigSnapshot {
            strategy: self.config.read().expect("config lock").clone(),
            mode,
        }
    }

    pub fn scan_once_debug(
        &mut self,
        context: &Context,
        controller: &Controller,
    ) -> Result<(Vec<RecognizedCard>, ScanDebugResult)> {
        let config = self.config.read().expect("config lock").clone();
        let stable = self.wait_for_stability(controller, &config)?;
        self.scan_cards_with_debug(context, &stable, config.ui_scale)
    }

    pub fn tick(
        &mut self,
        context: &Context,
        controller: &Controller,
        mode: RuntimeMode,
    ) -> Result<bool> {
        let config = self.config.read().expect("config lock").clone();
        let frame = self.capture_frame(controller)?;
        let rois = roi_set(config.ui_scale);
        let current_shop = crop_relative(&frame, rois.shop_display_roi);

        let changed = self
            .last_shop_image
            .as_ref()
            .map(|last| has_image_changed(last, &current_shop, config.change_threshold))
            .unwrap_or(true);

        if !changed {
            return Ok(true);
        }

        let stable = self.wait_for_stability(controller, &config)?;
        if self.is_hand_full(&stable, config.ui_scale) {
            self.log("手牌已满，等待下一轮");
            self.last_shop_image = Some(crop_relative(&stable, rois.shop_display_roi));
            return Ok(true);
        }

        let (cards, _) = self.scan_cards_with_debug(context, &stable, config.ui_scale)?;
        let mut actions = plan_actions(
            &cards,
            &config.item_list,
            &config.operator_list,
            &config.buy_only_operator_list,
            &config.six_star_list,
            &config.ocr_correction_map,
        );

        if mode == RuntimeMode::RefreshKeep {
            actions.retain(|action| {
                matches!(
                    action.kind,
                    PlannedActionKind::BuyItem | PlannedActionKind::BuyOnlyOperator
                )
            });
        }

        if actions.is_empty() {
            if mode == RuntimeMode::RefreshKeep {
                self.log("刷新保留模式：本轮无可购目标，按 D 刷新商店");
                self.send_refresh_key(controller)?;
                self.last_shop_image = None;
                return Ok(true);
            }

            self.last_shop_image = Some(current_shop);
            return Ok(true);
        }

        self.log(format!("本轮计划动作数: {}", actions.len()));
        for action in actions {
            match action.kind {
                PlannedActionKind::BuyItem => {
                    self.log(format!("购买道具: {}", action.name));
                    let (x, y) = center_of_roi(
                        &stable,
                        roi_set(config.ui_scale).price_rois[action.slot - 1],
                    );
                    self.double_click(controller, x, y)?;
                }
                PlannedActionKind::BuyOnlyOperator => {
                    self.log(format!("仅购买干员: {}", action.name));
                    let (x, y) = center_of_roi(
                        &stable,
                        roi_set(config.ui_scale).price_rois[action.slot - 1],
                    );
                    self.double_click(controller, x, y)?;
                    thread::sleep(Duration::from_secs_f32(config.post_action_refresh_wait));
                    let refreshed = self.shop_refreshed_after_action(
                        controller,
                        &stable,
                        config.ui_scale,
                        action.slot,
                        config.shop_refresh_change_threshold,
                    )?;
                    if refreshed {
                        self.log("仅购买后检测到商店刷新");
                        self.last_shop_image = None;
                        return Ok(true);
                    }
                }
                PlannedActionKind::BuySellOperator | PlannedActionKind::BuySellCheapOperator => {
                    self.log(format!("买卖干员: {}", action.name));
                    if self.perform_buy_sell(controller, &stable, action.slot, &config)? {
                        self.last_shop_image = None;
                        return Ok(true);
                    }
                }
            }

            thread::sleep(Duration::from_millis(100));
        }

        if mode == RuntimeMode::RefreshKeep {
            self.log("刷新保留模式：本轮购买完成，按 D 刷新商店");
            self.send_refresh_key(controller)?;
            self.last_shop_image = None;
            return Ok(true);
        }

        self.last_shop_image = Some(crop_relative(
            &self.capture_frame(controller)?,
            rois.shop_display_roi,
        ));
        Ok(true)
    }

    fn scan_cards_with_debug(
        &self,
        context: &Context,
        frame: &DynamicImage,
        scale: UiScale,
    ) -> Result<(Vec<RecognizedCard>, ScanDebugResult)> {
        let rois = roi_set(scale);
        let mut cards = Vec::new();
        let mut debug = ScanDebugResult {
            full_frame: Some(ImagePreview::from_dynamic(frame)),
            slots: Vec::new(),
        };

        for slot in 1..=6 {
            let price_crop = crop_relative(frame, rois.price_rois[slot - 1]);
            let name_crop = crop_relative(frame, rois.name_rois[slot - 1]);

            let price_text = self.ocr_text(context, &price_crop, true)?;
            let name_text = self.ocr_text(context, &name_crop, false)?;

            if !name_text.trim().is_empty() {
                let price = price_text.parse::<i32>().unwrap_or(-1);
                cards.push(RecognizedCard {
                    slot,
                    name: name_text.clone(),
                    price,
                });
            }

            debug.slots.push(SlotDebugInfo {
                slot,
                price_ocr: price_text.clone(),
                name_ocr: name_text.clone(),
                price_roi: Some(ImagePreview::from_dynamic(&price_crop)),
                name_roi: Some(ImagePreview::from_dynamic(&name_crop)),
            });
        }

        Ok((cards, debug))
    }

    fn ocr_text(&self, context: &Context, roi: &DynamicImage, number_only: bool) -> Result<String> {
        let preprocessed = preprocess_roi(roi, number_only);
        let buffer = MaaImageBuffer::from_dynamic_image(&preprocessed)?;
        let result = context
            .run_recognition_direct("OCR", &json!({ "only_rec": true }).to_string(), &buffer)?
            .and_then(|detail| ocr_text_from_detail(&detail, number_only));

        Ok(result.unwrap_or_default())
    }

    fn perform_buy_sell(
        &mut self,
        controller: &Controller,
        stable: &DynamicImage,
        slot: usize,
        config: &StrategyConfig,
    ) -> Result<bool> {
        let rois = roi_set(config.ui_scale);
        let shop_before = crop_relative(stable, rois.shop_display_roi);
        let hand_before = crop_relative(stable, rois.hand_area_roi);
        let (x, y) = center_of_roi(stable, rois.price_rois[slot - 1]);

        self.double_click(controller, x, y)?;
        thread::sleep(Duration::from_secs_f32(config.post_action_refresh_wait));

        let after_buy = self.capture_frame(controller)?;
        let shop_after = crop_relative(&after_buy, rois.shop_display_roi);
        if self.eval_shop_refresh(
            &shop_before,
            &shop_after,
            slot,
            config.ui_scale,
            config.shop_refresh_change_threshold,
        ) {
            if !self.is_hand_full(&after_buy, config.ui_scale) {
                self.log("商店刷新");
                return Ok(true);
            }
            self.log("只剩一格空位了，注意手牌管理");
        }

        let hand_after = crop_relative(&after_buy, rois.hand_area_roi);
        let Some(center_x) = find_hand_change_center(&hand_before, &hand_after) else {
            self.log("未检测到手牌变化");
            return Ok(false);
        };

        let (w, h) = after_buy.dimensions();
        let abs_x = (rois.hand_area_roi.0 * w as f32 + center_x) as i32;
        let abs_y = ((rois.hand_area_roi.1 + rois.hand_area_roi.3 / 2.0) * h as f32) as i32;
        controller.wait(controller.post_click(abs_x, abs_y)?);
        thread::sleep(Duration::from_secs_f32(config.sell_click_wait));
        controller.wait(controller.post_click_key('X' as i32)?);
        thread::sleep(Duration::from_secs_f32(config.post_action_refresh_wait));

        let after_sell = self.capture_frame(controller)?;
        let shop_after_sell = crop_relative(&after_sell, rois.shop_display_roi);
        if self.eval_shop_refresh(
            &shop_after,
            &shop_after_sell,
            slot,
            config.ui_scale,
            config.shop_refresh_change_threshold,
        ) {
            self.log("售卖后检测到商店刷新");
            return Ok(true);
        }

        Ok(false)
    }

    fn shop_refreshed_after_action(
        &self,
        controller: &Controller,
        stable: &DynamicImage,
        scale: UiScale,
        slot: usize,
        threshold: f32,
    ) -> Result<bool> {
        let rois = roi_set(scale);
        let before = crop_relative(stable, rois.shop_display_roi);
        let after = crop_relative(&self.capture_frame(controller)?, rois.shop_display_roi);
        Ok(self.eval_shop_refresh(&before, &after, slot, scale, threshold))
    }

    fn eval_shop_refresh(
        &self,
        before: &DynamicImage,
        after: &DynamicImage,
        excluded_slot: usize,
        scale: UiScale,
        threshold: f32,
    ) -> bool {
        let _ = scale;
        let parts_before = split_into_six(before);
        let parts_after = split_into_six(after);

        let mut changed = 0;
        let mut checked = 0;

        for (index, (lhs, rhs)) in parts_before.iter().zip(parts_after.iter()).enumerate() {
            if index + 1 == excluded_slot {
                continue;
            }
            checked += 1;
            if has_image_changed(lhs, rhs, threshold) {
                changed += 1;
            }
        }

        self.log(format!(
            "刷新检查: 商品{}号, 商店改变{changed}/{checked}, 商店是否刷新={}",
            excluded_slot,
            changed >= 3
        ));

        checked > 0 && changed >= 3
    }

    fn wait_for_stability(
        &self,
        controller: &Controller,
        config: &StrategyConfig,
    ) -> Result<DynamicImage> {
        let mut last = self.capture_frame(controller)?;
        let start = Instant::now();

        while start.elapsed().as_secs_f32() < config.stable_timeout {
            thread::sleep(Duration::from_millis(100));
            let current = self.capture_frame(controller)?;
            if !has_image_changed(&last, &current, config.stable_threshold) {
                return Ok(current);
            }
            last = current;
        }

        Ok(last)
    }

    fn capture_frame(&self, controller: &Controller) -> Result<DynamicImage> {
        controller.wait(controller.post_screencap()?);
        controller
            .cached_image()?
            .to_dynamic_image()
            .map_err(Into::into)
    }

    fn double_click(&self, controller: &Controller, x: i32, y: i32) -> Result<()> {
        controller.wait(controller.post_click(x, y)?);
        thread::sleep(Duration::from_millis(10));
        controller.wait(controller.post_click(x, y)?);
        Ok(())
    }

    fn send_refresh_key(&self, controller: &Controller) -> Result<()> {
        controller.wait(controller.post_click_key('D' as i32)?);
        Ok(())
    }

    fn is_hand_full(&self, frame: &DynamicImage, scale: UiScale) -> bool {
        let roi = crop_relative(frame, roi_set(scale).max_card_roi);
        let image = roi.to_rgba8();
        if image.is_empty() {
            return false;
        }

        let mut r = 0f32;
        let mut g = 0f32;
        let mut b = 0f32;
        for pixel in image.pixels() {
            r += pixel.0[0] as f32;
            g += pixel.0[1] as f32;
            b += pixel.0[2] as f32;
        }
        let count = image.pixels().len() as f32;
        let (h, s, v) = rgb_to_hsv(r / count, g / count, b / count);
        (5.0..=25.0).contains(&h) && s > 0.55 && v > 0.55
            || !(20.0..=340.0).contains(&h) && s > 0.47 && v > 0.47
    }

    fn log(&self, message: impl Into<String>) {
        (self.logger)(message.into());
    }
}

fn split_into_six(image: &DynamicImage) -> Vec<DynamicImage> {
    let image = image.to_rgba8();
    let part_width = image.width() / 6;
    (0..6)
        .map(|index| {
            let x = index * part_width;
            let width = if index == 5 {
                image.width().saturating_sub(x)
            } else {
                part_width
            };
            DynamicImage::ImageRgba8(
                image::imageops::crop_imm(&image, x, 0, width, image.height()).to_image(),
            )
        })
        .collect()
}

fn ocr_text_from_detail(detail: &RecognitionDetail, number_only: bool) -> Option<String> {
    let text = detail.as_ocr_result()?.text;
    if number_only {
        Some(
            text.chars()
                .filter(|character| character.is_ascii_digit())
                .collect(),
        )
    } else {
        Some(text.trim().to_string())
    }
}

fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let r = r / 255.0;
    let g = g / 255.0;
    let b = b / 255.0;

    let max = r.max(g.max(b));
    let min = r.min(g.min(b));
    let delta = max - min;

    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    (hue, saturation, max)
}
