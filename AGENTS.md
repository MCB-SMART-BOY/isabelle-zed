# AGENTS.md - Isabelle-Zed Development Guide

This is a Rust project (edition 2024). The codebase is minimal and follows standard Rust conventions.

## Build Commands

```bash
# Build the project
cargo build

# Build in release mode
cargo build --release

# Run the application
cargo run

# Run in release mode
cargo run --release
```

## Test Commands

```bash
# Run all tests
cargo test

# Run a single test by name
cargo test test_name

# Run tests with output
cargo test -- --nocapture

# Run doc tests
cargo test --doc
```

## Lint and Formatting

```bash
# Format code (follows rustfmt defaults)
cargo fmt

# Check formatting without making changes
cargo fmt -- --check

# Run clippy lints
cargo clippy

# Run clippy with all warnings treated as errors
cargo clippy -- -D warnings

# Run clippy on a single crate
cargo clippy -p isabelle-zed
```

## Type Checking

```bash
# Type check without building
cargo check

# Check a specific target
cargo check --target x86_64-unknown-linux-gnu
```

## Code Style Guidelines

### General Principles
- Follow standard Rust idioms and conventions
- Use `rustfmt` for formatting (run `cargo fmt` before committing)
- Address all clippy warnings

### Imports
- Use absolute paths for external crates: `use crate::module::Item`
- Group imports: standard library first, then external crates, then local modules
- Use `use` statements for bringing items into scope, avoid full paths in code

### Formatting
- Use 4 spaces for indentation
- Maximum line length: 100 characters (soft limit, use judgment)
- Add trailing commas in multi-line collections
- Match arms should have consistent braces

### Types
- Prefer explicit type annotations for function signatures
- Use generic types when appropriate for code reuse
- Prefer owned types (`String`, `Vec<T>`) over borrowed types when semantics allow

### Naming Conventions
- **Variables/Functions**: `snake_case` (e.g., `calculate_value`, `max_entries`)
- **Types/Enums**: `PascalCase` (e.g., `Config`, `ErrorKind`)
- **Constants**: `SCREAMING_SNAKE_CASE` (e.g., `MAX_BUFFER_SIZE`)
- **Modules**: `snake_case`
- Be descriptive: prefer `user_count` over `n` or `cnt`

### Error Handling
- Use `Result<T, E>` for functions that can fail
- Use appropriate error types (custom enums, `thiserror`, `anyhow`)
- Avoid `unwrap()` in production code; use `?` or explicit error handling
- Use `expect()` only for truly impossible failures with clear messages

### Patterns to Prefer
- **Early returns**: Return early for error cases or edge conditions
- **Builder pattern**: For complex struct construction
- **Iterator chains**: Prefer iterators over manual loops when concise
- **Match exhaustiveness**: Handle all enum variants explicitly
- **Clap/StructOpt**: For CLI argument parsing

### Patterns to Avoid
- `unsafe` blocks unless absolutely necessary
- Global mutable state
- Excessive `unwrap()` or `expect()`
- Unnecessary boxing (`Box<T>`) when stack allocation works

### Testing
- Unit tests go in the same file (within `#[cfg(test)]` module) or `tests/` directory
- Integration tests go in `tests/` directory
- Follow AAA (Arrange-Act-Assert) pattern for test clarity
- Use descriptive test names: `test_function_name_when_condition()`

## Project Structure

```
src/
  main.rs           # Application entry point
  lib.rs             # Library crate (if any)
  # Add modules as needed
tests/
  # Integration tests
```

## Additional Notes

- This project uses Rust edition 2024 (experimental). Ensure nightly toolchain is available if needed.
- Check `Cargo.toml` for current dependencies and configuration
- Run `cargo tree` to view dependency graph
