use super::{monitor_category, monitor_label, project_monitors};
use crate::graphics::monitors::{MonitorCategory, MonitorDisplayMode, MonitorKind};

#[test]
fn project_monitors_keep_owner_scope_category_and_list_mode() {
    let project: aida_ast::parser::ScratchProject = serde_json::from_str(
            r#"{
                "targets": [
                    {"name":"Stage","isStage":true,"variables":{"global-id":["score",7]},"lists":{"stage-list":["stage items",["a","b"]]}},
                    {"name":"Sprite1","isStage":false,"variables":{"local-id":["score",5]}},
                    {"name":"Sprite2","isStage":false,"variables":{"local-id-2":["score",9]}}
                ],
                "monitors": [
                    {"id":"local-id","mode":"default","opcode":"data_variable","params":{"VARIABLE":"score"},"spriteName":"Sprite1","value":5,"x":10,"y":20,"visible":true},
                    {"id":"local-id-2","mode":"default","opcode":"data_variable","params":{"VARIABLE":"score"},"spriteName":"Sprite2","value":9,"x":10,"y":60,"visible":true},
                    {"id":"global-id","mode":"default","opcode":"data_variable","params":{"VARIABLE":"score"},"spriteName":null,"value":7,"x":10,"y":100,"visible":true},
                    {"id":"stage-list","mode":"list","opcode":"data_listcontents","params":{"LIST":"stage items"},"spriteName":null,"value":2,"width":120,"height":150,"visible":true},
                    {"id":"plus","mode":"large","opcode":"operator_add","params":{},"spriteName":null,"value":3,"visible":true}
                ]
            }"#,
        )
        .unwrap();
    let monitors = project_monitors(&project);
    assert_eq!(monitors[0].name, "Sprite1: score");
    assert_eq!(monitors[1].name, "Sprite2: score");
    assert_eq!(monitors[2].name, "score");
    assert_ne!(monitors[0].value_key, monitors[1].value_key);
    assert_eq!(monitors[3].kind, MonitorKind::List);
    assert_eq!(monitors[3].category, MonitorCategory::Lists);
    assert_eq!(monitors[3].display_mode, MonitorDisplayMode::List);
    assert_eq!(monitors[3].width, 120.0);
    assert_eq!(monitors[3].height, 150.0);
    assert_eq!(monitors[4].kind, MonitorKind::Reporter);
    assert_eq!(monitors[4].category, MonitorCategory::Operators);
    assert_eq!(monitors[4].display_mode, MonitorDisplayMode::ValueOnly);
}

#[test]
fn motion_reporter_monitors_keep_sprite_target_and_label() {
    let project: aida_ast::parser::ScratchProject = serde_json::from_str(
            r#"{"targets":[
                {"name":"Stage","isStage":true},
                {"name":"Sprite1","isStage":false},
                {"name":"Sprite2","isStage":false}],
              "monitors":[
                {"id":"x1","mode":"default","opcode":"motion_xposition","params":{},"spriteName":"Sprite1","value":12,"visible":true},
                {"id":"y1","mode":"default","opcode":"motion_yposition","params":{},"spriteName":"Sprite1","value":-4,"visible":true},
                {"id":"d1","mode":"default","opcode":"motion_direction","params":{},"spriteName":"Sprite1","value":45,"visible":true},
                {"id":"x2","mode":"default","opcode":"motion_xposition","params":{},"spriteName":"Sprite2","value":99,"visible":true}] }"#,
        )
        .unwrap();
    let monitors = project_monitors(&project);
    assert_eq!(monitors[0].name, "Sprite1: x position");
    assert_eq!(monitors[1].name, "Sprite1: y position");
    assert_eq!(monitors[2].name, "Sprite1: direction");
    assert_eq!(monitors[3].name, "Sprite2: x position");
    assert_eq!(monitors[0].value_key, "__sprite/Sprite1/x");
    assert_eq!(monitors[1].value_key, "__sprite/Sprite1/y");
    assert_eq!(monitors[2].value_key, "__sprite/Sprite1/direction");
    assert_eq!(monitors[3].value_key, "__sprite/Sprite2/x");

    let mut state = crate::runtime::ScratchState::new(Vec::new());
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    state.register_target(2, "Sprite2", false);
    let sprite1 = state.original_instance(1).unwrap();
    state.set_current_instance(sprite1);
    state.set_xy(12.0, -4.0);
    state.set_direction(45.0);
    let sprite2 = state.original_instance(2).unwrap();
    state.set_current_instance(sprite2);
    state.set_xy(99.0, 0.0);
    let snapshot = state.observable_snapshot(1);
    for monitor in &monitors[..3] {
        let target_property = monitor.value_key.rsplit('/').next().unwrap();
        let sprite = snapshot
            .layers
            .iter()
            .find(|layer| layer.target == "Sprite1")
            .unwrap();
        let actual = match target_property {
            "x" => sprite.x,
            "y" => sprite.y,
            "direction" => sprite.direction,
            _ => unreachable!(),
        };
        let expected = match target_property {
            "x" => 12.0,
            "y" => -4.0,
            "direction" => 45.0,
            _ => unreachable!(),
        };
        assert_eq!(actual, expected);
    }
    assert_eq!(
        snapshot
            .layers
            .iter()
            .find(|layer| layer.target == "Sprite2")
            .unwrap()
            .x,
        99.0
    );
}

#[test]
fn looks_size_monitor_keeps_sprite_ownership_and_uses_snapshot_key() {
    let project: aida_ast::parser::ScratchProject = serde_json::from_str(
            r#"{"targets":[
                {"name":"Stage","isStage":true},
                {"name":"Sprite1","isStage":false},
                {"name":"Sprite2","isStage":false}],
              "monitors":[
                {"id":"size1","mode":"default","opcode":"looks_size","params":{},"spriteName":"Sprite1","value":100,"visible":true},
                {"id":"size2","mode":"default","opcode":"looks_size","params":{},"spriteName":"Sprite2","value":100,"visible":true}] }"#,
        )
        .unwrap();
    let monitors = project_monitors(&project);
    assert_eq!(monitors[0].name, "Sprite1: size");
    assert_eq!(monitors[1].name, "Sprite2: size");
    assert_eq!(monitors[0].value_key, "__sprite/Sprite1/size");
    assert_eq!(monitors[1].value_key, "__sprite/Sprite2/size");
    assert_eq!(monitors[0].category, MonitorCategory::Looks);

    let mut state = crate::runtime::ScratchState::new(Vec::new());
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    state.register_target(2, "Sprite2", false);
    state.set_original_visual(1, 90.0, 135.0, true);
    state.set_original_visual(2, 90.0, 75.0, true);
    let snapshot = state.observable_snapshot(1);
    assert_eq!(
        snapshot
            .layers
            .iter()
            .find(|layer| layer.target == "Sprite1")
            .unwrap()
            .size,
        135.0
    );
    assert_eq!(
        snapshot
            .layers
            .iter()
            .find(|layer| layer.target == "Sprite2")
            .unwrap()
            .size,
        75.0
    );
}

#[test]
fn motion_point_fixture_monitor_records_read_sprite1_state() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../aida-ast/tests/src/motion-point-test.sb3"
    );
    let json = aida_ast::extractor::read_project_json(path).unwrap();
    let project = aida_ast::parser::parse_project(&json).unwrap();
    let monitors = project_monitors(&project);
    let expected = [
        (
            "motion_xposition",
            "Sprite1: x position",
            "__sprite/Sprite1/x",
            0.0,
        ),
        (
            "motion_yposition",
            "Sprite1: y position",
            "__sprite/Sprite1/y",
            0.0,
        ),
        (
            "motion_direction",
            "Sprite1: direction",
            "__sprite/Sprite1/direction",
            90.0,
        ),
    ];
    let mut state = crate::runtime::ScratchState::new(Vec::new());
    for (target_index, target) in project.targets.iter().enumerate() {
        state.register_target(target_index, target.name.clone(), target.is_stage);
        state.set_original_xy(target_index, target.x, target.y);
        state.set_original_visual(target_index, target.direction, target.size, target.visible);
    }
    let snapshot = state.observable_snapshot(0);
    let sprite = snapshot
        .layers
        .iter()
        .find(|layer| layer.target == "Sprite1")
        .unwrap();
    for (opcode, label, key, value) in expected {
        let monitor = monitors
            .iter()
            .find(|monitor| monitor.value_key == key)
            .unwrap();
        assert_eq!(monitor.name, label);
        assert!(project.monitors.iter().any(|record| {
            record.opcode == opcode && record.sprite_name.as_deref() == Some("Sprite1")
        }));
        let observed = match opcode {
            "motion_xposition" => sprite.x,
            "motion_yposition" => sprite.y,
            "motion_direction" => sprite.direction,
            _ => unreachable!(),
        };
        assert_eq!(observed, value);
    }
}

#[test]
fn reporter_categories_cover_scratch_core_groups_and_extensions() {
    let cases = [
        ("motion_xposition", MonitorCategory::Motion),
        ("looks_size", MonitorCategory::Looks),
        ("sound_volume", MonitorCategory::Sound),
        ("event_example", MonitorCategory::Events),
        ("control_example", MonitorCategory::Control),
        ("sensing_timer", MonitorCategory::Sensing),
        ("operator_add", MonitorCategory::Operators),
        ("procedures_reporter", MonitorCategory::MyBlocks),
        ("pen_reporter", MonitorCategory::Extension([15, 189, 140])),
    ];
    for (opcode, expected) in cases {
        assert_eq!(monitor_category(opcode, MonitorKind::Reporter), expected);
    }
}

#[test]
fn backdrop_monitor_label_includes_number_or_name_selector() {
    let number = std::collections::HashMap::from([(
        "NUMBER_NAME".to_owned(),
        serde_json::Value::String("number".to_owned()),
    )]);
    let name = std::collections::HashMap::from([(
        "NUMBER_NAME".to_owned(),
        serde_json::Value::String("name".to_owned()),
    )]);
    assert_eq!(
        monitor_label("looks_backdropnumbername", &number),
        "backdrop number"
    );
    assert_eq!(
        monitor_label("looks_backdropnumbername", &name),
        "backdrop name"
    );

    let project: aida_ast::parser::ScratchProject = serde_json::from_str(
            r#"{"targets":[{"name":"Stage","isStage":true}],"monitors":[
                {"id":"backdrop-number","mode":"default","opcode":"looks_backdropnumbername","params":{"NUMBER_NAME":"number"},"spriteName":null,"value":0,"visible":true},
                {"id":"backdrop-name","mode":"default","opcode":"looks_backdropnumbername","params":{"NUMBER_NAME":"name"},"spriteName":null,"value":"backdrop1","visible":true}
            ]}"#,
        )
        .unwrap();
    let monitors = project_monitors(&project);
    assert_eq!(monitors[0].value_key, "__stage/backdrop/number");
    assert_eq!(monitors[1].value_key, "__stage/backdrop/name");
}

#[test]
fn current_period_monitor_labels_are_lowercase_units_without_current_prefix() {
    for (period, expected) in [
        ("YEAR", "year"),
        ("MONTH", "month"),
        ("DATE", "date"),
        ("DAYOFWEEK", "day of week"),
        ("HOUR", "hour"),
        ("MINUTE", "minute"),
        ("SECOND", "second"),
    ] {
        let params = std::collections::HashMap::from([(
            "CURRENTMENU".to_owned(),
            serde_json::Value::String(period.to_owned()),
        )]);
        assert_eq!(monitor_label("sensing_current", &params), expected);
    }
}
