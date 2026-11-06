use std::sync::mpsc;
use std::{fmt, io};

use crate::graphics::ListMonitorFileAction;
use crate::runtime::RuntimeInputEvent;

#[derive(Debug)]
pub(super) enum ListIoError {
    Read(io::Error),
    Write(io::Error),
    Parse(csv::Error),
}

impl fmt::Display for ListIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "failed to read imported list: {error}"),
            Self::Write(error) => write!(formatter, "failed to write exported list: {error}"),
            Self::Parse(error) => write!(formatter, "invalid list file: {error}"),
        }
    }
}

impl std::error::Error for ListIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) | Self::Write(error) => Some(error),
            Self::Parse(error) => Some(error),
        }
    }
}

pub(super) fn handle_list_monitor_file_action(
    action: ListMonitorFileAction,
    input_sender: &mpsc::Sender<RuntimeInputEvent>,
) -> Result<(), ListIoError> {
    match action {
        ListMonitorFileAction::Import { key } => {
            let path = rfd::FileDialog::new()
                .add_filter("list files", &["csv", "tsv", "txt"])
                .pick_file();
            let Some(path) = path else {
                return Ok(());
            };
            let contents = std::fs::read_to_string(path).map_err(ListIoError::Read)?;
            let items = parse_scratch_list_import(&contents).map_err(ListIoError::Parse)?;
            let _ = input_sender.send(RuntimeInputEvent::ListReplace { key, items });
        }
        ListMonitorFileAction::Export {
            file_name,
            contents,
        } => {
            let path = rfd::FileDialog::new()
                .add_filter("text file", &["txt"])
                .set_file_name(&file_name)
                .save_file();
            if let Some(path) = path {
                std::fs::write(path, contents).map_err(ListIoError::Write)?;
            }
        }
    }
    Ok(())
}

pub(super) fn parse_scratch_list_import(contents: &str) -> Result<Vec<String>, csv::Error> {
    let contents = contents.trim_start_matches('\u{feff}');
    let delimiter = detect_list_delimiter(contents);
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(contents.as_bytes());
    reader
        .records()
        .map(|record| record.map(|row| row.get(0).map(str::to_owned)))
        .filter_map(|record| match record {
            Ok(Some(value)) => Some(Ok(value)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn detect_list_delimiter(contents: &str) -> u8 {
    let mut counts = [(b',', 0usize), (b'\t', 0), (b';', 0), (b'|', 0)];
    for line in contents.lines().take(10) {
        let mut quoted = false;
        let mut chars = line.bytes().peekable();
        while let Some(character) = chars.next() {
            if character == b'"' {
                if quoted && chars.peek() == Some(&b'"') {
                    chars.next();
                } else {
                    quoted = !quoted;
                }
            } else if !quoted {
                for (delimiter, count) in &mut counts {
                    if character == *delimiter {
                        *count += 1;
                    }
                }
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| *count > 0)
        .map_or(b',', |(delimiter, _)| delimiter)
}
