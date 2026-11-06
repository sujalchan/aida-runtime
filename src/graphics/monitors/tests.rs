use super::{
    LIST_ROW_FONT_SIZE, LIST_ROW_HEIGHT, ListEditorInput, MonitorDisplayMode, VariableLayer,
    VariableMonitor,
};
use crate::runtime::{RuntimeInputEvent, ScratchValue, variable_snapshot_key};
use std::sync::Arc;

fn list_layer() -> VariableLayer {
    VariableLayer::new(vec![
        VariableMonitor::new("items", "2", 0.0, 0.0)
            .with_value_key("Stage::items#items-id@shared")
            .with_list_items(vec!["alpha".into(), "beta".into()], 180.0, 150.0),
    ])
    .unwrap()
}

fn click_local(
    layer: &mut VariableLayer,
    local_x: f32,
    local_y: f32,
) -> super::ListPointerResponse {
    let monitor = &layer.monitors[0];
    let x = monitor.x + local_x * monitor.scale;
    let y = monitor.y - local_y * monitor.scale;
    layer.handle_list_pointer(x, y)
}

fn list_with_items(length: usize, height: f32) -> VariableLayer {
    VariableLayer::new(vec![
        VariableMonitor::new("items", "", 0.0, 0.0)
            .with_value_key("Stage::items#items-id@shared")
            .with_list_items(
                (0..length).map(|index| index.to_string()).collect(),
                180.0,
                height,
            ),
    ])
    .unwrap()
}

fn stage_point_for_list(layer: &VariableLayer, local_x: f32, local_y: f32) -> (f32, f32) {
    let monitor = &layer.monitors[0];
    (
        monitor.x + local_x * monitor.scale,
        monitor.y - local_y * monitor.scale,
    )
}

#[test]
fn list_plus_appends_one_blank_item_and_opens_it_selected() {
    let mut layer = list_layer();
    let response = click_local(&mut layer, 12.0, 140.0);

    assert!(response.consumed);
    assert!(layer.list_editor_is_open());
    assert_eq!(layer.list_display_len(0), 3);
    assert_eq!(layer.visible_list_rows(0).last(), Some(&(3, String::new())));
    assert!(matches!(
        response.events.as_slice(),
        [RuntimeInputEvent::ListAppend { key }] if key == "Stage::items#items-id@shared"
    ));
    let editor = layer.list_editor.as_ref().unwrap();
    assert_eq!(editor.index, 3);
    assert!(editor.select_all);
    assert!(editor.appended);

    layer.edit_list_item(ListEditorInput::Text("new"));
    let (_, events) = layer.edit_list_item(ListEditorInput::Commit);
    assert!(matches!(
        events.as_slice(),
        [RuntimeInputEvent::ListSetItem { index: 3, value, .. }] if value == "new"
    ));
}

#[test]
fn list_value_editor_selects_all_and_click_positions_caret() {
    let mut layer = list_layer();
    click_local(&mut layer, 35.0, super::LIST_CONTENT_TOP + 11.0);
    assert!(layer.list_editor.as_ref().unwrap().select_all);

    layer.edit_list_item(ListEditorInput::Text("word"));
    let prefix_width = super::text_width(&layer.regular_font, "w", LIST_ROW_FONT_SIZE);
    click_local(
        &mut layer,
        super::LIST_ROW_VALUE_X + super::LIST_ROW_TEXT_INSET + prefix_width,
        super::LIST_CONTENT_TOP + 11.0,
    );
    assert_eq!(layer.list_editor.as_ref().unwrap().caret, 1);
    assert!(!layer.list_editor.as_ref().unwrap().select_all);

    layer.edit_list_item(ListEditorInput::Text("X"));
    let (_, events) = layer.edit_list_item(ListEditorInput::Commit);
    assert!(matches!(
        events.as_slice(),
        [RuntimeInputEvent::ListSetItem { index: 1, value, .. }] if value == "wXord"
    ));
}

#[test]
fn list_active_row_x_deletes_only_that_item() {
    let mut layer = list_layer();
    click_local(&mut layer, 35.0, super::LIST_CONTENT_TOP + 11.0);
    let response = click_local(&mut layer, 165.0, super::LIST_CONTENT_TOP + 11.0);

    assert!(response.consumed);
    assert!(!layer.list_editor_is_open());
    assert!(matches!(
        response.events.as_slice(),
        [RuntimeInputEvent::ListDeleteItem { key, index: 1 }]
            if key == "Stage::items#items-id@shared"
    ));
}

#[test]
fn list_resize_changes_only_available_area_not_typography_or_row_spacing() {
    let mut layer = list_layer();
    let width = layer.monitors[0].width;
    let height = layer.monitors[0].height;
    let scale = layer.monitors[0].scale;
    let row_height = LIST_ROW_HEIGHT;
    let font_size = LIST_ROW_FONT_SIZE;
    let response = click_local(&mut layer, width - 4.0, height - 4.0);
    assert!(response.resizing);

    let monitor = &layer.monitors[0];
    let start_x = monitor.x + (width - 4.0) * scale;
    let start_y = monitor.y - (height - 4.0) * scale;
    assert!(layer.drag_to(start_x + 20.0 * scale, start_y - 18.0 * scale));
    assert!((layer.monitors[0].width - (width + 20.0)).abs() < 0.01);
    assert!((layer.monitors[0].height - (height + 18.0)).abs() < 0.01);
    assert_eq!(LIST_ROW_HEIGHT, row_height);
    assert_eq!(LIST_ROW_FONT_SIZE, font_size);
    assert_eq!(super::LIST_ROW_TEXT_INSET, 6.0);
    assert_eq!(super::LIST_ROW_INDEX_X, 7.0);
}

#[test]
fn list_can_resize_to_stage_bounds_and_move_by_its_title_area() {
    let mut layer = list_layer();
    layer.monitors[0].x = -240.0;
    layer.monitors[0].y = 180.0;
    let scale = layer.monitors[0].scale;
    let width = layer.monitors[0].width;
    let height = layer.monitors[0].height;
    let response = click_local(&mut layer, width - 4.0, height - 4.0);
    assert!(response.resizing);
    let target_x = layer.monitors[0].x + 480.0 - 4.0 * scale;
    let target_y = layer.monitors[0].y - 360.0 + 4.0 * scale;
    assert!(layer.drag_to(target_x, target_y));
    assert!((layer.monitors[0].width * scale - 480.0).abs() < 0.01);
    assert!((layer.monitors[0].height * scale - 360.0).abs() < 0.01);

    let mut movable = list_layer();
    let monitor = &movable.monitors[0];
    let title_x = monitor.x + 20.0 * monitor.scale;
    let title_y = monitor.y - 12.0 * monitor.scale;
    let title_response = movable.handle_list_pointer(title_x, title_y);
    assert!(!title_response.consumed);
    assert!(movable.begin_drag(title_x, title_y));
    assert!(movable.drag_to(title_x + 30.0, title_y - 20.0));
}

#[test]
fn variable_monitor_builds_gpu_instances() {
    let mut layer =
        VariableLayer::new(vec![VariableMonitor::new("my variable", "0", 8.0, 8.0)]).unwrap();
    let (shapes, glyphs) = layer.build_gpu_monitor_instances(0, true, true).unwrap();
    assert!(!shapes.is_empty());
    assert!(!glyphs.is_empty());
}

#[test]
fn list_monitor_uses_list_layout_and_keeps_its_items() {
    let scalar = VariableMonitor::new("score", "3", 8.0, 8.0);
    let monitor = VariableMonitor::new("items", "3", 8.0, 8.0).with_list_items(
        vec!["red".to_owned(), "green".to_owned(), "blue".to_owned()],
        110.0,
        200.0,
    );
    let mut layer = VariableLayer::new(vec![scalar, monitor]).unwrap();
    assert_eq!(layer.monitors[0].scale, super::SCALAR_MONITOR_SCALE);
    assert_eq!(layer.monitors[1].scale, super::LIST_MONITOR_SCALE);
    assert_eq!(layer.monitors[1].display_mode, MonitorDisplayMode::List);
    assert_eq!(layer.monitors[1].category, super::MonitorCategory::Lists);
    assert_eq!(layer.monitors[1].kind, super::MonitorKind::List);
    let (shapes, glyphs) = layer.build_gpu_monitor_instances(1, true, true).unwrap();
    assert_eq!(shapes.len(), 8);
    assert!(glyphs.len() > 10);
    let bounds = layer.monitor_bounds(1);
    assert!(
        layer
            .hit_test((bounds.left + bounds.right) * 0.5 - 240.0, 0.0)
            .is_some()
    );
    assert!(
        layer
            .toggle_compact_at((bounds.left + bounds.right) * 0.5 - 240.0, 0.0)
            .is_none()
    );
}

#[test]
fn list_monitors_draw_above_scalar_monitors() {
    let list = VariableMonitor::new("items", "1", 0.0, 0.0).with_list_items(
        vec!["one".to_owned()],
        110.0,
        100.0,
    );
    let scalar = VariableMonitor::new("score", "1", 0.0, 0.0);
    let layer = VariableLayer::new(vec![list, scalar]).unwrap();

    assert_eq!(
        layer.gpu_monitor_ids_in_draw_order().collect::<Vec<_>>(),
        vec![1, 0]
    );
}

#[test]
fn only_scalar_variable_monitors_cycle_into_slider_mode() {
    let mut scalar =
        VariableLayer::new(vec![VariableMonitor::new("score", "1", 0.0, 0.0)]).unwrap();
    assert!(scalar.toggle_compact_at(1.0, -1.0).is_some());
    assert_eq!(
        scalar.monitors[0].display_mode,
        MonitorDisplayMode::ValueOnly
    );
    assert!(scalar.toggle_compact_at(1.0, -1.0).is_some());
    assert_eq!(scalar.monitors[0].display_mode, MonitorDisplayMode::Slider);

    let reporter = VariableMonitor::new("x position", "0", 0.0, 0.0)
        .with_kind(super::MonitorKind::Reporter)
        .with_category(super::MonitorCategory::Motion);
    let mut reporter = VariableLayer::new(vec![reporter]).unwrap();
    assert!(reporter.toggle_compact_at(1.0, -1.0).is_some());
    assert_eq!(
        reporter.monitors[0].display_mode,
        MonitorDisplayMode::ValueOnly
    );
    assert!(reporter.toggle_compact_at(1.0, -1.0).is_some());
    assert_eq!(
        reporter.monitors[0].display_mode,
        MonitorDisplayMode::Expanded
    );

    let list =
        VariableMonitor::new("items", "0", 0.0, 0.0).with_list_items(Vec::new(), 110.0, 120.0);
    let mut list = VariableLayer::new(vec![list]).unwrap();
    assert!(list.toggle_compact_at(1.0, -1.0).is_none());
    assert_eq!(list.monitors[0].display_mode, MonitorDisplayMode::List);
}

#[test]
fn list_snapshot_refreshes_rows_without_converting_list_to_scalar_readout() {
    let key = "Stage::items#list-id@shared";
    let monitor = VariableMonitor::new("items", "1", 0.0, 0.0)
        .with_value_key(key)
        .with_list_items(vec!["one".to_owned()], 110.0, 120.0);
    let mut layer = VariableLayer::new(vec![monitor]).unwrap();
    let items = Arc::new(vec![
        ScratchValue::Text("one".into()),
        ScratchValue::Text("two".into()),
    ]);
    assert_eq!(
        layer.set_monitor_list(key, Arc::clone(&items)),
        Some((0, true))
    );
    assert!(Arc::ptr_eq(&layer.monitors[0].list_items, &items));
    assert_eq!(layer.monitors[0].display_mode, MonitorDisplayMode::List);
    assert_eq!(layer.take_dirty_gpu_shapes(), vec![0]);
}

#[test]
fn large_list_scroll_keeps_absolute_indices_and_builds_only_visible_gpu_rows() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../aida-ast/tests/src/monitor-tests-sensing.sb3"
    );
    let json = aida_ast::extractor::read_project_json(path).unwrap();
    let project = aida_ast::parser::parse_project(&json).unwrap();
    let source = project
        .targets
        .iter()
        .flat_map(|target| target.lists.values())
        .find(|list| list.items.len() == 200_000)
        .expect("fixture should contain its complete list");
    let values = source
        .items
        .iter()
        .map(|item| match item {
            serde_json::Value::String(value) => ScratchValue::Text(value.clone()),
            serde_json::Value::Number(value) => ScratchValue::Number(value.as_f64().unwrap()),
            serde_json::Value::Bool(value) => ScratchValue::Boolean(*value),
            _ => ScratchValue::Text(item.to_string()),
        })
        .collect::<Vec<_>>();
    let monitor = VariableMonitor::new("large list", "", 0.0, 0.0).with_list_values(
        Arc::new(values),
        220.0,
        196.0,
    );
    let mut layer = VariableLayer::new(vec![monitor]).unwrap();

    let first_rows = layer.visible_list_rows(0);
    assert_eq!(
        first_rows.first(),
        Some(&(1, source.items[0].as_i64().unwrap().to_string()))
    );
    assert_eq!(first_rows.len(), 7);
    let top_scrollbar = layer.list_scrollbar_geometry(0).unwrap();
    assert_eq!(
        top_scrollbar.thumb_height,
        super::LIST_SCROLLBAR_MIN_THUMB_HEIGHT
    );
    assert_eq!(top_scrollbar.thumb_y, top_scrollbar.track_y);
    let (_, first_glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();
    assert!(first_glyphs.len() < 256);

    assert_eq!(layer.scroll_list_at(10.0, -30.0, 250_000.0), Some(0));
    let last_rows = layer.visible_list_rows(0);
    assert_eq!(last_rows.first().unwrap().0, 199_995);
    assert_eq!(
        last_rows.last(),
        Some(&(200_000, source.items[199_999].as_i64().unwrap().to_string()))
    );
    let (_, last_glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();
    assert!(last_glyphs.len() < 256);
    assert_eq!(layer.monitors[0].list_scroll_offset, 199_994);
    let bottom_scrollbar = layer.list_scrollbar_geometry(0).unwrap();
    assert!(
        (bottom_scrollbar.thumb_y + bottom_scrollbar.thumb_height
            - (bottom_scrollbar.track_y + bottom_scrollbar.track_height))
            .abs()
            < 0.01
    );

    // drag the scrollbar across the full fixture list without traversing its item data.
    assert!(layer.set_list_scroll_offset(0, 0));
    let top_scrollbar = layer.list_scrollbar_geometry(0).unwrap();
    let pointer = stage_point_for_list(
        &layer,
        top_scrollbar.track_x + 1.0,
        top_scrollbar.thumb_y + top_scrollbar.thumb_height * 0.5,
    );
    assert!(
        layer
            .handle_list_pointer(pointer.0, pointer.1)
            .scrollbar_dragging
    );
    let bottom_pointer = stage_point_for_list(
        &layer,
        top_scrollbar.track_x + 1.0,
        top_scrollbar.track_y + top_scrollbar.track_height - top_scrollbar.thumb_height * 0.5,
    );
    assert!(layer.drag_to(bottom_pointer.0, bottom_pointer.1));
    assert_eq!(layer.visible_list_rows(0).last().unwrap().0, 200_000);
    let dragged_scrollbar = layer.list_scrollbar_geometry(0).unwrap();
    assert!(
        (dragged_scrollbar.thumb_y + dragged_scrollbar.thumb_height
            - (dragged_scrollbar.track_y + dragged_scrollbar.track_height))
            .abs()
            < 0.01
    );
}

#[test]
fn list_wheel_scroll_accumulates_fractional_rows_and_clamps_to_available_range() {
    let monitor = VariableMonitor::new("items", "", 0.0, 0.0).with_list_items(
        (0..20).map(|index| index.to_string()).collect(),
        160.0,
        196.0,
    );
    let mut layer = VariableLayer::new(vec![monitor]).unwrap();
    assert_eq!(layer.scroll_list_at(10.0, -30.0, 0.5), None);
    assert_eq!(layer.scroll_list_at(10.0, -30.0, 0.5), Some(0));
    assert_eq!(layer.monitors[0].list_scroll_offset, 1);
    assert_eq!(layer.scroll_list_at(10.0, -30.0, 10_000.0), Some(0));
    assert_eq!(layer.monitors[0].list_scroll_offset, 14);
    assert_eq!(layer.scroll_list_at(10.0, -30.0, 3.0), None);
}

#[test]
fn list_scrollbar_only_appears_on_hover_when_content_exceeds_the_viewport() {
    let empty = list_with_items(0, 196.0);
    let fitting = list_with_items(6, 196.0);
    assert_eq!(empty.list_scrollbar_geometry(0), None);
    assert_eq!(fitting.list_scrollbar_geometry(0), None);

    let mut scrollable = list_with_items(7, 196.0);
    let geometry = scrollable.list_scrollbar_geometry(0).unwrap();
    let (hidden_shapes, _) = scrollable
        .build_gpu_monitor_instances(0, true, false)
        .unwrap();
    let track_index = hidden_shapes.len() - 3;
    let thumb_index = hidden_shapes.len() - 2;
    assert_eq!(hidden_shapes[track_index].fill[3], 0.0);
    assert_eq!(hidden_shapes[track_index].fill, [0.0; 4]);
    assert!(track_index < thumb_index);
    assert!(
        scrollable.update_list_scrollbar_hover(Some(stage_point_for_list(
            &scrollable,
            geometry.track_x + 1.0,
            geometry.track_y + 1.0,
        )))
    );
    assert!(scrollable.list_scrollbar_is_visible(0));
    let (visible_shapes, _) = scrollable
        .build_gpu_monitor_instances(0, true, false)
        .unwrap();
    assert_eq!(visible_shapes.len(), hidden_shapes.len());
    assert_eq!(visible_shapes[track_index].fill[3], 42.0 / 255.0);
    assert_eq!(visible_shapes[thumb_index].fill[3], 190.0 / 255.0);
    assert!(visible_shapes[track_index].fill[0] < visible_shapes[thumb_index].fill[0]);
    for index in 0..visible_shapes.len() {
        if index != track_index && index != thumb_index {
            assert_eq!(visible_shapes[index], hidden_shapes[index]);
        }
    }
    assert!(scrollable.update_list_scrollbar_hover(None));
    assert!(!scrollable.list_scrollbar_is_visible(0));
}

#[test]
fn partial_last_list_row_keeps_glyph_spacing_and_clips_at_the_footer() {
    let mut layer = list_with_items(20, 196.0);
    let (_, glyphs) = layer.build_gpu_monitor_instances(0, true, true).unwrap();
    let visible_rows = layer.visible_list_rows(0);
    let last_row = visible_rows.last().unwrap();
    assert_eq!(last_row, &(7, "6".to_owned()));

    let footer_top = layer.monitors[0].height - super::LIST_FOOTER_HEIGHT;
    let partial_row_y = super::LIST_CONTENT_TOP + 6.0 * super::LIST_ROW_HEIGHT;
    let last_row_glyphs = glyphs
        .iter()
        .filter(|glyph| glyph.clip_y[0] == partial_row_y)
        .collect::<Vec<_>>();
    assert!(last_row_glyphs.len() >= 2);
    assert!(
        last_row_glyphs
            .iter()
            .all(|glyph| glyph.clip_y[1] == footer_top)
    );
    assert!(
        last_row_glyphs
            .iter()
            .any(|glyph| glyph.local_position[0] < super::LIST_ROW_VALUE_X)
    );
    assert!(
        last_row_glyphs
            .iter()
            .any(|glyph| glyph.local_position[0] >= super::LIST_ROW_VALUE_X)
    );
    let index_glyph_id = layer.glyph_atlas.get('7', true).unwrap().id;
    let value_glyph_id = layer.glyph_atlas.get('6', false).unwrap().id;
    assert!(
        last_row_glyphs
            .iter()
            .any(|glyph| glyph.glyph_id == index_glyph_id)
    );
    assert!(
        last_row_glyphs
            .iter()
            .any(|glyph| glyph.glyph_id == value_glyph_id)
    );
    let previous_index_glyph_id = layer.glyph_atlas.get('6', true).unwrap().id;
    let previous_value_glyph_id = layer.glyph_atlas.get('5', false).unwrap().id;
    let previous_row_y = partial_row_y - super::LIST_ROW_HEIGHT;
    let previous_row_glyphs = glyphs
        .iter()
        .filter(|glyph| glyph.clip_y[0] == previous_row_y)
        .collect::<Vec<_>>();
    let last_index_y = last_row_glyphs
        .iter()
        .find(|glyph| glyph.glyph_id == index_glyph_id)
        .unwrap()
        .local_position[1];
    let previous_index_y = previous_row_glyphs
        .iter()
        .find(|glyph| glyph.glyph_id == previous_index_glyph_id)
        .unwrap()
        .local_position[1];
    let last_value_y = last_row_glyphs
        .iter()
        .find(|glyph| glyph.glyph_id == value_glyph_id)
        .unwrap()
        .local_position[1];
    let previous_value_y = previous_row_glyphs
        .iter()
        .find(|glyph| glyph.glyph_id == previous_value_glyph_id)
        .unwrap()
        .local_position[1];
    assert_eq!(last_index_y - previous_index_y, super::LIST_ROW_HEIGHT);
    assert_eq!(last_value_y - previous_value_y, super::LIST_ROW_HEIGHT);
}

#[test]
fn list_scrollbar_thumb_scales_and_maps_top_middle_and_bottom_offsets() {
    let proportional = list_with_items(20, 196.0)
        .list_scrollbar_geometry(0)
        .unwrap();
    let expected_thumb_height =
        proportional.track_height * proportional.track_height / (20.0 * super::LIST_ROW_HEIGHT);
    assert!((proportional.thumb_height - expected_thumb_height).abs() < 0.01);

    let mut layer = list_with_items(1_000, 196.0);
    let top = layer.list_scrollbar_geometry(0).unwrap();
    assert_eq!(top.thumb_y, top.track_y);
    assert_eq!(top.thumb_height, super::LIST_SCROLLBAR_MIN_THUMB_HEIGHT);

    let max_offset = layer.monitors[0].max_list_scroll_offset(1_000);
    assert!(layer.set_list_scroll_offset(0, max_offset / 2));
    let middle = layer.list_scrollbar_geometry(0).unwrap();
    let travel = middle.track_height - middle.thumb_height;
    assert!((middle.thumb_y - (middle.track_y + travel * 0.5)).abs() < 1.0);

    assert!(layer.set_list_scroll_offset(0, max_offset));
    let bottom = layer.list_scrollbar_geometry(0).unwrap();
    assert!(
        (bottom.thumb_y + bottom.thumb_height - (bottom.track_y + bottom.track_height)).abs()
            < 0.01
    );
}

#[test]
fn list_scrollbar_drag_and_track_click_scroll_visible_rows() {
    let mut layer = list_with_items(100, 196.0);
    let geometry = layer.list_scrollbar_geometry(0).unwrap();
    let pointer = stage_point_for_list(
        &layer,
        geometry.track_x + 2.0,
        geometry.thumb_y + geometry.thumb_height * 0.5,
    );
    let response = layer.handle_list_pointer(pointer.0, pointer.1);
    assert!(response.consumed);
    assert!(response.scrollbar_dragging);

    let bottom_pointer_y = geometry.track_y + geometry.track_height - geometry.thumb_height * 0.5;
    let bottom_pointer = stage_point_for_list(&layer, geometry.track_x + 2.0, bottom_pointer_y);
    assert!(layer.drag_to(bottom_pointer.0, bottom_pointer.1));
    assert_eq!(layer.monitors[0].list_scroll_offset, 100 - 6);
    assert_eq!(layer.visible_list_rows(0).first().unwrap().0, 95);
    layer.end_drag();

    assert!(layer.set_list_scroll_offset(0, 0));
    let top = layer.list_scrollbar_geometry(0).unwrap();
    let track_click = stage_point_for_list(
        &layer,
        top.track_x + 2.0,
        top.thumb_y + top.thumb_height + 2.0,
    );
    let response = layer.handle_list_pointer(track_click.0, track_click.1);
    assert!(response.consumed);
    assert!(!response.scrollbar_dragging);
    assert!(layer.monitors[0].list_scroll_offset > 0);
}

#[test]
fn list_scrollbar_recalculates_after_list_growth_and_viewport_resize() {
    let mut layer = list_with_items(20, 196.0);
    layer.take_dirty_gpu_shapes();
    let initial = layer.list_scrollbar_geometry(0).unwrap();
    let longer = Arc::new(
        (0..40)
            .map(|index| ScratchValue::Text(index.to_string()))
            .collect(),
    );
    assert_eq!(
        layer.set_monitor_list("Stage::items#items-id@shared", longer),
        Some((0, true))
    );
    assert_eq!(layer.take_dirty_gpu_shapes(), vec![0]);
    let after_growth = layer.list_scrollbar_geometry(0).unwrap();
    assert!(after_growth.thumb_height < initial.thumb_height);

    let monitor = &layer.monitors[0];
    let scale = monitor.scale;
    let start_x = monitor.x + (monitor.width - 4.0) * scale;
    let start_y = monitor.y - (monitor.height - 4.0) * scale;
    assert!(click_local(&mut layer, 176.0, 192.0).resizing);
    assert!(layer.drag_to(start_x, start_y - 44.0 * scale));
    let after_resize = layer.list_scrollbar_geometry(0).unwrap();
    assert!(after_resize.track_height > after_growth.track_height);
    assert!(after_resize.thumb_height > after_growth.thumb_height);
    assert_eq!(layer.take_dirty_gpu_shapes(), vec![0]);
}

#[test]
fn list_changes_refresh_visible_text_without_rebuilding_row_shapes() {
    let initial = Arc::new(
        (0..20)
            .map(|index| ScratchValue::Number(index as f64))
            .collect::<Vec<_>>(),
    );
    let monitor = VariableMonitor::new("items", "", 0.0, 0.0)
        .with_value_key("Stage::items#list-id@shared")
        .with_list_values(Arc::clone(&initial), 160.0, 196.0);
    let mut layer = VariableLayer::new(vec![monitor]).unwrap();
    layer.take_dirty_gpu_shapes();
    layer.take_dirty_gpu_texts();
    let (_, original_glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();

    let mut updated = (*initial).clone();
    updated[19] = ScratchValue::Text("updated tail".into());
    assert_eq!(
        layer.set_monitor_list("Stage::items#list-id@shared", Arc::new(updated)),
        Some((0, true))
    );
    assert!(layer.take_dirty_gpu_shapes().is_empty());
    assert_eq!(layer.take_dirty_gpu_texts(), vec![0]);
    assert_eq!(
        layer.visible_list_rows(0).last(),
        Some(&(7, "6".to_owned()))
    );
    let (_, unchanged_glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();
    assert_eq!(original_glyphs, unchanged_glyphs);

    layer.scroll_list_at(10.0, -30.0, 100.0);
    assert_eq!(
        layer.visible_list_rows(0).last(),
        Some(&(20, "updated tail".to_owned()))
    );
    let (_, updated_glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();
    assert_ne!(original_glyphs, updated_glyphs);
}

#[test]
fn gpu_text_keeps_literal_special_characters() {
    let mut layer =
        VariableLayer::new(vec![VariableMonitor::new("a < b", "1 & 2", 0.0, 0.0)]).unwrap();
    let (_, glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();
    assert_eq!(glyphs.len(), 6);
}

#[test]
fn timer_value_update_only_dirties_its_glyph_instances() {
    let mut layer = VariableLayer::new(vec![
        VariableMonitor::new("timer", "12.3456789", 0.0, 0.0),
        VariableMonitor::new("other", "static", 0.0, 80.0),
    ])
    .unwrap();
    layer.take_dirty_gpu_shapes();
    layer.take_dirty_gpu_texts();
    layer.take_dirty_transforms();
    let revision = layer.glyph_atlas_revision();

    layer
        .set_monitor_value("timer", "12.3456788".to_owned())
        .unwrap();
    assert!(layer.take_dirty_gpu_shapes().is_empty());
    assert_eq!(layer.take_dirty_gpu_texts(), vec![0]);
    assert_eq!(layer.glyph_atlas_revision(), revision);

    layer.take_dirty_gpu_texts();
    assert_eq!(
        layer.set_monitor_value("timer", "12.3456788".to_owned()),
        None
    );
    assert!(layer.take_dirty_gpu_shapes().is_empty());
    assert!(layer.take_dirty_gpu_texts().is_empty());
}

#[test]
fn stage_variable_snapshot_updates_its_visible_monitor() {
    let id = "`jEk@4|i[#Fk?(8x)AV.-my variable";
    let key = variable_snapshot_key("Stage", "my variable", id, None);
    let mut layer = VariableLayer::new(vec![
        VariableMonitor::new("my variable", "0", 5.0, 5.0).with_value_key(key.clone()),
    ])
    .unwrap();
    layer.take_dirty_gpu_texts();

    assert_eq!(
        layer.set_monitor_value(&key, "341".to_owned()),
        Some((0, true))
    );
    assert_eq!(layer.monitors[0].value, "341");
    assert_eq!(layer.take_dirty_gpu_texts(), vec![0]);
}

#[test]
fn slider_drag_coalesces_to_the_latest_numeric_value_for_its_variable_key() {
    let id = "slider-id";
    let key = variable_snapshot_key("Stage", "limit", id, None);
    let monitor = VariableMonitor::new("limit", "10", 0.0, 0.0)
        .with_display_mode(MonitorDisplayMode::Slider)
        .with_value_key(key.clone())
        .with_slider_range(0.0, 100.0);
    let mut layer = VariableLayer::new(vec![monitor]).unwrap();
    layer.take_dirty_gpu_texts();

    let monitor = &layer.monitors[0];
    let y = monitor.y - 42.0 * monitor.scale;
    let high_x = monitor.x + (14.0 + 0.8 * (monitor.width - 28.0)) * monitor.scale;
    assert!(layer.begin_slider_drag(high_x, y));

    let monitor = &layer.monitors[0];
    let low_x = monitor.x + (14.0 + 0.02 * (monitor.width - 28.0)) * monitor.scale;
    assert!(layer.drag_to(low_x, y));

    let changes = layer.take_changed_manual_values();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].0, key);
    assert_eq!(changes[0].1, 2.0);
    assert_eq!(layer.take_dirty_gpu_texts(), vec![0]);
}

#[test]
fn monitor_drag_updates_only_its_transform() {
    let mut layer = VariableLayer::new(vec![VariableMonitor::new("timer", "0", 0.0, 0.0)]).unwrap();
    let _ = layer.take_dirty_gpu_shapes();
    let _ = layer.take_dirty_gpu_texts();
    let _ = layer.take_dirty_transforms();
    let before = layer.gpu_monitor_transform(0);
    assert!(layer.begin_drag(1.0, -1.0));
    assert!(layer.drag_to(20.0, -1.0));
    assert!(layer.take_dirty_gpu_shapes().is_empty());
    assert!(layer.take_dirty_gpu_texts().is_empty());
    assert_eq!(layer.take_dirty_transforms(), vec![0]);
    assert_ne!(layer.gpu_monitor_transform(0), before);
}

#[test]
fn bring_to_front_changes_draw_order_without_changing_gpu_ids() {
    let mut layer = VariableLayer::new(vec![
        VariableMonitor::new("front later", "1", 0.0, 0.0),
        VariableMonitor::new("back", "2", 120.0, 100.0),
    ])
    .unwrap();
    let _ = layer.take_dirty_gpu_shapes();
    let _ = layer.take_dirty_gpu_texts();
    let _ = layer.take_dirty_transforms();
    assert!(layer.begin_drag(1.0, -1.0));
    assert_eq!(
        layer.gpu_monitor_ids_in_draw_order().collect::<Vec<_>>(),
        vec![1, 0]
    );
    assert!(layer.take_dirty_gpu_shapes().is_empty());
    assert!(layer.take_dirty_gpu_texts().is_empty());
    assert!(layer.take_dirty_transforms().is_empty());
}

#[test]
fn render_scale_changes_only_text_and_reuses_scale_tier() {
    let mut layer =
        VariableLayer::new(vec![VariableMonitor::new("timer", "12.345", 0.0, 0.0)]).unwrap();
    assert!(layer.set_render_scale(1.0));
    let _ = layer.take_dirty_gpu_shapes();
    let _ = layer.take_dirty_gpu_texts();
    let _ = layer.take_dirty_transforms();
    let low_scale_batch = layer.build_gpu_monitor_instances(0, true, true).unwrap();
    let low_scale_metadata = layer.glyph_metadata();
    let low_scale_bounds = low_scale_metadata[low_scale_batch.1[0].glyph_id as usize].bounds;
    let first_revision = layer.glyph_atlas_revision();

    assert!(layer.set_render_scale(5.0));
    assert_eq!(layer.take_dirty_gpu_texts(), vec![0]);
    assert!(layer.take_dirty_gpu_shapes().is_empty());
    assert!(layer.take_dirty_transforms().is_empty());
    let high_scale_batch = layer.build_gpu_monitor_instances(0, false, true).unwrap();
    let high_scale_metadata = layer.glyph_metadata();
    let high_scale_bounds = high_scale_metadata[high_scale_batch.1[0].glyph_id as usize].bounds;
    let high_scale_revision = layer.glyph_atlas_revision();
    assert!(high_scale_revision > first_revision);
    for index in 0..4 {
        assert!(
            (low_scale_bounds[index] - high_scale_bounds[index]).abs() < 0.2,
            "bounds differ at {index}: {} vs {}",
            low_scale_bounds[index],
            high_scale_bounds[index]
        );
    }
    assert_eq!(
        low_scale_batch
            .1
            .iter()
            .map(|glyph| glyph.local_position)
            .collect::<Vec<_>>(),
        high_scale_batch
            .1
            .iter()
            .map(|glyph| glyph.local_position)
            .collect::<Vec<_>>(),
    );

    assert!(!layer.set_render_scale(5.4));
    assert!(layer.take_dirty_gpu_texts().is_empty());
    assert_eq!(layer.glyph_atlas_revision(), high_scale_revision);
}
