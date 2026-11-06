use super::{CONTEXT_MENU_ROW_HEIGHT, LIST_MENU_OPTIONS, MonitorContextAction, VariableLayer};
use crate::graphics::monitors::{ListMonitorFileAction, VariableMonitor};

fn monitor_point(layer: &VariableLayer) -> (f32, f32) {
    let monitor = &layer.monitors[0];
    (
        monitor.x + 20.0 * monitor.scale,
        monitor.y - 15.0 * monitor.scale,
    )
}

fn menu_item_point(layer: &VariableLayer, index: usize) -> (f32, f32) {
    let menu = layer.context_menu.unwrap();
    (
        menu.left + 5.0 - super::STAGE_HALF_WIDTH,
        super::STAGE_HALF_HEIGHT - (menu.top + index as f32 * CONTEXT_MENU_ROW_HEIGHT + 8.0),
    )
}

#[test]
fn list_monitor_context_menu_has_only_import_export_and_hide() {
    let monitor = VariableMonitor::new("items", "2", 0.0, 0.0)
        .with_value_key("Stage::items#list-id@shared")
        .with_list_items(vec!["first".into(), "second".into()], 140.0, 120.0);
    let mut layer = VariableLayer::new(vec![monitor]).unwrap();
    let (x, y) = monitor_point(&layer);

    assert!(layer.open_context_menu(x, y));
    assert_eq!(layer.context_menu.unwrap().options, LIST_MENU_OPTIONS);
    assert_eq!(
        layer.context_menu.unwrap().options,
        &[
            MonitorContextAction::ImportList,
            MonitorContextAction::ExportList,
            MonitorContextAction::Hide,
        ]
    );

    let (x, y) = menu_item_point(&layer, 0);
    let (monitor_index, _, action) = layer.activate_context_menu(Some((x, y))).unwrap();
    assert_eq!(action, MonitorContextAction::ImportList);
    assert!(matches!(
        layer.list_file_action(monitor_index, action),
        Some(ListMonitorFileAction::Import { key }) if key == "Stage::items#list-id@shared"
    ));

    let (x, y) = monitor_point(&layer);
    layer.open_context_menu(x, y);
    let (x, y) = menu_item_point(&layer, 1);
    let (monitor_index, _, action) = layer.activate_context_menu(Some((x, y))).unwrap();
    assert_eq!(action, MonitorContextAction::ExportList);
    assert!(matches!(
        layer.list_file_action(monitor_index, action),
        Some(ListMonitorFileAction::Export { file_name, contents })
            if file_name == "items.txt" && contents == "first\r\nsecond"
    ));
}

#[test]
fn scalar_and_reporter_menus_do_not_offer_list_or_slider_only_actions() {
    let scalar = VariableMonitor::new("score", "2", 0.0, 0.0);
    let mut layer = VariableLayer::new(vec![scalar]).unwrap();
    let (x, y) = monitor_point(&layer);
    layer.open_context_menu(x, y);
    let options = layer.context_menu.unwrap().options;
    assert!(options.contains(&MonitorContextAction::Slider));
    assert!(!options.contains(&MonitorContextAction::ImportList));
    assert!(!options.contains(&MonitorContextAction::ExportList));

    let reporter = VariableMonitor::new("x position", "0", 0.0, 0.0)
        .with_kind(super::super::MonitorKind::Reporter);
    let mut layer = VariableLayer::new(vec![reporter]).unwrap();
    let (x, y) = monitor_point(&layer);
    layer.open_context_menu(x, y);
    let options = layer.context_menu.unwrap().options;
    assert!(!options.contains(&MonitorContextAction::Slider));
    assert!(!options.contains(&MonitorContextAction::ImportList));
    assert!(!options.contains(&MonitorContextAction::ExportList));
}
