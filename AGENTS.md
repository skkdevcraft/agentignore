# AGENTS.md

> Guidance for AI agents working in this Rust codebase.

---

## Toolchain & Environment

- **Rust edition:** 2024
- **Package manager / build tool:** Cargo (do not invoke `rustc` directly)
- **Minimum Supported Rust Version (MSRV):** defined in `Cargo.toml` under `[package] rust-version`

Check versions before making toolchain assumptions:

```bash
rustc --version
cargo --version
```

---

## Project Layout

```
agentignore/
├── Cargo.toml          # Workspace or crate manifest
├── Cargo.lock          # Committed for binaries; excluded for libraries
├── src/
│   ├── main.rs         # Binary entry point
│   ├── lib.rs          # Library root (if dual crate)
│   └── **/*.rs         # Modules
├── tests/              # Integration tests (each file = separate test binary)
├── benches/            # Criterion benchmarks
├── examples/           # Runnable examples (`cargo run --example <name>`)
└── AGENTS.md           # This file
```

If this is a Cargo workspace, each crate lives under a subdirectory with its own `Cargo.toml`.

---

## Build Commands

Always use these commands — never compile files manually.

```bash
# Fast type-check (no binary produced — use constantly during edits)
cargo check

# Debug build
cargo build

# Optimized release build (use for perf testing)
cargo build --release

# Run the binary
cargo run
cargo run --release

# Run a specific example
cargo run --example <name>
```

---

## Testing

```bash
# Run all tests
cargo test

# Run a specific test by name (substring match)
cargo test <test_name>

# Run tests in a specific module
cargo test <module>::

# Run integration tests only
cargo test --test <filename>

# Show stdout from passing tests (suppressed by default)
cargo test -- --nocapture

# Run tests single-threaded (when tests share global state)
cargo test -- --test-threads=1
```

Tests live:
- **Unit tests:** in the same `.rs` file as the code, inside `#[cfg(test)] mod tests { ... }`
- **Integration tests:** in `tests/` — they can only use the public API
- **Doc tests:** in `///` doc comments — these are compiled and run by `cargo test`

---

## Code Quality

Run these before finishing any task. All must pass with zero warnings.

```bash
# Linter — treat all warnings as errors
cargo clippy -- -D warnings

# Auto-formatter — always run before committing
cargo fmt

# Check formatting without modifying files (for CI)
cargo fmt -- --check

# Audit dependencies for known vulnerabilities
cargo audit        # install with: cargo install cargo-audit
```

**Clippy is authoritative.** If Clippy flags something, fix the code — do not add `#[allow(...)]` unless there is a documented reason in a comment on the same line.

---

## Dependency Management

```bash
# Add a dependency
cargo add <crate>

# Add with specific features
cargo add <crate> --features <feature1>,<feature2>

# Add a dev-only dependency (tests/benches)
cargo add <crate> --dev

# Remove a dependency
cargo remove <crate>

# Show dependency tree
cargo tree

# Check for outdated dependencies
cargo outdated      # install with: cargo install cargo-outdated
```

### Dependency policy

- Prefer crates with >1M downloads and active maintenance on [crates.io](https://crates.io).
- Minimise the dependency footprint — do not add a crate just for one small utility.
- Prefer `serde` for serialisation, `thiserror` for library errors, `anyhow` for application errors, `tokio` for async runtimes.
- Never add a dependency that pulls in `unsafe` code without explicit approval.

---

## Rust-Specific Coding Rules

### Ownership & Borrowing

- Prefer borrowing (`&T`, `&mut T`) over cloning unless ownership is semantically required.
- Only `.clone()` when necessary — flag every `.clone()` call with a mental note about whether it's avoidable.
- Avoid `Rc<RefCell<T>>` unless modelling a genuinely shared ownership graph; prefer restructuring data.

### Error Handling

- **Libraries:** define a crate-local error type using `thiserror`. Never use `unwrap()` or `expect()` in library code.
- **Binaries / application code:** use `anyhow::Result` for propagation. `unwrap()` is acceptable only in `main()` on startup configuration — add a comment explaining why a failure here is unrecoverable.
- Use the `?` operator for propagation — never manually `match` on `Result` just to re-wrap.
- Never use `.unwrap()` in test helpers that run in non-test contexts.

```rust
// Good
fn parse_config(path: &Path) -> Result<Config, ConfigError> { ... }

// Bad — panics on error
fn parse_config(path: &Path) -> Config { ... }
```

### Panics

- No `panic!()`, `unwrap()`, or `expect()` in production paths without an explicit `// SAFETY:` or `// PANIC:` comment explaining the invariant that guarantees it never fires.
- Prefer `expect("descriptive message")` over bare `unwrap()` when a panic is intentional (e.g. programmer error, not recoverable user input).

### Lifetimes

- Let the compiler infer lifetimes wherever possible via elision.
- Add explicit lifetime annotations only when the compiler requires them or when they clarify intent.
- Avoid `'static` bounds unless the type genuinely needs to live for the program lifetime.

### Unsafe

- **Do not write `unsafe` code** without an explicit discussion and documented `// SAFETY:` comment explaining every invariant being upheld.
- Prefer safe abstractions over raw pointer manipulation.

### Iterators & Functional Style

- Prefer iterator chains over manual `for` loops when it improves clarity.
- Avoid `collect()`-ing into a `Vec` only to immediately iterate again — chain the operations.

### Structs & Enums

- Derive `Debug` on all public and most private types.
- Derive `Clone` only when cloning makes semantic sense for the type.
- Use `#[non_exhaustive]` on public enums in library crates to preserve future extensibility.
- Prefer `enum` over boolean parameters in public APIs (`enum Direction { Ascending, Descending }` not `bool`).

### Naming Conventions

Follow Rust's standard naming conventions — the compiler and Clippy will warn on violations:

| Item | Convention | Example |
|------|-----------|---------|
| Types, traits, enums | `UpperCamelCase` | `HttpClient` |
| Functions, methods, variables | `snake_case` | `parse_header` |
| Constants, statics | `SCREAMING_SNAKE_CASE` | `MAX_RETRIES` |
| Modules | `snake_case` | `http_client` |
| Lifetimes | short lowercase | `'a`, `'buf` |

### Module Organisation

- Keep modules small and focused.
- Use `pub(crate)` instead of `pub` for items that must be visible within the crate but are not part of the public API.
- Avoid `pub use` re-exports unless deliberately flattening a public API surface.

---

## Documentation

- All `pub` functions, structs, enums, and modules must have `///` doc comments.
- Include at least one `# Examples` section in doc comments for non-trivial public functions — these are compiled as doc tests.
- Use `cargo doc --open` to review rendered documentation.

```rust
/// Parses a duration from a human-readable string like `"5s"` or `"2m30s"`.
///
/// # Errors
///
/// Returns `ParseError::InvalidFormat` if the string does not match the expected pattern.
///
/// # Examples
///
/// ```
/// let d = parse_duration("1m30s").unwrap();
/// assert_eq!(d.as_secs(), 90);
/// ```
pub fn parse_duration(s: &str) -> Result<Duration, ParseError> { ... }
```

---

## Async Code (if applicable)

- Use `tokio` as the async runtime unless the project specifies otherwise.
- Annotate the entry point with `#[tokio::main]`.
- Do not mix blocking I/O with async code — use `tokio::task::spawn_blocking` for blocking operations inside async contexts.
- Prefer `async fn` over manually constructing `Future` types.
- Never `.await` inside a `Mutex` lock guard — this can deadlock; use `tokio::sync::Mutex` if you must hold a lock across an await point.

---

## Performance

- Profile before optimising — use `cargo build --release` and a profiler (`perf`, `flamegraph`, `cargo-flamegraph`).
- Benchmarks live in `benches/` and use the `criterion` crate.
- Do not introduce allocations in hot paths without measurement justification.

---

## Git & Commit Hygiene

- Each commit should be a logical, atomic unit of work.
- Commit message format: `<type>(<scope>): <short description>` (e.g. `fix(parser): handle empty input gracefully`)
- Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `perf`
- Do not commit with Clippy warnings or `cargo fmt` diff present.
- Do not commit `target/` — it is in `.gitignore`.

---

## What Agents Should Always Do

- [ ] Run `cargo check` after every non-trivial edit to catch errors early.
- [ ] Run `cargo clippy -- -D warnings` before marking a task done.
- [ ] Run `cargo fmt` before committing.
- [ ] Run `cargo test` and ensure all tests pass.
- [ ] Add or update tests for any logic changes.
- [ ] Update doc comments when changing public API signatures.

## What Agents Must Never Do

- Invoke `rustc` directly — always go through `cargo`.
- Add `#[allow(clippy::...)]` without a code comment explaining why.
- Use `.unwrap()` in library code or user-facing error paths.
- Introduce `unsafe` blocks without a `// SAFETY:` comment.
- Commit code that does not compile (`cargo check` fails).
- Add dependencies without checking they are actively maintained.
- Modify `Cargo.lock` manually — it is managed by Cargo.