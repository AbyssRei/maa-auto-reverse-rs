use ab_glyph::{FontArc, PxScale};
use image::imageops::{FilterType, crop_imm, resize};
use image::{DynamicImage, GenericImageView, GrayImage, ImageBuffer, Luma, Rgba, RgbaImage};
use imageproc::contrast::{ThresholdType, threshold};
use imageproc::drawing::{
    draw_filled_rect_mut, draw_hollow_rect_mut, draw_line_segment_mut, draw_text_mut,
};
use imageproc::rect::Rect;
use ndarray::Array3;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;

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
    pub recognized: bool,
    pub price_ocr: String,
    pub name_ocr: String,
    pub price_roi: Option<ImagePreview>,
    pub name_roi: Option<ImagePreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanDebugResult {
    pub full_frame: Option<ImagePreview>,
    pub annotated_frame: Option<ImagePreview>,
    pub recognized_frame: Option<ImagePreview>,
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
    hand_area_roi: (0.1234, 0.5907, 0.6599, 0.1102),
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
    hand_area_roi: (0.1240, 0.5824, 0.6594, 0.0889),
    shop_display_roi: (0.0995, 0.7435, 0.7599, 0.2444),
};

pub fn roi_set(scale: UiScale) -> RoiTemplateSet {
    match scale {
        UiScale::Scale90 => ROI_90,
        UiScale::Scale100 => ROI_100,
    }
}

pub fn annotate_scan_frame(image: &DynamicImage, scale: UiScale) -> ImagePreview {
    let mut canvas = image.to_rgba8();
    let rois = roi_set(scale);

    draw_roi_with_label(
        &mut canvas,
        rois.remaining_funds_roi,
        0,
        Rgba([255, 196, 61, 255]),
    );

    for (index, roi) in rois.price_rois.iter().enumerate() {
        draw_roi_with_label(&mut canvas, *roi, index + 1, Rgba([47, 184, 106, 255]));
    }

    for (index, roi) in rois.name_rois.iter().enumerate() {
        draw_roi_with_label(&mut canvas, *roi, index + 1, Rgba([59, 130, 246, 255]));
    }

    ImagePreview::from_dynamic(&DynamicImage::ImageRgba8(canvas))
}

pub fn annotate_recognized_slots_frame(
    image: &DynamicImage,
    scale: UiScale,
    slots: &[SlotDebugInfo],
) -> ImagePreview {
    let mut canvas = image.to_rgba8();
    let rois = roi_set(scale);
    let roi_obstacles = collect_roi_obstacles(&canvas, rois);
    let mut label_obstacles = vec![Rect::at(14, 14).of_size(158, 62)];

    draw_legend(&mut canvas);
    draw_roi_with_label(
        &mut canvas,
        rois.remaining_funds_roi,
        0,
        Rgba([255, 196, 61, 255]),
    );

    for slot in slots.iter().filter(|slot| !slot.recognized) {
        let index = slot.slot.saturating_sub(1);
        if let Some(price_roi) = rois.price_rois.get(index) {
            draw_muted_rect(&mut canvas, *price_roi, Rgba([148, 163, 184, 255]));
        }
        if let Some(name_roi) = rois.name_rois.get(index) {
            draw_muted_rect(&mut canvas, *name_roi, Rgba([100, 116, 139, 255]));
        }
    }

    for slot in slots.iter().filter(|slot| slot.recognized) {
        let index = slot.slot.saturating_sub(1);
        if let Some(price_roi) = rois.price_rois.get(index) {
            draw_emphasis_rect(&mut canvas, *price_roi, Rgba([255, 99, 71, 255]));
            draw_corner_markers(&mut canvas, *price_roi, Rgba([255, 99, 71, 255]));
            draw_roi_with_label(&mut canvas, *price_roi, slot.slot, Rgba([255, 99, 71, 255]));
            draw_text_tag(
                &mut canvas,
                *price_roi,
                &short_price_text(&slot.price_ocr),
                Rgba([255, 99, 71, 255]),
                TextAnchor::Above,
                &roi_obstacles,
                &mut label_obstacles,
            );
        }
        if let Some(name_roi) = rois.name_rois.get(index) {
            draw_emphasis_rect(&mut canvas, *name_roi, Rgba([249, 115, 22, 255]));
            draw_corner_markers(&mut canvas, *name_roi, Rgba([249, 115, 22, 255]));
            draw_text_tag(
                &mut canvas,
                *name_roi,
                &short_name_text(&slot.name_ocr),
                Rgba([249, 115, 22, 255]),
                TextAnchor::Below,
                &roi_obstacles,
                &mut label_obstacles,
            );
        }
    }

    ImagePreview::from_dynamic(&DynamicImage::ImageRgba8(canvas))
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

fn draw_roi_with_label(image: &mut RgbaImage, roi: RelativeRoi, label: usize, color: Rgba<u8>) {
    let rect = roi_to_rect(image.width(), image.height(), roi);
    draw_hollow_rect_mut(image, rect, color);

    let label_rect = Rect::at(rect.left(), (rect.top() - 18).max(0)).of_size(18, 18);
    draw_filled_rect_mut(image, label_rect, Rgba([16, 18, 27, 255]));
    draw_digit(
        image,
        label_rect.left() + 4,
        label_rect.top() + 3,
        label % 10,
        color,
    );
}

fn draw_emphasis_rect(image: &mut RgbaImage, roi: RelativeRoi, color: Rgba<u8>) {
    let rect = roi_to_rect(image.width(), image.height(), roi);
    draw_hollow_rect_mut(image, rect, color);
    let outer =
        Rect::at(rect.left() - 1, rect.top() - 1).of_size(rect.width() + 2, rect.height() + 2);
    draw_hollow_rect_mut(image, outer, color);
}

fn draw_muted_rect(image: &mut RgbaImage, roi: RelativeRoi, color: Rgba<u8>) {
    let rect = roi_to_rect(image.width(), image.height(), roi);
    draw_hollow_rect_mut(image, rect, color);
}

fn draw_corner_markers(image: &mut RgbaImage, roi: RelativeRoi, color: Rgba<u8>) {
    let rect = roi_to_rect(image.width(), image.height(), roi);
    let marker = 6;

    draw_filled_rect_mut(
        image,
        Rect::at(rect.left() - 1, rect.top() - 1).of_size(marker, 2),
        color,
    );
    draw_filled_rect_mut(
        image,
        Rect::at(rect.left() - 1, rect.top() - 1).of_size(2, marker),
        color,
    );

    draw_filled_rect_mut(
        image,
        Rect::at(rect.right() - marker as i32 + 1, rect.top() - 1).of_size(marker, 2),
        color,
    );
    draw_filled_rect_mut(
        image,
        Rect::at(rect.right(), rect.top() - 1).of_size(2, marker),
        color,
    );

    draw_filled_rect_mut(
        image,
        Rect::at(rect.left() - 1, rect.bottom() - 1).of_size(2, marker),
        color,
    );
    draw_filled_rect_mut(
        image,
        Rect::at(rect.left() - 1, rect.bottom() + 1).of_size(marker, 2),
        color,
    );

    draw_filled_rect_mut(
        image,
        Rect::at(rect.right(), rect.bottom() - 1).of_size(2, marker),
        color,
    );
    draw_filled_rect_mut(
        image,
        Rect::at(rect.right() - marker as i32 + 1, rect.bottom() + 1).of_size(marker, 2),
        color,
    );
}

fn draw_legend(image: &mut RgbaImage) {
    draw_filled_rect_mut(
        image,
        Rect::at(14, 14).of_size(158, 62),
        Rgba([15, 23, 42, 220]),
    );
    draw_filled_rect_mut(
        image,
        Rect::at(24, 24).of_size(14, 14),
        Rgba([255, 99, 71, 255]),
    );
    draw_digit(image, 46, 25, 1, Rgba([255, 99, 71, 255]));

    draw_filled_rect_mut(
        image,
        Rect::at(24, 46).of_size(14, 14),
        Rgba([249, 115, 22, 255]),
    );
    draw_digit(image, 46, 47, 2, Rgba([249, 115, 22, 255]));

    draw_filled_rect_mut(
        image,
        Rect::at(96, 24).of_size(14, 14),
        Rgba([148, 163, 184, 255]),
    );
    draw_digit(image, 118, 25, 0, Rgba([148, 163, 184, 255]));
}

#[derive(Clone, Copy)]
enum TextAnchor {
    Above,
    Below,
}

fn draw_text_tag(
    image: &mut RgbaImage,
    roi: RelativeRoi,
    text: &str,
    color: Rgba<u8>,
    anchor: TextAnchor,
    roi_obstacles: &[Rect],
    label_obstacles: &mut Vec<Rect>,
) {
    if text.is_empty() {
        return;
    }

    let Some(font) = overlay_font() else {
        return;
    };

    let rect = roi_to_rect(image.width(), image.height(), roi);
    let char_count = text.chars().count().max(1) as u32;
    let width = (char_count * 14 + 14).min(image.width().saturating_sub(8));
    let height = 22u32;
    let tag_rect = find_text_tag_rect(
        image,
        rect,
        width,
        height,
        anchor,
        roi_obstacles,
        label_obstacles,
    );
    let x = tag_rect.left() as u32;
    let y = tag_rect.top() as u32;

    draw_filled_rect_mut(image, tag_rect, Rgba([15, 23, 42, 230]));
    draw_hollow_rect_mut(image, tag_rect, color);
    draw_connector_line(
        image,
        rect,
        tag_rect,
        color,
        roi_obstacles,
        label_obstacles,
        expand_rect(rect, 4),
    );
    draw_text_mut(
        image,
        color,
        x as i32 + 6,
        y as i32 + 3,
        PxScale::from(15.0),
        font,
        text,
    );
    label_obstacles.push(tag_rect);
}

fn draw_connector_line(
    image: &mut RgbaImage,
    roi_rect: Rect,
    tag_rect: Rect,
    color: Rgba<u8>,
    roi_obstacles: &[Rect],
    label_obstacles: &[Rect],
    source_obstacle: Rect,
) {
    let (start_x, start_y) = rect_center(roi_rect);
    let (end_x, end_y) = nearest_point_on_rect(tag_rect, start_x, start_y);

    let direct = [((start_x, start_y), (end_x, end_y))];
    if path_is_clear(&direct, roi_obstacles, label_obstacles, source_obstacle) {
        draw_segments(image, &direct, color);
        return;
    }

    let elbow_candidates = [
        (start_x, end_y),
        (end_x, start_y),
        (start_x, start_y + ((end_y - start_y) / 2)),
        (start_x + ((end_x - start_x) / 2), end_y),
    ];

    for elbow in elbow_candidates {
        let path = [((start_x, start_y), elbow), (elbow, (end_x, end_y))];
        if path_is_clear(&path, roi_obstacles, label_obstacles, source_obstacle) {
            draw_segments(image, &path, color);
            return;
        }
    }

    draw_segments(image, &direct, color);
}

fn draw_segments(image: &mut RgbaImage, segments: &[((i32, i32), (i32, i32))], color: Rgba<u8>) {
    for ((x1, y1), (x2, y2)) in segments {
        draw_line_segment_mut(
            image,
            (*x1 as f32, *y1 as f32),
            (*x2 as f32, *y2 as f32),
            color,
        );
    }
}

fn path_is_clear(
    segments: &[((i32, i32), (i32, i32))],
    roi_obstacles: &[Rect],
    label_obstacles: &[Rect],
    source_obstacle: Rect,
) -> bool {
    segments.iter().all(|(start, end)| {
        !roi_obstacles
            .iter()
            .filter(|rect| **rect != source_obstacle)
            .any(|rect| segment_intersects_rect(*start, *end, expand_rect(*rect, 3)))
            && !label_obstacles
                .iter()
                .any(|rect| segment_intersects_rect(*start, *end, expand_rect(*rect, 3)))
    })
}

fn segment_intersects_rect(start: (i32, i32), end: (i32, i32), rect: Rect) -> bool {
    let steps = ((start.0 - end.0).abs().max((start.1 - end.1).abs())).max(1);

    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let x = start.0 as f32 + (end.0 - start.0) as f32 * t;
        let y = start.1 as f32 + (end.1 - start.1) as f32 * t;
        if point_in_rect((x as i32, y as i32), rect) {
            return true;
        }
    }

    false
}

fn point_in_rect(point: (i32, i32), rect: Rect) -> bool {
    point.0 >= rect.left()
        && point.0 <= rect.right()
        && point.1 >= rect.top()
        && point.1 <= rect.bottom()
}

fn find_text_tag_rect(
    image: &RgbaImage,
    roi_rect: Rect,
    width: u32,
    height: u32,
    anchor: TextAnchor,
    roi_obstacles: &[Rect],
    label_obstacles: &[Rect],
) -> Rect {
    let mut candidates = Vec::new();

    let left = roi_rect.left().max(4);
    let right = (roi_rect.right() - width as i32 + 1).max(4);
    let center = (roi_rect.left() + (roi_rect.width() as i32 - width as i32) / 2).max(4);

    match anchor {
        TextAnchor::Above => {
            candidates.push((left, roi_rect.top().saturating_sub(26)));
            candidates.push((center, roi_rect.top().saturating_sub(26)));
            candidates.push((right, roi_rect.top().saturating_sub(26)));
            candidates.push((left, roi_rect.bottom() + 6));
            candidates.push((right, roi_rect.bottom() + 6));
        }
        TextAnchor::Below => {
            candidates.push((left, roi_rect.bottom() + 6));
            candidates.push((center, roi_rect.bottom() + 6));
            candidates.push((right, roi_rect.bottom() + 6));
            candidates.push((left, roi_rect.top().saturating_sub(26)));
            candidates.push((right, roi_rect.top().saturating_sub(26)));
        }
    }

    candidates.push((4, roi_rect.bottom() + 28));
    candidates.push((
        image.width() as i32 - width as i32 - 4,
        roi_rect.top().saturating_sub(48),
    ));

    for (x, y) in candidates {
        let rect = clamp_rect_to_image(image, Rect::at(x, y).of_size(width, height));
        if !roi_obstacles
            .iter()
            .any(|taken| rects_overlap(*taken, rect))
            && !label_obstacles
                .iter()
                .any(|taken| rects_overlap(*taken, rect))
        {
            return rect;
        }
    }

    clamp_rect_to_image(
        image,
        Rect::at(left, roi_rect.bottom() + 6).of_size(width, height),
    )
}

fn clamp_rect_to_image(image: &RgbaImage, rect: Rect) -> Rect {
    let max_x = image.width().saturating_sub(rect.width() + 4) as i32;
    let max_y = image.height().saturating_sub(rect.height() + 4) as i32;
    let x = rect.left().clamp(4, max_x.max(4));
    let y = rect.top().clamp(4, max_y.max(4));
    Rect::at(x, y).of_size(rect.width(), rect.height())
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    let ax2 = a.left() + a.width() as i32;
    let ay2 = a.top() + a.height() as i32;
    let bx2 = b.left() + b.width() as i32;
    let by2 = b.top() + b.height() as i32;

    a.left() < bx2 && ax2 > b.left() && a.top() < by2 && ay2 > b.top()
}

fn expand_rect(rect: Rect, padding: i32) -> Rect {
    let width = (rect.width() as i32 + padding * 2).max(1) as u32;
    let height = (rect.height() as i32 + padding * 2).max(1) as u32;
    Rect::at(rect.left() - padding, rect.top() - padding).of_size(width, height)
}

fn collect_roi_obstacles(image: &RgbaImage, rois: RoiTemplateSet) -> Vec<Rect> {
    let mut obstacles = Vec::with_capacity(13);
    obstacles.push(expand_rect(
        roi_to_rect(image.width(), image.height(), rois.remaining_funds_roi),
        4,
    ));
    obstacles.extend(
        rois.price_rois
            .iter()
            .map(|roi| expand_rect(roi_to_rect(image.width(), image.height(), *roi), 4)),
    );
    obstacles.extend(
        rois.name_rois
            .iter()
            .map(|roi| expand_rect(roi_to_rect(image.width(), image.height(), *roi), 4)),
    );
    obstacles
}

fn rect_center(rect: Rect) -> (i32, i32) {
    (
        rect.left() + rect.width() as i32 / 2,
        rect.top() + rect.height() as i32 / 2,
    )
}

fn nearest_point_on_rect(rect: Rect, x: i32, y: i32) -> (i32, i32) {
    let left = rect.left();
    let right = rect.right();
    let top = rect.top();
    let bottom = rect.bottom();

    let clamped_x = x.clamp(left, right);
    let clamped_y = y.clamp(top, bottom);

    let distances = [
        ((left, clamped_y), (x - left).abs()),
        ((right, clamped_y), (x - right).abs()),
        ((clamped_x, top), (y - top).abs()),
        ((clamped_x, bottom), (y - bottom).abs()),
    ];

    distances
        .into_iter()
        .min_by_key(|(_, distance)| *distance)
        .map(|(point, _)| point)
        .unwrap_or((clamped_x, clamped_y))
}

fn short_price_text(text: &str) -> String {
    if text.trim().is_empty() {
        "-".to_string()
    } else {
        text.trim().chars().take(4).collect()
    }
}

fn short_name_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        "-".to_string()
    } else {
        let mut short: String = trimmed.chars().take(6).collect();
        if trimmed.chars().count() > 6 {
            short.push('…');
        }
        short
    }
}

fn overlay_font() -> Option<&'static FontArc> {
    static FONT: OnceLock<Option<FontArc>> = OnceLock::new();
    FONT.get_or_init(load_overlay_font).as_ref()
}

fn load_overlay_font() -> Option<FontArc> {
    for path in candidate_font_paths() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Ok(font) = FontArc::try_from_vec(bytes) {
            return Some(font);
        }
    }
    None
}

fn candidate_font_paths() -> [PathBuf; 4] {
    [
        PathBuf::from(r"C:\Windows\Fonts\simhei.ttf"),
        PathBuf::from(r"C:\Windows\Fonts\msyh.ttc"),
        PathBuf::from(r"C:\Windows\Fonts\simsun.ttc"),
        PathBuf::from(r"C:\Windows\Fonts\msyhbd.ttc"),
    ]
}

fn roi_to_rect(width: u32, height: u32, roi: RelativeRoi) -> Rect {
    let x = (roi.0 * width as f32).max(0.0) as i32;
    let y = (roi.1 * height as f32).max(0.0) as i32;
    let roi_width = (roi.2 * width as f32).max(1.0) as u32;
    let roi_height = (roi.3 * height as f32).max(1.0) as u32;
    Rect::at(x, y).of_size(roi_width, roi_height)
}

fn draw_digit(image: &mut RgbaImage, x: i32, y: i32, digit: usize, color: Rgba<u8>) {
    let segments = match digit {
        0 => [true, true, true, true, true, true, false],
        1 => [false, true, true, false, false, false, false],
        2 => [true, true, false, true, true, false, true],
        3 => [true, true, true, true, false, false, true],
        4 => [false, true, true, false, false, true, true],
        5 => [true, false, true, true, false, true, true],
        6 => [true, false, true, true, true, true, true],
        7 => [true, true, true, false, false, false, false],
        8 => [true, true, true, true, true, true, true],
        9 => [true, true, true, true, false, true, true],
        _ => [false, false, false, false, false, false, false],
    };

    let horizontal = [(x + 1, y), (x + 1, y + 5), (x + 1, y + 10)];
    let vertical = [(x + 8, y + 1), (x + 8, y + 6), (x, y + 6), (x, y + 1)];

    if segments[0] {
        draw_filled_rect_mut(
            image,
            Rect::at(horizontal[0].0, horizontal[0].1).of_size(6, 2),
            color,
        );
    }
    if segments[1] {
        draw_filled_rect_mut(
            image,
            Rect::at(vertical[0].0, vertical[0].1).of_size(2, 4),
            color,
        );
    }
    if segments[2] {
        draw_filled_rect_mut(
            image,
            Rect::at(vertical[1].0, vertical[1].1).of_size(2, 4),
            color,
        );
    }
    if segments[3] {
        draw_filled_rect_mut(
            image,
            Rect::at(horizontal[2].0, horizontal[2].1).of_size(6, 2),
            color,
        );
    }
    if segments[4] {
        draw_filled_rect_mut(
            image,
            Rect::at(vertical[2].0, vertical[2].1).of_size(2, 4),
            color,
        );
    }
    if segments[5] {
        draw_filled_rect_mut(
            image,
            Rect::at(vertical[3].0, vertical[3].1).of_size(2, 4),
            color,
        );
    }
    if segments[6] {
        draw_filled_rect_mut(
            image,
            Rect::at(horizontal[1].0, horizontal[1].1).of_size(6, 2),
            color,
        );
    }
}

pub fn preprocess_roi(image: &DynamicImage, is_number: bool) -> DynamicImage {
    let gray = image.to_luma8();
    let scale = if is_number { 3.0 } else { 2.0 };
    let resized = resize(
        &gray,
        (gray.width() as f32 * scale) as u32,
        (gray.height() as f32 * scale) as u32,
        FilterType::CatmullRom,
    );
    let mut processed = resized;

    if is_number {
        if border_mean_gray(&processed) < 100.0 {
            invert_gray_in_place(&mut processed);
        }
        processed = threshold(&processed, 150, ThresholdType::Binary);
    } else if mean_gray(&processed) < 100.0 {
        invert_gray_in_place(&mut processed);
    }

    let bordered = add_white_border(&processed, 10);
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

fn invert_gray_in_place(image: &mut GrayImage) {
    for pixel in image.pixels_mut() {
        pixel.0[0] = 255 - pixel.0[0];
    }
}

fn border_mean_gray(image: &GrayImage) -> f32 {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return 0.0;
    }

    let mut sum = 0u64;
    let mut count = 0u64;

    for x in 0..width {
        sum += image.get_pixel(x, 0).0[0] as u64;
        count += 1;
        if height > 1 {
            sum += image.get_pixel(x, height - 1).0[0] as u64;
            count += 1;
        }
    }

    if width > 1 && height > 2 {
        for y in 1..(height - 1) {
            sum += image.get_pixel(0, y).0[0] as u64;
            count += 1;
            sum += image.get_pixel(width - 1, y).0[0] as u64;
            count += 1;
        }
    }

    if count == 0 {
        0.0
    } else {
        sum as f32 / count as f32
    }
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
    let before = before.to_rgb8();
    let after = after.to_rgb8();

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
    let mut best_score = 0.0f32;
    let mut best_idx = None;

    for slot in 0..slots {
        let start = (slot as f32 * slot_width) as usize;
        let end = if slot == slots - 1 {
            width
        } else {
            ((slot as f32 + 1.0) * slot_width) as usize
        };

        let (before_mean, before_std) = mean_std_rgb_region(&before, start, end, height);
        let (after_mean, after_std) = mean_std_rgb_region(&after, start, end, height);

        let mean_diff = before_mean
            .iter()
            .zip(after_mean.iter())
            .map(|(lhs, rhs)| (rhs - lhs).abs())
            .sum::<f32>();
        let std_diff = before_std
            .iter()
            .zip(after_std.iter())
            .map(|(lhs, rhs)| (rhs - lhs).abs())
            .sum::<f32>();
        let score = mean_diff + std_diff;

        if score > best_score {
            best_score = score;
            best_idx = Some(slot);
        }
    }

    if best_score <= 5.0 {
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

fn mean_std_rgb_region(
    image: &image::RgbImage,
    start_x: usize,
    end_x: usize,
    height: usize,
) -> ([f32; 3], [f32; 3]) {
    let count = ((end_x.saturating_sub(start_x)) * height).max(1) as f32;
    let mut sum = [0.0f32; 3];

    for y in 0..height {
        for x in start_x..end_x {
            let pixel = image.get_pixel(x as u32, y as u32);
            for (channel, value) in pixel.0.iter().enumerate().take(3) {
                sum[channel] += *value as f32;
            }
        }
    }

    let mean = [sum[0] / count, sum[1] / count, sum[2] / count];
    let mut variance_sum = [0.0f32; 3];

    for y in 0..height {
        for x in start_x..end_x {
            let pixel = image.get_pixel(x as u32, y as u32);
            for (channel, value) in pixel.0.iter().enumerate().take(3) {
                let delta = *value as f32 - mean[channel];
                variance_sum[channel] += delta * delta;
            }
        }
    }

    let std = [
        (variance_sum[0] / count).sqrt(),
        (variance_sum[1] / count).sqrt(),
        (variance_sum[2] / count).sqrt(),
    ];

    (mean, std)
}
