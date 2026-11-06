use std::path::PathBuf;

use crate::runtime::{PacingMode, ScratchValue};

#[derive(Debug, Clone)]
pub(crate) struct RunOptions {
    pub(crate) pacing: PacingMode,
    pub(crate) max_ticks: Option<usize>,
    pub(crate) graphical: bool,
    pub(crate) dev_mode: bool,
    pub(crate) test_mode: bool,
    pub(crate) hd_pen_mode: bool,
    pub(crate) username: Option<String>,
    pub(crate) project: Option<PathBuf>,
    pub(crate) log_observables: bool,
    pub(crate) log_variables: bool,
    pub(crate) expected_variables: Vec<(String, ScratchValue)>,
}

pub(crate) fn parse_run_options(
    args: impl IntoIterator<Item = String>,
) -> Result<RunOptions, String> {
    let mut fps = None;
    let mut turbo = false;
    let mut max_ticks = None;
    let mut graphical = false;
    let mut headless = false;
    let mut dev_mode = false;
    let mut test_mode = false;
    let mut hd_pen_mode = false;
    let mut username = None;
    let mut project = None;
    let mut log_observables = false;
    let mut log_variables = false;
    let mut expected_variables = Vec::new();
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--fps" => {
                if turbo {
                    return Err("--fps and --turbo cannot be used together".to_owned());
                }
                if fps.is_some() {
                    return Err("--fps may only be specified once".to_owned());
                }
                let value = args.next().ok_or("--fps requires a number")?;
                let value = value.parse::<f64>().map_err(|_| {
                    format!("invalid FPS value {value:?}; expected a positive number")
                })?;
                fps = Some(PacingMode::from_fps(value)?);
            }
            "--turbo" => {
                if fps.is_some() {
                    return Err("--fps and --turbo cannot be used together".to_owned());
                }
                if turbo {
                    return Err("--turbo may only be specified once".to_owned());
                }
                turbo = true;
            }
            "--max-ticks" => {
                if max_ticks.is_some() {
                    return Err("--max-ticks may only be specified once".to_owned());
                }
                let value = args
                    .next()
                    .ok_or("--max-ticks requires a positive integer")?;
                let value = value.parse::<usize>().map_err(|_| {
                    format!("invalid tick limit {value:?}; expected a positive integer")
                })?;
                if value == 0 {
                    return Err("--max-ticks must be greater than zero".to_owned());
                }
                max_ticks = Some(value);
            }
            "--headless" => {
                if headless || graphical {
                    return Err("choose either --headless or --graphical once".to_owned());
                }
                headless = true;
            }
            "--graphical" => {
                if graphical || headless {
                    return Err("choose either --headless or --graphical once".to_owned());
                }
                graphical = true;
            }
            "--dev" => dev_mode = true,
            "--hd-pen" => {
                if hd_pen_mode {
                    return Err("--hd-pen may only be specified once".to_owned());
                }
                hd_pen_mode = true;
            }
            "--username" => {
                if username.is_some() {
                    return Err("--username may only be specified once".to_owned());
                }
                let value = args.next().ok_or("--username requires a name")?;
                if value.is_empty() {
                    return Err("--username requires a non-empty name".to_owned());
                }
                username = Some(value);
            }
            "--test" => {
                if test_mode {
                    return Err("--test may only be specified once".to_owned());
                }
                test_mode = true;
            }
            "--project" => {
                if project.is_some() {
                    return Err("--project may only be specified once".to_owned());
                }
                project = Some(
                    args.next()
                        .ok_or("--project requires an .sb3 project path")?
                        .into(),
                );
            }
            "--log-observables" => {
                if log_observables {
                    return Err("--log-observables may only be specified once".to_owned());
                }
                log_observables = true;
            }
            "--log-variables" => {
                if log_variables {
                    return Err("--log-variables may only be specified once".to_owned());
                }
                log_variables = true;
            }
            "--expect-variable" => {
                let expression = args.next().ok_or("--expect-variable requires KEY=JSON")?;
                expected_variables.push(parse_expected_variable(&expression)?);
            }
            _ => return Err(format!("unknown option {argument:?}")),
        }
    }

    let pacing = if turbo {
        PacingMode::Turbo
    } else if let Some(fps) = fps {
        fps
    } else {
        PacingMode::from_fps(PacingMode::DEFAULT_FPS)?
    };
    if hd_pen_mode && !graphical {
        return Err("--hd-pen requires --graphical".to_owned());
    }
    Ok(RunOptions {
        pacing,
        max_ticks,
        graphical,
        dev_mode,
        test_mode,
        hd_pen_mode,
        username,
        project,
        log_observables,
        log_variables,
        expected_variables,
    })
}

pub(crate) fn parse_expected_variable(value: &str) -> Result<(String, ScratchValue), String> {
    let (key, value) = value
        .split_once('=')
        .ok_or("--expect-variable requires KEY=JSON")?;
    if key.is_empty() {
        return Err("--expect-variable requires a non-empty variable key".to_owned());
    }
    let value: serde_json::Value = serde_json::from_str(value)
        .map_err(|error| format!("invalid expected variable value: {error}"))?;
    let value = match value {
        serde_json::Value::Number(number) => ScratchValue::Number(
            number
                .as_f64()
                .ok_or("expected numeric variable value is out of range")?,
        ),
        serde_json::Value::String(text) => ScratchValue::Text(text),
        serde_json::Value::Bool(value) => ScratchValue::Boolean(value),
        _ => return Err("expected variable value must be a number, string, or boolean".to_owned()),
    };
    Ok((key.to_owned(), value))
}
