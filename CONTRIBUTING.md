# Contributing to Claw Agent RS

Thanks for your interest in contributing! Here's how to get started.

## Development Setup

```bash
# Clone the repo
git clone https://github.com/piyanggoon/claw-agent-rs.git
cd claw-agent-rs

# Copy environment config
cp .env.example .env
# Edit .env — add your ANTHROPIC_API_KEY

# Build
cargo build

# Run tests
cargo test

# Run the server
cargo run
```

**Requirements:**
- Rust 1.85+ (edition 2024)
- An Anthropic API key

## Project Structure

```
src/
├── agent/       # Agent runner + LLM provider
├── db/          # SQLite schema + CRUD
├── memory/      # Memory manager (save/recall/forget)
├── scheduler/   # Task scheduler engine
├── soul/        # Soul file I/O + markdown parser + prompt builder
├── tools/       # All 21 custom tools
└── web/         # Axum HTTP server + routes
```

See [AGENTS.md](AGENTS.md) for detailed architecture documentation.

## How to Contribute

### Reporting Bugs

Open an issue with:
- Steps to reproduce
- Expected vs actual behavior
- Rust version (`rustc --version`)
- OS and environment

### Suggesting Features

Open an issue with:
- Use case description
- Proposed solution (optional)

### Submitting Code

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/my-feature`
3. Make your changes
4. Ensure tests pass: `cargo test`
5. Ensure it compiles cleanly: `cargo check`
6. Commit with a clear message
7. Push and open a Pull Request

### Adding a New Tool

1. Create the tool struct in the appropriate file under `src/tools/`
2. Implement `Tool<ClawContext>` trait (see existing tools for pattern)
3. Register it in `src/tools/mod.rs::register_all_tools()`
4. Add a test in `tests/tools_integration.rs`
5. Document it in `groups/default/AGENTS.md` (agent instructions)

### Adding a New LLM Provider

1. Add the provider in `src/agent/provider.rs`
2. Add routing logic based on model name prefix
3. Ensure the corresponding API key config exists in `src/config.rs`

## Code Style

- Follow standard Rust conventions (`cargo fmt`, `cargo clippy`)
- Use `anyhow::Result` for error handling
- Keep tools self-contained — each tool should handle its own errors gracefully
- Return `ToolResult::error(...)` instead of propagating errors with `?` in tool `execute()`
- Use `tracing` for logging (not `println!`)

## Testing

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_01_soul_read
```

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
