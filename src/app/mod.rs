mod cli;
mod headless;
mod input;
mod list_io;
mod runner;
mod window;
mod worker;

use cli::RunOptions;

#[cfg(test)]
mod tests;

pub(crate) use cli::parse_run_options;

#[cfg(test)]
pub(crate) use headless::{assert_expected_variables, run_headless};
#[cfg(test)]
pub(crate) use runner::project_window_title;
#[cfg(test)]
pub(crate) use worker::{apply_input_batch, enqueue_pen_commands, wait_for_renderer};

pub(crate) fn run(options: RunOptions) -> Result<(), String> {
    runner::run(options)
}
