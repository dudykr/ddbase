# cargo link

Link local dependencies in a cargo project with ease.

# Installation

```bash
cargo install cargo-link2
```

# Usage

```bash
cargo link ~/path/to/dependency
```

```
Usage: cargo link [OPTIONS] <TARGET_DIR>

Arguments:
  <TARGET_DIR>
          The target directory to link to the current project.

          If the target directory is a cargo workspace, all packages in the workspace will be linked.

Options:
  -C, --dir <DIR>
          Changes the link location to <dir>.

          Defaults to the current directory.

  -h, --help
          Print help (see a summary with '-h')
```

# License

Apache-2.0
