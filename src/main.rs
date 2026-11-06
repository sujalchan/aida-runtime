#[rustfmt::skip]
mod program;
mod app;
mod graphics;

// Generated project code uses the runtime library API through this crate alias.
pub use aida_runtime::runtime;

fn main() {
    let options = match app::parse_run_options(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!(
                "error: {message}\nusage: aida-runtime [--headless | --graphical --project <file.sb3>] [--test] [--dev] [--hd-pen] [--username <name>] [--fps <number> | --turbo] [--max-ticks <count>] [--log-observables] [--log-variables] [--expect-variable <key>=<json-value>]"
            );
            std::process::exit(2);
        }
    };
    if let Err(error) = app::run(options) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;
