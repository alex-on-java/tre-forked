# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

`tre` is a Rust CLI tool that serves as a modern alternative to the classic `tree` command. It displays directory structures as tree diagrams with additional features: respects `.gitignore`, supports `LS_COLORS`, and can generate shell aliases to quickly open displayed files.

## Build Commands

```bash
# Build (also generates shell completions in scripts/completion/)
make build
# or: SHELL_COMPLETIONS_DIR=scripts/completion cargo build

# Run checks (cargo check + clippy with warnings as errors)
make check

# Run tests (unit + integration)
make test
# or: cargo test

# Run a single test
cargo test <test_name>

# Build release binary
cargo build --release
```

## Architecture

The codebase follows a pipeline architecture for transforming directory contents into formatted output:

1. **CLI Parsing** (`cli.rs`): Uses `clap` derive macros to parse arguments into `Interface` struct
2. **Path Discovery** (`path_finders.rs`): Three strategies for finding files:
   - `find_non_git_ignored_paths`: Uses `git ls-files` to respect `.gitignore` (default)
   - `find_non_hidden_paths`: Excludes dotfiles without using git
   - `find_all_paths`: Shows everything including hidden files
3. **Tree Building** (`file_tree.rs`): Converts flat path list into `FileTree` using a `Slab` allocator for node storage and `IndexMap` for ordered children
4. **Formatting** (`diagram_formatting.rs`): Recursively traverses tree to generate `FormattedEntry` with box-drawing prefixes (├──, └──, │)
5. **Output** (`output.rs`): Prints with `LS_COLORS` support via `lscolors` crate; writes shell alias files to `/tmp/tre_aliases_$USER`

Alternative output: `json_formatting.rs` produces JSON instead of tree diagrams.

## Key Implementation Details

- Shell completions are generated at build time via `build.rs` using `clap_complete`
- Windows vs Unix: Different alias file formats (.ps1/.bat vs shell aliases), different default editors
- The `-e` flag enables editor aliasing where `e<N>` aliases are created for each displayed file
- Regex exclusion patterns (`-E`) are compiled once and applied during path filtering

## Testing

Integration tests in `tests/integration_tests.rs` use the `fixtures/` directory and `assert_cmd` to test the binary. Tests verify gitignore behavior, hidden file handling, and the `-a`/`-s` flags.
