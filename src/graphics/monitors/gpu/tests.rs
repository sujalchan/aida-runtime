use super::{
    LIST_ROW_RADIUS, MONITOR_FRAME_RADIUS, MONITOR_SHAPE_BOTTOM_ROUNDED, MONITOR_SHAPE_TOP_ROUNDED,
    MONITOR_VALUE_RADIUS,
};
use crate::graphics::bubbles::{
    BUBBLE_BODY_CORNER_RADIUS, BUBBLE_BORDER, BUBBLE_BORDER_WIDTH, BUBBLE_FONT_SIZE,
    BUBBLE_SPEECH_TAIL_EXTENT, BUBBLE_TAIL_CORNER_CLEARANCE, BubbleRenderInput,
};
use crate::graphics::color::linear_rgba;
use crate::graphics::monitors::{
    LIST_CONTENT_TOP, LIST_EMPTY_TOP_PADDING, LIST_FOOTER_SIDE_PADDING, LIST_HEADER_FONT_SIZE,
    LIST_HEADER_HORIZONTAL_PADDING, LIST_LENGTH_FONT_SIZE, LIST_ROW_FONT_SIZE, LIST_ROW_VALUE_X,
    MonitorKind, VariableLayer, VariableMonitor,
};
use crate::graphics::text::scaled_px_bounds;
use crate::graphics::{SCRATCH_HEIGHT, SCRATCH_WIDTH};
use crate::runtime::{BubbleKind, BubbleSnapshot};

#[test]
fn list_monitor_frame_matches_scalar_border_and_keeps_rounded_value_rows() {
    let scalar = VariableMonitor::new("score", "5", 0.0, 0.0);
    let list = VariableMonitor::new("items", "", 0.0, 0.0)
        .with_kind(MonitorKind::List)
        .with_list_items(vec!["one".to_owned()], 120.0, 100.0);
    let mut layer = VariableLayer::new(vec![scalar, list]).unwrap();
    let scalar_frame = layer
        .build_gpu_monitor_instances_for_id(0, true, false)
        .unwrap()
        .0[0];
    let scalar_value = layer
        .build_gpu_monitor_instances_for_id(0, true, false)
        .unwrap()
        .0[1];
    let list_frame = layer
        .build_gpu_monitor_instances_for_id(1, true, false)
        .unwrap()
        .0[0];

    assert_eq!(list_frame.params[0], scalar_frame.params[0]);
    assert_eq!(list_frame.params[1], scalar_frame.params[1]);
    assert_eq!(list_frame.fill, scalar_frame.fill);
    assert_eq!(scalar_frame.fill, linear_rgba([229, 240, 255, 255]));
    assert_eq!(list_frame.border, scalar_frame.border);
    assert_eq!(scalar_value.params[1], 1.0);
    assert_eq!(scalar_value.border, linear_rgba([0, 0, 0, 38]));
    assert_eq!(scalar_value.params[0], MONITOR_VALUE_RADIUS);
    let list_shapes = layer
        .build_gpu_monitor_instances_for_id(1, true, false)
        .unwrap()
        .0;
    assert_eq!(list_shapes[1].params[0], MONITOR_FRAME_RADIUS - 1.0);
    assert_eq!(list_shapes[1].shape[2], MONITOR_SHAPE_TOP_ROUNDED);
    assert_eq!(
        list_shapes.last().unwrap().params[0],
        MONITOR_FRAME_RADIUS - 1.0
    );
    assert_eq!(
        list_shapes.last().unwrap().shape[2],
        MONITOR_SHAPE_BOTTOM_ROUNDED
    );
    assert_eq!(list_shapes[2].params[0], LIST_ROW_RADIUS);
    assert_eq!(list_shapes[2].rect[0], LIST_ROW_VALUE_X);
    assert_eq!(list_shapes[2].rect[2], 120.0 - LIST_ROW_VALUE_X - 6.0);

    let header = super::fit_monitor_text(
        &layer.bold_font,
        "a very long list name that must be clipped",
        64.0 - LIST_HEADER_HORIZONTAL_PADDING * 2.0,
        LIST_HEADER_FONT_SIZE,
    );
    let plus_width =
        crate::graphics::monitors::text_width(&layer.bold_font, "+", LIST_LENGTH_FONT_SIZE);
    let equals_width =
        crate::graphics::monitors::text_width(&layer.bold_font, "=", LIST_LENGTH_FONT_SIZE);
    let footer_text_width = 64.0 - LIST_FOOTER_SIDE_PADDING * 2.0 - plus_width - equals_width;
    let length = super::fit_monitor_text(
        &layer.bold_font,
        "length 123456789",
        footer_text_width,
        LIST_LENGTH_FONT_SIZE,
    );
    assert!(
        crate::graphics::monitors::text_width(&layer.bold_font, &header, LIST_HEADER_FONT_SIZE)
            <= 40.0
    );
    assert!(
        crate::graphics::monitors::text_width(&layer.bold_font, &length, LIST_LENGTH_FONT_SIZE)
            <= footer_text_width
    );
    let title_ink =
        super::glyph_text_x_bounds(&layer.bold_font, "items", LIST_HEADER_FONT_SIZE).unwrap();
    let title_start =
        super::centered_glyph_start(&layer.bold_font, "items", LIST_HEADER_FONT_SIZE, 120.0);
    assert!(((title_start + (title_ink.0 + title_ink.1) * 0.5) - 60.0).abs() < 0.01);
    let (_, list_glyphs) = layer
        .build_gpu_monitor_instances_for_id(1, false, true)
        .unwrap();
    assert!((list_glyphs[0].scale - 14.0 / 18.0).abs() < f32::EPSILON);
    assert!((list_glyphs[5].scale - LIST_ROW_FONT_SIZE / 18.0).abs() < f32::EPSILON);
    assert!((list_glyphs[6].scale - LIST_ROW_FONT_SIZE / 19.0).abs() < f32::EPSILON);
    assert!((list_glyphs.last().unwrap().scale - 2.0 / 3.0).abs() < f32::EPSILON);

    let empty_list = VariableMonitor::new("items", "", 0.0, 0.0)
        .with_kind(MonitorKind::List)
        .with_list_items(Vec::new(), 120.0, 100.0);
    let mut empty_layer = VariableLayer::new(vec![empty_list]).unwrap();
    let (_, empty_glyphs) = empty_layer
        .build_gpu_monitor_instances_for_id(0, false, true)
        .unwrap();
    let empty_text_start = super::centered_glyph_start(
        &empty_layer.regular_font,
        "(empty)",
        LIST_ROW_FONT_SIZE,
        120.0,
    );
    let empty_text_center = LIST_CONTENT_TOP + LIST_EMPTY_TOP_PADDING + LIST_ROW_FONT_SIZE * 0.5;
    let empty_text_baseline = crate::graphics::monitors::canvas::centered_baseline(
        &empty_layer.regular_font,
        LIST_ROW_FONT_SIZE,
        empty_text_center,
    );
    assert!((empty_glyphs[5].local_position[0] - empty_text_start).abs() < f32::EPSILON);
    assert!((empty_glyphs[5].local_position[1] - empty_text_baseline).abs() < f32::EPSILON);
}

#[test]
fn full_stage_list_monitor_builds_unpadded_gpu_instances_for_all_visible_rows() {
    let long_item = "1234567890".repeat(10);
    let items = vec![long_item; 200];
    let monitor = VariableMonitor::new("large list", "", 0.0, 0.0)
        .with_kind(MonitorKind::List)
        .with_list_items(
            items,
            SCRATCH_WIDTH as f32 / 0.65,
            SCRATCH_HEIGHT as f32 / 0.65,
        );
    let mut layer = VariableLayer::new(vec![monitor]).unwrap();
    let (shapes, glyphs) = layer
        .build_gpu_monitor_instances_for_id(0, true, true)
        .unwrap();

    assert!(shapes.len() > 16);
    assert!(glyphs.len() > 256);
    assert!(glyphs.len() < 4096);
}

#[test]
fn scaled_pixel_bounds_keep_top_left_glyph_orientation() {
    let bounds = scaled_px_bounds(
        ab_glyph::Rect {
            min: ab_glyph::point(1.0, -27.0),
            max: ab_glyph::point(21.0, 3.0),
        },
        4,
    );
    assert_eq!(bounds, [0.25, -6.75, 5.0, 7.5]);
}

#[test]
fn monitor_color_bytes_are_converted_from_srgb_before_surface_output() {
    let color = linear_rgba([128, 64, 255, 128]);
    assert!((color[0] - 0.21586).abs() < 0.0001);
    assert!((color[1] - 0.05127).abs() < 0.0001);
    assert_eq!(color[2], 1.0);
    assert!((color[3] - 128.0 / 255.0).abs() < f32::EPSILON);
}

#[test]
fn bubble_layout_wraps_clamps_and_keeps_each_bubble_instanced() {
    let mut layer = VariableLayer::new(Vec::new()).unwrap();
    let long_text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.";
    let batch = layer
        .build_gpu_bubble_instances(&[
            BubbleRenderInput {
                bubble: BubbleSnapshot {
                    kind: BubbleKind::Say,
                    text: long_text.to_owned(),
                },
                x: -100.0,
                y: 170.0,
                sprite_width: 80.0,
                sprite_height: 100.0,
            },
            BubbleRenderInput {
                bubble: BubbleSnapshot {
                    kind: BubbleKind::Think,
                    text: "Hmm...".to_owned(),
                },
                x: 40.0,
                y: 0.0,
                sprite_width: 80.0,
                sprite_height: 100.0,
            },
        ])
        .unwrap();

    assert_eq!(batch.transforms.len(), 2);
    assert!(batch.transforms[0][1] >= 8.0);
    let say_body = batch.shapes[0];
    assert_eq!(say_body.params[0], 17.0);
    assert!((say_body.params[1] - BUBBLE_BORDER_WIDTH).abs() < f32::EPSILON);
    assert_eq!(say_body.border, linear_rgba(BUBBLE_BORDER));
    assert_eq!(say_body.shape[2], 1.0);
    assert_eq!(
        say_body.rect[3],
        say_body.shape[1] + BUBBLE_SPEECH_TAIL_EXTENT
    );
    let say_min_anchor = BUBBLE_BODY_CORNER_RADIUS + BUBBLE_TAIL_CORNER_CLEARANCE;
    let expected_say_anchor = (SCRATCH_WIDTH as f32 * 0.5 - 100.0 - batch.transforms[0][0])
        .clamp(say_min_anchor, say_body.shape[0] - say_min_anchor);
    assert!((say_body.shape[3] - expected_say_anchor).abs() < 0.01);
    assert_eq!(batch.shapes.len(), 5);
    assert!(batch.shapes[0].rect[3] > 100.0);
    assert!(say_body.shape[1] <= 344.0);
    assert!(
        batch.shapes[1..4]
            .iter()
            .all(|circle| circle.rect[0].abs() < 100.0 && circle.rect[1].abs() < 400.0)
    );
    assert_eq!(batch.shapes[4].monitor_id, 1);
    assert_eq!(batch.shapes[4].rect[0..2], [0.0, 0.0]);
    let thought_body = batch.shapes[4].rect;
    let largest_circle = batch.shapes[1].rect;
    let thought_min_anchor = BUBBLE_BODY_CORNER_RADIUS + 4.6;
    let expected_thought_anchor = (SCRATCH_WIDTH as f32 * 0.5 + 40.0 - batch.transforms[1][0])
        .clamp(thought_min_anchor, thought_body[2] - thought_min_anchor);
    assert!((largest_circle[0] + largest_circle[2] * 0.5 - expected_thought_anchor).abs() < 0.01);
    assert!((largest_circle[1] + largest_circle[3] * 0.5 - thought_body[3]).abs() < 0.01);
    let middle_circle = batch.shapes[2].rect;
    let smallest_circle = batch.shapes[3].rect;
    assert!(middle_circle[0] < largest_circle[0]);
    assert!(smallest_circle[0] < middle_circle[0]);
    assert!(middle_circle[1] > largest_circle[1]);
    assert!(smallest_circle[1] > middle_circle[1]);
    let large_center_x = largest_circle[0] + largest_circle[2] * 0.5;
    let middle_center_x = middle_circle[0] + middle_circle[2] * 0.5;
    let small_center_x = smallest_circle[0] + smallest_circle[2] * 0.5;
    let circle_distance = |a: [f32; 4], b: [f32; 4]| {
        let dx = (a[0] + a[2] * 0.5) - (b[0] + b[2] * 0.5);
        let dy = (a[1] + a[3] * 0.5) - (b[1] + b[3] * 0.5);
        (dx * dx + dy * dy).sqrt()
    };
    let large_middle_gap = circle_distance(largest_circle, middle_circle)
        - (largest_circle[2] + middle_circle[2]) * 0.5;
    let middle_small_gap = circle_distance(middle_circle, smallest_circle)
        - (middle_circle[2] + smallest_circle[2]) * 0.5;
    assert!(large_middle_gap >= 0.0);
    assert!(middle_small_gap >= 0.0);
    assert!((large_middle_gap - middle_small_gap).abs() < 1.0);
    assert!(middle_center_x - small_center_x < large_center_x - middle_center_x);
    assert!(batch.glyphs.iter().any(|glyph| glyph.monitor_id == 0));
    assert!(batch.glyphs.iter().any(|glyph| glyph.monitor_id == 1));
    assert!((batch.glyphs[0].scale - BUBBLE_FONT_SIZE / 19.0).abs() < f32::EPSILON);
}

#[test]
fn speech_tail_mirrors_toward_the_owning_sprite() {
    let mut layer = VariableLayer::new(Vec::new()).unwrap();
    let batch = layer
        .build_gpu_bubble_instances(&[
            BubbleRenderInput {
                bubble: BubbleSnapshot {
                    kind: BubbleKind::Say,
                    text: "left".to_owned(),
                },
                x: -100.0,
                y: 0.0,
                sprite_width: 80.0,
                sprite_height: 100.0,
            },
            BubbleRenderInput {
                bubble: BubbleSnapshot {
                    kind: BubbleKind::Say,
                    text: "right".to_owned(),
                },
                x: 160.0,
                y: 0.0,
                sprite_width: 80.0,
                sprite_height: 100.0,
            },
        ])
        .unwrap();

    assert_eq!(batch.shapes.len(), 2);
    assert_eq!(batch.shapes[0].shape[2], 1.0);
    assert_eq!(batch.shapes[1].shape[2], 2.0);
    for index in 0..2 {
        assert!((batch.transforms[index][1] + batch.shapes[index].shape[1] - 138.0).abs() < 0.01);
    }
}

#[cfg(test)]
mod monitor_color_tests {
    use super::super::monitor_color;
    use crate::graphics::monitors::MonitorCategory;

    #[test]
    fn default_monitor_category_colors_match_scratch_palette() {
        let categories = [
            (MonitorCategory::Motion, [76, 151, 255, 255]),
            (MonitorCategory::Looks, [153, 102, 255, 255]),
            (MonitorCategory::Sound, [207, 99, 207, 255]),
            (MonitorCategory::Events, [255, 191, 0, 255]),
            (MonitorCategory::Control, [255, 171, 25, 255]),
            (MonitorCategory::Sensing, [92, 177, 214, 255]),
            (MonitorCategory::Operators, [89, 192, 89, 255]),
            (MonitorCategory::Variables, [255, 140, 26, 255]),
            (MonitorCategory::Lists, [255, 102, 26, 255]),
            (MonitorCategory::MyBlocks, [255, 102, 128, 255]),
            (
                MonitorCategory::Extension([15, 189, 140]),
                [15, 189, 140, 255],
            ),
        ];
        for (category, expected) in categories {
            assert_eq!(monitor_color(category), expected);
        }
    }
}
