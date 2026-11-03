# aida-runtime

`aida-runtime` executes Rust programs generated from Scratch (`.sb3`) projects
by [`aida-ast`](https://github.com/sujalchan/aida-ast). The generated project
module is `src/program.rs`; the runtime crate provides the scheduler and block
execution support used by that module.

## Repository layout

For the complete workflow, `aida-runtime` needs `aida-ast` alongside it in the
base `aida` project. The base repository is
[`sujalchan/aida`](https://github.com/sujalchan/aida), which includes both
repositories as Git submodules. Clone the base project and initialize its
submodules:

```sh
git clone --recurse-submodules https://github.com/sujalchan/aida.git
cd aida
```

If you already cloned the base project without the component repositories:

```sh
git submodule update --init --recursive
```

The directories should look like this:

```text
aida/
├── Cargo.toml
├── aida-ast/
└── aida-runtime/
```

Run Cargo commands from the `aida` root. This lets Cargo find the workspace
members and lets `aida-ast` write generated code to
`aida-runtime/src/program.rs`.

## Generate and run a project

From the base project root, generate the runtime module from a Scratch project
and run it:

```sh
cargo run -p aida-ast -- --runtime aida-runtime/src/program.rs path/to/project.sb3
cargo run -p aida-runtime -- --turbo
```

To opt into generated fast maths paths, add `--fast-math` to the `aida-ast`
command. Do not pass it to `aida-runtime`:

```sh
cargo run -p aida-ast -- --fast-math --runtime aida-runtime/src/program.rs path/to/project.sb3
cargo run -p aida-runtime -- --turbo
```

The root launchers combine those steps. For example:

```sh
./run.sh path/to/project.sb3 --turbo
```

On Windows, use `run.bat` from the same root:

```powershell
.\run.bat path\to\project.sb3 --turbo
```

Headless mode is the default and creates no window or graphics device. The
runtime defaults to 30 FPS. `--fps <number>` selects another positive, finite
tick rate, and `--turbo` disables frame pacing while continuing to honor Scratch
`wait` blocks.

## Graphical and headless observation

The generated program and scheduler remain the source of live AIDA state in
both modes. The graphical renderer runs on the main thread and reads snapshots
published by the execution worker. Headless mode starts neither a renderer
thread nor winit/wgpu resources. Variable and output logging is opt-in.

`aida-runtime` does not launch or require the `aida-ast` executable at runtime.
It executes the already-generated `src/program.rs` through AIDA's existing
`GeneratedProgram` and `ScriptScheduler`. The renderer's `.sb3` input is read
through the `aida-ast` library API for costumes and monitor layout only; it does
not generate or run scripts. `ScratchState` remains the canonical variable,
list, clone, and position state. The renderer keeps only display-side caches,
updated from immutable snapshots of that state.

After `src/program.rs` has been generated, the runtime can be built and run
without invoking the AST executable:

```sh
cargo build -p aida-runtime
cargo run -p aida-runtime -- --headless --turbo
cargo run -p aida-runtime -- --graphical --project path/to/the-same-project.sb3
```

The graphical project path must match the project used to generate
`src/program.rs`; it supplies graphics metadata and assets, not executable
scripts.

Use the root launcher to generate and run the same project graphically:

```sh
./run.sh path/to/project.sb3 --graphical
./run.sh path/to/project.sb3 --graphical --turbo
```

For direct runtime invocation, first generate the module from that project,
then pass its path to the graphical frontend:

```sh
cargo run -p aida-ast -- --runtime aida-runtime/src/program.rs path/to/project.sb3
cargo run -p aida-runtime -- --graphical --project path/to/project.sb3
```

Headless observation supports ordered `say` output and variable-change events,
plus a final variable/list snapshot:

```sh
cargo run -p aida-runtime -- --headless --turbo --log-observables --log-variables
cargo run -p aida-runtime -- --headless --max-ticks 100 --expect-variable 'Stage::score#variable-id@shared=42'
```

Variable keys include target, display name, Scratch ID, and instance identity:
`Target::name#id@shared` for stage variables and
`Target::name#id@<instance>` for sprite-local values. Expected values use JSON
numbers, strings, or booleans. `--max-ticks` is available in either mode to
bound projects that intentionally keep running.

The renderer displays the Scratch stage and variable monitors, follows live
variable and sprite position snapshots, handles window resizing, and renders
clones as separate sprite instances. Observable event delivery uses an ordered
queue; snapshots are latest-value updates and can be coalesced while the window
is busy.

See the [base project README](https://github.com/sujalchan/aida) for the
complete run and test guide.
