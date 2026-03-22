use image::imageops::{FilterType, crop_imm, resize};
use image::{DynamicImage, GenericImageView, GrayImage, ImageBuffer, Luma, RgbaImage};
use imageproc::contrast::{ThresholdType, threshold};
use ndarray::Array3;
use serde::{Deserialize, Serialize};

use crate::domain::UiScale;

pub type RelativeRoi = (f32, f32, f32, f32);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagePreview {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl ImagePreview {
    pub fn from_dynamic(image: &DynamicImage) -> Self {
        let rgba = image.to_rgba8();
        Self {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotDebugInfo {
    pub slot: usize,
    pub price_ocr: String,
    pub name_ocr: String,
    pub price_roi: Option<ImagePreview>,
    pub name_roi: Option<ImagePreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanDebugResult {
    pub full_frame: Option<ImagePreview>,
    pub slots: Vec<SlotDebugInfo>,
}

#[derive(Debug, Clone, Copy)]
pub struct RoiTemplateSet {
    pub remaining_funds_roi: RelativeRoi,
    pub price_rois: [RelativeRoi; 6],
    pub name_rois: [RelativeRoi; 6],
    pub max_card_roi: RelativeRoi,
    pub hand_area_roi: RelativeRoi,
    pub shop_display_roi: RelativeRoi,
}

pub const ROI_90: RoiTemplateSet = RoiTemplateSet {
    remaining_funds_roi: (0.9381, 0.7481, 0.0240, 0.0342),
    price_rois: [
        (0.8141, 0.7704, 0.0100, 0.0232),
        (0.6990, 0.7704, 0.0100, 0.0232),
        (0.5839, 0.7704, 0.0100, 0.0232),
        (0.4688, 0.7704, 0.0100, 0.0232),
        (0.3537, 0.7704, 0.0100, 0.0232),
        (0.2386, 0.7704, 0.0100, 0.0232),
    ],
    name_rois: [
        (0.7807, 0.9556, 0.0818, 0.0250),
        (0.6656, 0.9556, 0.0818, 0.0250),
        (0.5505, 0.9556, 0.0818, 0.0250),
        (0.4354, 0.9556, 0.0818, 0.0250),
        (0.3203, 0.9556, 0.0818, 0.0250),
        (0.2052, 0.9556, 0.0818, 0.0250),
    ],
    max_card_roi: (0.3625, 0.7037, 0.2740, 0.0435),
    hand_area_roi: (0.1250, 0.6324, 0.6594, 0.1046),
    shop_display_roi: (0.1901, 0.7685, 0.6844, 0.2204),
};

pub const ROI_100: RoiTemplateSet = RoiTemplateSet {
    remaining_funds_roi: (0.9313, 0.7204, 0.0266, 0.0370),
    price_rois: [
        (0.7927, 0.7444, 0.0120, 0.0269),
        (0.6650, 0.7444, 0.0120, 0.0269),
        (0.5373, 0.7444, 0.0120, 0.0269),
        (0.4095, 0.7444, 0.0120, 0.0269),
        (0.2818, 0.7444, 0.0120, 0.0269),
        (0.1541, 0.7444, 0.0120, 0.0269),
    ],
    name_rois: [
        (0.7568, 0.9500, 0.0896, 0.0296),
        (0.6289, 0.9500, 0.0896, 0.0296),
        (0.5010, 0.9500, 0.0896, 0.0296),
        (0.3730, 0.9500, 0.0896, 0.0296),
        (0.2451, 0.9500, 0.0896, 0.0296),
        (0.1172, 0.9500, 0.0896, 0.0296),
    ],
    max_card_roi: (0.3460, 0.6687, 0.3073, 0.0514),
    hand_area_roi: (0.1250, 0.6324, 0.6594, 0.1046),
    shop_display_roi: (0.0995, 0.7435, 0.7599, 0.2444),
};

pub fn roi_set(scale: UiScale) -> RoiTemplateSet {
    match scale {
        UiScale::Scale90 => ROI_90,
        UiScale::Scale100 => ROI_100,
    }
}

pub fn crop_relative(image: &DynamicImage, roi: RelativeRoi) -> DynamicImage {
    let (w, h) = image.dimensions();
    let x = (roi.0 * w as f32).max(0.0) as u32;
    let y = (roi.1 * h as f32).max(0.0) as u32;
    let width = (roi.2 * w as f32).max(1.0) as u32;
    let height = (roi.3 * h as f32).max(1.0) as u32;
    crop_imm(
        image,
        x,
        y,
        width.min(w.saturating_sub(x)),
        height.min(h.saturating_sub(y)),
    )
    .to_image()
    .into()
}

pub fn center_of_roi(image: &DynamicImage, roi: RelativeRoi) -> (i32, i32) {
    let (w, h) = image.dimensions();
    let cx = ((roi.0 + roi.2 / 2.0) * w as f32) as i32;
    let cy = ((roi.1 + roi.3 / 2.0) * h as f32) as i32;
    (cx, cy)
}

pub fn preprocess_roi(image: &DynamicImage, is_number: bool) -> DynamicImage {
    let mut gray = image.to_luma8();
    let scale = if is_number { 3.0 } else { 2.0 };
    gray = resize(
        &gray,
        (gray.width() as f32 * scale) as u32,
        (gray.height() as f32 * scale) as u32,
        FilterType::CatmullRom,
    );

    if mean_gray(&gray) < 100.0 {
        for pixel in gray.pixels_mut() {
            pixel.0[0] = 255 - pixel.0[0];
        }
    }

    if is_number {
        gray = threshold(&gray, 150, ThresholdType::Binary);
    }

    let bordered = add_white_border(&gray, 10);
    DynamicImage::ImageRgb8(DynamicImage::ImageLuma8(bordered).to_rgb8())
}

pub fn mean_gray(image: &GrayImage) -> f32 {
    let sum: u64 = image.pixels().map(|pixel| pixel.0[0] as u64).sum();
    sum as f32 / image.pixels().len() as f32
}

pub fn add_white_border(image: &GrayImage, border: u32) -> GrayImage {
    let mut out = ImageBuffer::from_pixel(
        image.width() + border * 2,
        image.height() + border * 2,
        Luma([255]),
    );
    image::imageops::replace(&mut out, image, border as i64, border as i64);
    out
}

pub fn has_image_changed(before: &DynamicImage, after: &DynamicImage, threshold: f32) -> bool {
    let before = resize(&before.to_rgba8(), 64, 64, FilterType::Triangle);
    let after = resize(&after.to_rgba8(), 64, 64, FilterType::Triangle);

    let before_array = rgba_to_ndarray(&before);
    let after_array = rgba_to_ndarray(&after);
    let diff = (&before_array - &after_array).mapv(|value| value.abs());
    let mean = diff.sum() / diff.len() as f32;

    mean > threshold
}

pub fn rgba_to_ndarray(image: &RgbaImage) -> Array3<f32> {
    let raw = image.as_raw();
    let mut array = Array3::zeros((image.height() as usize, image.width() as usize, 4));

    for (index, value) in raw.iter().enumerate() {
        let pixel = index / 4;
        let channel = index % 4;
        let x = pixel % image.width() as usize;
        let y = pixel / image.width() as usize;
        array[(y, x, channel)] = *value as f32;
    }

    array
}

pub fn find_hand_change_center(before: &DynamicImage, after: &DynamicImage) -> Option<f32> {
    let before = before.to_rgba8();
    let after = after.to_rgba8();

    if before.dimensions() != after.dimensions() {
        return None;
    }

    let width = before.width() as usize;
    let height = before.height() as usize;
    if width == 0 || height == 0 {
        return None;
    }

    let slots = 10;
    let slot_width = width as f32 / slots as f32;
    let mut best_score = 0usize;
    let mut best_idx = None;

    for slot in 0..slots {
        let start = (slot as f32 * slot_width) as usize;
        let end = if slot == slots - 1 {
            width
        } else {
            ((slot as f32 + 1.0) * slot_width) as usize
        };

        let mut score = 0usize;
        for y in 0..height {
            for x in start..end {
                let a = before.get_pixel(x as u32, y as u32);
                let b = after.get_pixel(x as u32, y as u32);
                let delta =
                    a.0.iter()
                        .zip(b.0.iter())
                        .map(|(lhs, rhs)| lhs.abs_diff(*rhs) as usize)
                        .sum::<usize>();
                if delta > 60 {
                    score += 1;
                }
            }
        }

        if score > best_score {
            best_score = score;
            best_idx = Some(slot);
        }
    }

    if best_score <= 50 {
        return None;
    }

    best_idx.map(|slot| {
        let start = slot as f32 * slot_width;
        let end = if slot == slots - 1 {
            width as f32
        } else {
            (slot as f32 + 1.0) * slot_width
        };
        start + (end - start) / 2.0
    })
}
