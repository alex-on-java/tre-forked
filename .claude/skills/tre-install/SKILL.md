---
name: tre-install
description: |
  This skill installs tre locally from source. Use this skill when the user asks to build, install, or update tre on their machine.
---

# Tre Install

Install tre (a modern `tree` alternative) from source.

## Prerequisites

- Rust toolchain (`rustup` recommended, present on the machine)

## Version Bump

To release a new version, edit `version` in `Cargo.toml` before building.
Ask user explicitly, whether he wants to bump it, using `AskUserQuestion` tool

## Installation

To install tre locally, run the install script:

```bash
scripts/install.sh
```

The script will:
1. Build tre in release mode with `cargo build --release`
2. Copy the binary to `/usr/local/bin/` and `~/.cargo/bin/`
3. Verify the installation with `tre --version`

## Manual Steps

If needed, individual steps can be run manually:

```bash
# Build
cargo build --release

# Install (both locations to avoid PATH shadowing)
cp target/release/tre /usr/local/bin/
cp target/release/tre ~/.cargo/bin/

# Verify
tre --version
```
