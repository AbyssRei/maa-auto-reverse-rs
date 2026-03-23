use crate::domain::config::{RuntimeMode, StrategyConfig, UiScale};
use crate::domain::image_ops::{
    ImagePreview, ScanDebugResult, SlotDebugInfo, annotate_recognized_slots_frame,
    annotate_scan_frame, center_of_roi, crop_relative, find_hand_change_center, has_image_changed,
    preprocess_roi, roi_set,
};
use crate::domain::strategy::{PlannedActionKind, RecognizedCard, plan_actions};
use crate::infra::win_input;
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
    target_hwnd: isize,
}

impl AutoReverseEngine {
    pub fn new(config: StrategyConfig, logger: SharedLogger, target_hwnd: isize) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            logger,
            last_shop_image: None,
            target_hwnd,
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
        let manual_refresh =
            win_input::is_key_pressed('D' as i32) || win_input::is_key_pressed('d' as i32);
        if manual_refresh {
            self.log("检测到 D 键，立即执行新一轮识别");
        }

        let changed = manual_refresh
            || self
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
            if config.auto_reverse_auto_refresh {
                self.log("倒转自动刷新：本轮无可操作目标，按 D 刷新商店");
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
                    let shop_before = crop_relative(
                        &self.capture_frame(controller)?,
                        roi_set(config.ui_scale).shop_display_roi,
                    );
                    self.double_click(controller, x, y)?;
                    thread::sleep(Duration::from_secs_f32(config.post_action_refresh_wait));
                    let after_buy_frame = self.capture_frame(controller)?;
                    let shop_after =
                        crop_relative(&after_buy_frame, roi_set(config.ui_scale).shop_display_roi);
                    let (mut refreshed, changed, checked) = self.eval_shop_refresh(
                        &shop_before,
                        &shop_after,
                        action.slot,
                        config.ui_scale,
                        config.shop_refresh_change_threshold,
                    );
                    if self.is_hand_full(&after_buy_frame, config.ui_scale) && refreshed {
                        self.log(
                            "购买保留类干员后手牌区已满导致的UI大幅置灰异常，已被过滤为假刷新",
                        );
                        refreshed = false;
                    }
                    self.log(format!(
                        "仅购买后刷新检查: slot={}, excluded={:?}, changed={changed}/{checked}, threshold={}, refreshed={refreshed}",
                        action.slot,
                        self.shop_region_index_from_slot(action.slot, config.ui_scale),
                        config.shop_refresh_change_threshold,
                    ));
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

            thread::sleep(Duration::from_secs_f32(config.action_interval));
        }

        if mode == RuntimeMode::RefreshKeep {
            self.log("刷新保留模式：本轮购买完成，按 D 刷新商店");
            self.send_refresh_key(controller)?;
            self.last_shop_image = None;
            return Ok(true);
        }

        if config.auto_reverse_auto_refresh {
            self.log("倒转自动刷新：本轮操作完成，按 D 刷新商店");
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
            annotated_frame: Some(annotate_scan_frame(frame, scale)),
            recognized_frame: None,
            slots: Vec::new(),
        };

        for slot in 1..=6 {
            let price_crop = crop_relative(frame, rois.price_rois[slot - 1]);
            let name_crop = crop_relative(frame, rois.name_rois[slot - 1]);

            let price_text = self.ocr_text(context, &price_crop, true)?;
            let name_text = self.ocr_text(context, &name_crop, false)?;
            self.log(format!(
                "扫描槽位{}: 价格OCR='{}' 名称OCR='{}'",
                slot, price_text, name_text
            ));

            let recognized = !name_text.trim().is_empty();
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
                recognized,
                price_ocr: price_text.clone(),
                name_ocr: name_text.clone(),
                price_roi: Some(ImagePreview::from_dynamic(&price_crop)),
                name_roi: Some(ImagePreview::from_dynamic(&name_crop)),
            });
        }

        debug.recognized_frame = Some(annotate_recognized_slots_frame(frame, scale, &debug.slots));

        Ok((cards, debug))
    }

    fn ocr_text(&self, context: &Context, roi: &DynamicImage, number_only: bool) -> Result<String> {
        let candidates = if number_only {
            vec![preprocess_roi(roi, true), roi.clone()]
        } else {
            vec![roi.clone(), preprocess_roi(roi, false)]
        };

        for candidate in candidates {
            let buffer = maa_buffer_from_bgr_image(&candidate)?;
            let result = context
                .run_recognition_direct(
                    "OCR",
                    &json!({
                        "expected": [],
                        "threshold": 0.0,
                        "replace": [],
                        "only_rec": true
                    })
                    .to_string(),
                    &buffer,
                )?
                .and_then(|detail| ocr_text_from_detail(&detail, number_only));

            if let Some(text) = result.filter(|text| !text.trim().is_empty()) {
                return Ok(text);
            }
        }

        Ok(String::new())
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
        let hand_full_after_buy = self.is_hand_full(&after_buy, config.ui_scale);
        let (mut refreshed_buy, changed_buy, checked_buy) = self.eval_shop_refresh(
            &shop_before,
            &shop_after,
            slot,
            config.ui_scale,
            config.shop_refresh_change_threshold,
        );
        if hand_full_after_buy && refreshed_buy {
            self.log("购买后手牌区满，UI大面积置灰产生的巨大颜色差值被强制拦截为假刷新");
            refreshed_buy = false;
        }
        self.log(format!(
            "购买后刷新检查: 商品{}号, 商店改变{changed_buy}/{checked_buy}, 商店是否刷新={refreshed_buy}",
            slot
        ));
        if refreshed_buy {
            self.log("购买后触发了商店刷新，程序将优先完成本次售卖套现");
        }

        let mut after_frame = after_buy;
        let mut hand_after = crop_relative(&after_frame, rois.hand_area_roi);
        let mut center_x = find_hand_change_center(&hand_before, &hand_after);

        if center_x.is_none() && hand_full_after_buy {
            self.log("购买后手牌已满，继续截图检测手牌变动位置后执行售卖");
            let deadline = Instant::now() + Duration::from_secs_f32(config.stable_timeout.max(0.3));
            while Instant::now() < deadline {
                thread::sleep(Duration::from_millis(100));
                after_frame = self.capture_frame(controller)?;
                hand_after = crop_relative(&after_frame, rois.hand_area_roi);
                center_x = find_hand_change_center(&hand_before, &hand_after);
                if center_x.is_some() {
                    break;
                }
            }
        }

        let Some(center_x) = center_x else {
            self.log("未检测到手牌变化");
            return Ok(false);
        };

        let (w, h) = after_frame.dimensions();
        let abs_x = (rois.hand_area_roi.0 * w as f32 + center_x) as i32;
        let abs_y = ((rois.hand_area_roi.1 + rois.hand_area_roi.3) * h as f32) as i32;

        let mut after_sell = self.capture_frame(controller)?;
        let mut hand_full_after_sell = true;
        for attempt in 0..3 {
            controller.wait(controller.post_click(abs_x, abs_y)?);
            thread::sleep(Duration::from_secs_f32(config.sell_click_wait));
            self.send_key_press(controller, 'X' as i32)?;
            thread::sleep(Duration::from_secs_f32(config.post_action_refresh_wait));
            after_sell = self.capture_frame(controller)?;
            hand_full_after_sell = self.is_hand_full(&after_sell, config.ui_scale);
            if !hand_full_after_sell {
                break;
            }
            if attempt < 2 {
                self.log(format!(
                    "售卖重试 {}/3: 售卖后手牌仍然满载，漏键检测生效，再次对该坐标尝试出售...",
                    attempt + 1
                ));
            }
        }

        let shop_after_sell = crop_relative(&after_sell, rois.shop_display_roi);
        let baseline_shop = if hand_full_after_buy {
            &shop_before
        } else {
            &shop_after
        };
        let (mut refreshed_sell, changed_sell, checked_sell) = self.eval_shop_refresh(
            baseline_shop,
            &shop_after_sell,
            slot,
            config.ui_scale,
            config.shop_refresh_change_threshold,
        );
        if hand_full_after_sell && refreshed_sell {
            self.log("售卖操作结束仍满载（假装卖掉了实际上是UI置灰发红），强制重置为假刷新");
            refreshed_sell = false;
        }
        self.log(format!(
            "售卖后刷新检查: 商品{}号, 商店改变{changed_sell}/{checked_sell}, 商店是否刷新={refreshed_sell}, 售卖后手牌区是否满={hand_full_after_sell}",
            slot
        ));

        let final_refresh_state = refreshed_buy || refreshed_sell;
        if hand_full_after_sell {
            self.log("所有售卖重试结束，手牌区仍满载，请人工留意");
        }
        if final_refresh_state {
            self.log("操作期间检测到商店刷新，即将重新扫描");
        }

        Ok(final_refresh_state)
    }

    fn eval_shop_refresh(
        &self,
        before: &DynamicImage,
        after: &DynamicImage,
        excluded_slot: usize,
        scale: UiScale,
        threshold: f32,
    ) -> (bool, usize, usize) {
        let parts_before = split_into_six(before);
        let parts_after = split_into_six(after);
        let excluded_region = self.shop_region_index_from_slot(excluded_slot, scale);

        let mut changed = 0;
        let mut checked = 0;

        for (index, (lhs, rhs)) in parts_before.iter().zip(parts_after.iter()).enumerate() {
            if excluded_region == Some(index) {
                continue;
            }
            checked += 1;
            if has_image_changed(lhs, rhs, threshold) {
                changed += 1;
            }
        }

        (checked > 0 && changed >= 3, changed, checked)
    }

    fn wait_for_stability(
        &self,
        controller: &Controller,
        config: &StrategyConfig,
    ) -> Result<DynamicImage> {
        let mut last = self.capture_frame(controller)?;
        let start = Instant::now();

        while start.elapsed().as_secs_f32() < config.stable_timeout {
            thread::sleep(Duration::from_secs_f32(config.stable_poll_interval));
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
        let config = self.config.read().expect("config lock");
        thread::sleep(Duration::from_secs_f32(config.double_click_interval));
        controller.wait(controller.post_click(x, y)?);
        Ok(())
    }

    fn send_refresh_key(&self, controller: &Controller) -> Result<()> {
        self.send_key_press(controller, 'D' as i32)
    }

    fn send_key_press(&self, controller: &Controller, keycode: i32) -> Result<()> {
        match win_input::press_key(self.target_hwnd, keycode) {
            Ok(()) => {
                self.log(format!("使用 Win32 SendInput 发送按键: {}", keycode));
            }
            Err(error) => {
                self.log(format!(
                    "Win32 SendInput 发送按键失败，回退到 MAA 键盘通道: {error}"
                ));
                controller.wait(controller.post_key_down(keycode)?);
                thread::sleep(Duration::from_millis(50));
                controller.wait(controller.post_key_up(keycode)?);
            }
        }
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

    fn shop_region_index_from_slot(&self, slot: usize, scale: UiScale) -> Option<usize> {
        if !(1..=6).contains(&slot) {
            return None;
        }

        let rois = roi_set(scale);
        let mut ordered_slots = (0..6)
            .map(|index| {
                let roi = rois.price_rois[index];
                let center_x = roi.0 + roi.2 / 2.0;
                let center_y = roi.1 + roi.3 / 2.0;
                (index + 1, center_x, center_y)
            })
            .collect::<Vec<_>>();
        ordered_slots.sort_by(|lhs, rhs| lhs.1.total_cmp(&rhs.1).then(lhs.2.total_cmp(&rhs.2)));
        ordered_slots
            .iter()
            .enumerate()
            .find_map(|(region, (candidate_slot, _, _))| {
                (*candidate_slot == slot).then_some(region)
            })
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
    let text = detail
        .detail
        .get("best")
        .and_then(|best| best.get("text"))
        .and_then(|text| text.as_str())
        .map(str::to_owned)
        .or_else(|| {
            detail
                .detail
                .get("filtered")
                .and_then(|items| items.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("text").and_then(|text| text.as_str()))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
        })?;

    if number_only {
        Some(
            text.chars()
                .map(|character| match character {
                    'O' | 'o' | 'Q' | 'D' => '0',
                    'I' | 'l' | 'i' => '1',
                    'Z' | 'z' => '2',
                    'S' | 's' => '5',
                    'B' => '8',
                    'b' => '6',
                    'g' | 'q' => '9',
                    other => other,
                })
                .filter(|character| character.is_ascii_digit())
                .collect(),
        )
    } else {
        Some(text.trim().to_string())
    }
}

fn maa_buffer_from_bgr_image(image: &DynamicImage) -> Result<MaaImageBuffer> {
    let rgb = image.to_rgb8();
    let (width, height) = rgb.dimensions();
    let mut bgr = Vec::with_capacity((width * height * 3) as usize);
    for pixel in rgb.pixels() {
        bgr.push(pixel[2]);
        bgr.push(pixel[1]);
        bgr.push(pixel[0]);
    }

    let mut buffer = MaaImageBuffer::new()?;
    buffer.set(&bgr, width as i32, height as i32)?;
    Ok(buffer)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::StrategyConfig;

    fn noop_logger() -> SharedLogger {
        Arc::new(|_| {})
    }

    #[test]
    fn maps_slots_to_left_to_right_shop_regions() {
        let engine = AutoReverseEngine::new(StrategyConfig::default(), noop_logger(), 0);

        assert_eq!(
            engine.shop_region_index_from_slot(6, UiScale::Scale90),
            Some(0)
        );
        assert_eq!(
            engine.shop_region_index_from_slot(5, UiScale::Scale90),
            Some(1)
        );
        assert_eq!(
            engine.shop_region_index_from_slot(4, UiScale::Scale90),
            Some(2)
        );
        assert_eq!(
            engine.shop_region_index_from_slot(3, UiScale::Scale90),
            Some(3)
        );
        assert_eq!(
            engine.shop_region_index_from_slot(2, UiScale::Scale90),
            Some(4)
        );
        assert_eq!(
            engine.shop_region_index_from_slot(1, UiScale::Scale90),
            Some(5)
        );
    }
}
