use super::super::*;
use crate::graphics::monitors::{
    MonitorCategory, MonitorDisplayMode, MonitorKind, VariableMonitor,
};
use crate::runtime::{list_snapshot_key, variable_snapshot_key};

pub(in crate::graphics::state) fn project_monitors(
    project: &aida_ast::parser::ScratchProject,
) -> Vec<VariableMonitor> {
    project
        .monitors
        .iter()
        .map(|monitor| {
            let id = monitor.id.as_str();
            let target = project.targets.iter().find(|target| {
                let is_reporter =
                    monitor.opcode != "data_variable" && monitor.opcode != "data_listcontents";
                let owns_monitor =
                    target.variables.contains_key(id) || target.lists.contains_key(id);
                monitor
                    .sprite_name
                    .as_deref()
                    .map_or(target.is_stage, |sprite| target.name == sprite)
                    && (owns_monitor || is_reporter)
            });
            let variable = target.and_then(|target| target.variables.get(id));
            let list = target.and_then(|target| target.lists.get(id));
            let fallback_name = monitor
                .params
                .get("VARIABLE")
                .or_else(|| monitor.params.get("LIST"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("reporter");
            let is_list = monitor.opcode == "data_listcontents" || list.is_some();
            let kind = if is_list {
                MonitorKind::List
            } else if monitor.opcode == "data_variable" || variable.is_some() {
                MonitorKind::ScalarVariable
            } else {
                MonitorKind::Reporter
            };
            let base_name = variable
                .map(|variable| variable.name.as_str())
                .or_else(|| list.map(|list| list.name.as_str()))
                .unwrap_or(fallback_name);
            let monitor_name = if kind == MonitorKind::ScalarVariable || kind == MonitorKind::List {
                base_name.to_owned()
            } else {
                monitor_label(&monitor.opcode, &monitor.params)
            };
            let name = target.map_or_else(
                || monitor_name.clone(),
                |target| format!("{}: {monitor_name}", target.name),
            );
            let value_key = if monitor.opcode == "sensing_timer" {
                "timer".to_owned()
            } else if let Some(target) = target {
                let target_index = project
                    .targets
                    .iter()
                    .position(|item| std::ptr::eq(item, target))
                    .unwrap_or(0);
                if let Some(variable) = variable {
                    variable_snapshot_key(
                        &target.name,
                        &variable.name,
                        id,
                        (!target.is_stage).then_some(target_index),
                    )
                } else if let Some(list) = list {
                    list_snapshot_key(
                        &target.name,
                        &list.name,
                        id,
                        (!target.is_stage).then_some(target_index),
                    )
                } else if target.is_stage && monitor.opcode == "looks_backdropnumbername" {
                    match monitor
                        .params
                        .get("NUMBER_NAME")
                        .and_then(serde_json::Value::as_str)
                    {
                        Some("number") => "__stage/backdrop/number".to_owned(),
                        _ => "__stage/backdrop/name".to_owned(),
                    }
                } else {
                    sprite_reporter_key(&target.name, &monitor.opcode).unwrap_or_else(|| {
                        format!(
                            "__reporter/{}/{}/{}",
                            target.name, monitor.opcode, monitor.id
                        )
                    })
                }
            } else {
                match monitor.opcode.as_str() {
                    "sensing_timer" => "timer".to_owned(),
                    "sensing_mousex" => "__input/mouse_x".to_owned(),
                    "sensing_mousey" => "__input/mouse_y".to_owned(),
                    "looks_backdropnumbername" => match monitor
                        .params
                        .get("NUMBER_NAME")
                        .and_then(serde_json::Value::as_str)
                    {
                        Some("number") => "__stage/backdrop/number".to_owned(),
                        _ => "__stage/backdrop/name".to_owned(),
                    },
                    _ => format!("__reporter/{}/{}", monitor.opcode, monitor.id),
                }
            };
            let value = monitor
                .value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| monitor.value.to_string());
            let mode = match monitor.mode.as_str() {
                "large" => MonitorDisplayMode::ValueOnly,
                "slider" => MonitorDisplayMode::Slider,
                "list" => MonitorDisplayMode::List,
                _ => MonitorDisplayMode::Expanded,
            };
            let category = monitor_category(&monitor.opcode, kind);
            let mut result = VariableMonitor::new(
                name,
                value,
                monitor.x as f32 - SCRATCH_WIDTH as f32 * 0.5,
                SCRATCH_HEIGHT as f32 * 0.5 - monitor.y as f32,
            )
            .with_display_mode(mode)
            .with_category(category)
            .with_kind(kind)
            .with_value_key(value_key)
            .with_visibility(monitor.visible)
            .with_slider_range(monitor.slider_min, monitor.slider_max);
            if list.is_some() {
                result = result.with_list_values(
                    Arc::new(Vec::new()),
                    if monitor.width > 0.0 {
                        monitor.width as f32
                    } else {
                        110.0
                    },
                    if monitor.height > 0.0 {
                        monitor.height as f32
                    } else {
                        200.0
                    },
                );
            }
            result
        })
        .collect()
}

fn monitor_category(opcode: &str, kind: MonitorKind) -> MonitorCategory {
    if kind == MonitorKind::List {
        return MonitorCategory::Lists;
    }
    if kind == MonitorKind::ScalarVariable {
        return MonitorCategory::Variables;
    }
    match opcode.split('_').next().unwrap_or("") {
        "motion" => MonitorCategory::Motion,
        "looks" => MonitorCategory::Looks,
        "sound" => MonitorCategory::Sound,
        "event" => MonitorCategory::Events,
        "control" => MonitorCategory::Control,
        "sensing" => MonitorCategory::Sensing,
        "operator" => MonitorCategory::Operators,
        "procedures" => MonitorCategory::MyBlocks,
        "pen" => MonitorCategory::Extension([15, 189, 140]),
        _ => MonitorCategory::Extension([15, 189, 140]),
    }
}

fn monitor_label(
    opcode: &str,
    params: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    if opcode == "looks_backdropnumbername" || opcode == "looks_costumenumbername" {
        let subject = if opcode == "looks_backdropnumbername" {
            "backdrop"
        } else {
            "costume"
        };
        let detail = match params
            .get("NUMBER_NAME")
            .and_then(serde_json::Value::as_str)
        {
            Some("number") => "number",
            Some("name") => "name",
            _ => return subject.to_owned(),
        };
        return format!("{subject} {detail}");
    }
    if opcode == "sensing_current" {
        return match params
            .get("CURRENTMENU")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "year" => "year".to_owned(),
            "month" => "month".to_owned(),
            "date" => "date".to_owned(),
            "dayofweek" => "day of week".to_owned(),
            "hour" => "hour".to_owned(),
            "minute" => "minute".to_owned(),
            "second" => "second".to_owned(),
            _ => "current".to_owned(),
        };
    }
    let label = match opcode {
        "motion_xposition" => "x position",
        "motion_yposition" => "y position",
        "motion_direction" => "direction",
        "looks_size" => "size",
        "sound_volume" => "Volume",
        "sensing_answer" => "answer",
        "sensing_loudness" => "loudness",
        "sensing_timer" => "timer",
        "sensing_mousex" => "mouse x",
        "sensing_mousey" => "mouse y",
        "sensing_mousedown" => "mouse down?",
        "sensing_keypressed" => "key pressed?",
        "sensing_username" => "username",
        "sensing_dayssince2000" => "days since 2000",
        _ => {
            return opcode
                .split_once('_')
                .map_or_else(|| opcode.to_owned(), |(_, rest)| rest.replace('_', " "));
        }
    };
    label.to_owned()
}

fn sprite_reporter_key(target_name: &str, opcode: &str) -> Option<String> {
    let property = match opcode {
        "motion_xposition" => "x",
        "motion_yposition" => "y",
        "motion_direction" => "direction",
        "looks_size" => "size",
        _ => return None,
    };
    Some(format!("__sprite/{target_name}/{property}"))
}

#[cfg(test)]
#[path = "monitors/tests.rs"]
mod monitor_project_tests;
