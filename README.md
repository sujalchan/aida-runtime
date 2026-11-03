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

The runtime defaults to 30 FPS. `--fps <number>` selects another positive,
finite tick rate, and `--turbo` disables frame pacing while continuing to honor
Scratch `wait` blocks. See the [base project README](https://github.com/sujalchan/aida)
for the complete run and test guide.
