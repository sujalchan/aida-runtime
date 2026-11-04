use super::{
    MonitorBounds, MonitorDisplayMode, MonitorKind, SCRATCH_HEIGHT, SCRATCH_WIDTH,
    STAGE_HALF_HEIGHT, STAGE_HALF_WIDTH, VariableLayer,
};

pub(super) const CONTEXT_MENU_WIDTH: f32 = 106.0;
pub(super) const CONTEXT_MENU_ROW_HEIGHT: f32 = 17.0;
pub(super) const SLIDER_RANGE_EDITOR_WIDTH: f32 = 156.0;
pub(super) const SLIDER_RANGE_EDITOR_HEIGHT: f32 = 72.0;

#[derive(Clone, Copy)]
pub(super) struct MonitorContextMenu {
    pub(super) monitor_index: usize,
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) hovered_item: Option<usize>,
    pub(super) options: &'static [MonitorContextAction],
}

pub(super) struct SliderRangeEditor {
    pub(super) monitor_index: usize,
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) min_text: String,
    pub(super) max_text: String,
    pub(super) editing_min: bool,
    replace_on_next_text: bool,
}

pub(crate) enum SliderRangeEditorInput<'a> {
    Text(&'a str),
    Backspace,
    SwitchField,
    Apply,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MonitorContextAction {
    NormalReadout,
    LargeReadout,
    Slider,
    ChangeSliderRange,
    ImportList,
    ExportList,
    Hide,
    Dismiss,
}

const VARIABLE_MENU_OPTIONS: &[MonitorContextAction] = &[
    MonitorContextAction::NormalReadout,
    MonitorContextAction::LargeReadout,
    MonitorContextAction::Slider,
    MonitorContextAction::Hide,
];
const SLIDER_VARIABLE_MENU_OPTIONS: &[MonitorContextAction] = &[
    MonitorContextAction::NormalReadout,
    MonitorContextAction::LargeReadout,
    MonitorContextAction::Slider,
    MonitorContextAction::ChangeSliderRange,
    MonitorContextAction::Hide,
];
const READOUT_MENU_OPTIONS: &[MonitorContextAction] = &[
    MonitorContextAction::NormalReadout,
    MonitorContextAction::LargeReadout,
    MonitorContextAction::Hide,
];
const LIST_MENU_OPTIONS: &[MonitorContextAction] = &[
    MonitorContextAction::ImportList,
    MonitorContextAction::ExportList,
    MonitorContextAction::Hide,
];

impl VariableLayer {
    pub(crate) fn list_file_action(
        &self,
        monitor_index: usize,
        action: MonitorContextAction,
    ) -> Option<super::ListMonitorFileAction> {
        let monitor = self.monitors.get(monitor_index)?;
        if monitor.kind != MonitorKind::List {
            return None;
        }
        match action {
            MonitorContextAction::ImportList => Some(super::ListMonitorFileAction::Import {
                key: monitor.value_key.clone(),
            }),
            MonitorContextAction::ExportList => {
                let mut contents = String::new();
                for (index, value) in monitor.list_items.iter().enumerate() {
                    if index > 0 {
                        contents.push_str("\r\n");
                    }
                    contents.push_str(&crate::runtime::scratch_string(value.clone()));
                }
                Some(super::ListMonitorFileAction::Export {
                    file_name: format!("{}.txt", monitor.name),
                    contents,
                })
            }
            _ => None,
        }
    }

    pub(crate) fn open_context_menu(&mut self, x: f32, y: f32) -> bool {
        let Some(monitor_index) = self.hit_test(x, y) else {
            return self.close_context_menu();
        };
        let monitor = &self.monitors[monitor_index];
        let options = match (monitor.kind, monitor.display_mode) {
            (MonitorKind::List, _) => LIST_MENU_OPTIONS,
            (MonitorKind::ScalarVariable, MonitorDisplayMode::Slider) => {
                SLIDER_VARIABLE_MENU_OPTIONS
            }
            (MonitorKind::ScalarVariable, _) => VARIABLE_MENU_OPTIONS,
            _ => READOUT_MENU_OPTIONS,
        };
        let menu_height = CONTEXT_MENU_ROW_HEIGHT * options.len() as f32;
        let left = (x + STAGE_HALF_WIDTH).clamp(0.0, SCRATCH_WIDTH as f32 - CONTEXT_MENU_WIDTH);
        let top = (STAGE_HALF_HEIGHT - y).clamp(0.0, SCRATCH_HEIGHT as f32 - menu_height);
        self.context_menu = Some(MonitorContextMenu {
            monitor_index,
            left,
            top,
            hovered_item: None,
            options,
        });
        true
    }

    pub(crate) fn close_context_menu(&mut self) -> bool {
        let was_open = self.context_menu.is_some() || self.slider_range_editor.is_some();
        self.slider_range_editor = None;
        self.context_menu.take().is_some() || was_open
    }

    pub(crate) fn context_menu_is_open(&self) -> bool {
        self.context_menu.is_some() || self.slider_range_editor.is_some()
    }

    pub(crate) fn update_context_menu_hover(&mut self, pointer: Option<(f32, f32)>) -> bool {
        if self.slider_range_editor.is_some() {
            return false;
        }
        let Some(menu) = &mut self.context_menu else {
            return false;
        };
        let hovered_item = context_menu_item(menu, pointer);
        if menu.hovered_item == hovered_item {
            return false;
        }
        menu.hovered_item = hovered_item;
        true
    }

    pub(crate) fn activate_context_menu(
        &mut self,
        pointer: Option<(f32, f32)>,
    ) -> Option<(usize, MonitorBounds, MonitorContextAction)> {
        if let Some(editor) = self.slider_range_editor.as_mut() {
            let (x, y) = pointer?;
            let x = x + STAGE_HALF_WIDTH;
            let y = STAGE_HALF_HEIGHT - y;
            let relative_y = y - editor.top;
            if x < editor.left
                || x > editor.left + SLIDER_RANGE_EDITOR_WIDTH
                || !(18.0..=46.0).contains(&relative_y)
            {
                return None;
            }
            editor.editing_min = relative_y < 32.0;
            editor.replace_on_next_text = true;
            let index = editor.monitor_index;
            return Some((
                index,
                self.monitor_bounds(index),
                MonitorContextAction::Dismiss,
            ));
        }

        let menu = self.context_menu.take()?;
        let old_bounds = self.monitor_bounds(menu.monitor_index);
        let action = match context_menu_item(&menu, pointer).map(|index| menu.options[index]) {
            Some(MonitorContextAction::NormalReadout) => {
                self.set_monitor_display_mode(menu.monitor_index, MonitorDisplayMode::Expanded);
                MonitorContextAction::NormalReadout
            }
            Some(MonitorContextAction::LargeReadout) => {
                self.set_monitor_display_mode(menu.monitor_index, MonitorDisplayMode::ValueOnly);
                MonitorContextAction::LargeReadout
            }
            Some(MonitorContextAction::Slider) => {
                self.set_monitor_display_mode(menu.monitor_index, MonitorDisplayMode::Slider);
                MonitorContextAction::Slider
            }
            Some(MonitorContextAction::ChangeSliderRange) => {
                let monitor = &self.monitors[menu.monitor_index];
                self.slider_range_editor = Some(SliderRangeEditor {
                    monitor_index: menu.monitor_index,
                    left: menu
                        .left
                        .clamp(0.0, SCRATCH_WIDTH as f32 - SLIDER_RANGE_EDITOR_WIDTH),
                    top: menu
                        .top
                        .clamp(0.0, SCRATCH_HEIGHT as f32 - SLIDER_RANGE_EDITOR_HEIGHT),
                    min_text: format_range_number(monitor.slider_min),
                    max_text: format_range_number(monitor.slider_max),
                    editing_min: true,
                    replace_on_next_text: true,
                });
                MonitorContextAction::ChangeSliderRange
            }
            Some(MonitorContextAction::ImportList) => MonitorContextAction::ImportList,
            Some(MonitorContextAction::ExportList) => MonitorContextAction::ExportList,
            Some(MonitorContextAction::Hide) => {
                let monitor = &mut self.monitors[menu.monitor_index];
                monitor.visibility = super::MonitorVisibility::Hidden;
                monitor.shadow_tile = None;
                self.gpu_translation_enabled = self
                    .monitors
                    .iter()
                    .filter(|monitor| monitor.visibility == super::MonitorVisibility::Visible)
                    .take(2)
                    .count()
                    == 1;
                MonitorContextAction::Hide
            }
            Some(MonitorContextAction::Dismiss) | None => MonitorContextAction::Dismiss,
        };
        self.slider_hovered = None;
        self.slider_thumb_hovered = None;
        Some((menu.monitor_index, old_bounds, action))
    }

    pub(crate) fn edit_slider_range(&mut self, input: SliderRangeEditorInput<'_>) -> bool {
        let Some(editor) = self.slider_range_editor.as_mut() else {
            return false;
        };
        match input {
            SliderRangeEditorInput::Text(text) => {
                let field = if editor.editing_min {
                    &mut editor.min_text
                } else {
                    &mut editor.max_text
                };
                if !text.is_empty() && editor.replace_on_next_text {
                    field.clear();
                    editor.replace_on_next_text = false;
                }
                for character in text.chars() {
                    if character.is_ascii_digit()
                        || (character == '.' && !field.contains('.'))
                        || (character == '-' && field.is_empty())
                    {
                        field.push(character);
                    }
                }
            }
            SliderRangeEditorInput::Backspace => {
                let field = if editor.editing_min {
                    &mut editor.min_text
                } else {
                    &mut editor.max_text
                };
                field.pop();
                editor.replace_on_next_text = false;
            }
            SliderRangeEditorInput::SwitchField => {
                editor.editing_min = !editor.editing_min;
                editor.replace_on_next_text = true;
            }
            SliderRangeEditorInput::Cancel => {
                self.slider_range_editor = None;
                return true;
            }
            SliderRangeEditorInput::Apply => {
                let min = editor.min_text.parse::<f64>().ok();
                let max = editor.max_text.parse::<f64>().ok();
                if let (Some(min), Some(max)) = (min, max)
                    && min.is_finite()
                    && max.is_finite()
                    && min < max
                {
                    let index = editor.monitor_index;
                    self.monitors[index].slider_min = min;
                    self.monitors[index].slider_max = max;
                    self.slider_range_editor = None;
                    self.mark_gpu_shapes(index);
                    return true;
                }
            }
        }
        true
    }
}

pub(super) fn format_slider_value(value: f64, min: f64, max: f64) -> String {
    let decimals = decimal_places(min).max(decimal_places(max));
    let scale = 10_f64.powi(decimals as i32);
    let rounded = (value * scale).round() / scale;
    if decimals == 0 {
        return format!("{rounded:.0}");
    }
    let mut text = format!("{rounded:.decimals$}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".to_owned() } else { text }
}

fn format_range_number(value: f64) -> String {
    format_slider_value(value, value, value)
}

fn decimal_places(value: f64) -> usize {
    let mut text = format!("{value:.9}");
    while text.ends_with('0') {
        text.pop();
    }
    text.split_once('.')
        .map_or(0, |(_, fraction)| fraction.len())
}

#[cfg(test)]
mod tests {
    use super::{CONTEXT_MENU_ROW_HEIGHT, LIST_MENU_OPTIONS, MonitorContextAction, VariableLayer};
    use crate::graphics::variables::{ListMonitorFileAction, VariableMonitor};

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
}

fn context_menu_item(menu: &MonitorContextMenu, pointer: Option<(f32, f32)>) -> Option<usize> {
    let (x, y) = pointer?;
    let x = x + STAGE_HALF_WIDTH;
    let y = STAGE_HALF_HEIGHT - y;
    (x >= menu.left
        && x < menu.left + CONTEXT_MENU_WIDTH
        && y >= menu.top
        && y < menu.top + CONTEXT_MENU_ROW_HEIGHT * menu.options.len() as f32)
        .then(|| ((y - menu.top) / CONTEXT_MENU_ROW_HEIGHT).floor() as usize)
        .filter(|index| *index < menu.options.len())
}
