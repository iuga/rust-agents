# Rust Agent
Agent Network written in Rust

## Desktop chat

Run from the repository root with your existing `settings.yaml` and Ollama models:

```sh
make chat
# or: cargo run --features desktop --bin chat
```

The native Rust UI uses GPUI Kit, a dark theme, and one active Alpha conversation.
Replies stream as they arrive. Expand a tool entry to see its arguments, result,
and running/completed/failed status. Alpha's delegations to Beta and Gamma appear
as tool calls; their internal tool activity is not expanded into separate entries.

- **Enter** sends; **Shift+Enter** inserts a newline.
- **Close / Cmd+W** hides the window to the Bot menu-bar icon. The draft,
  conversation, and any active request remain alive.
- **Bot → Open Chat** restores the window; **Bot → Quit / Cmd+Q** exits.
- Messages and tool activity stay in memory and are discarded on exit. The
  existing knowledge index is still stored on disk.

Desktop integration targets macOS (15+ with Xcode Command Line Tools). The first
build compiles GPUI and can take several minutes. `cargo run` still starts the MCP
server; its build does not enable desktop dependencies.

### Checks

```sh
cargo check --workspace --all-targets --features desktop --locked
cargo clippy --workspace --all-targets --features desktop -- -D warnings
cargo test --workspace --features desktop --locked
```

Manual smoke test: ask Alpha to generate a UUID, expand the Beta tool call, send a
follow-up, and close/reopen the window during a response. The same conversation
should remain visible. Quit and relaunch to confirm the transcript starts empty.

# Backlog
- [] Just expose one agent (alpha, the leader and coordinator)
- [] RAG should be async and do not block the main thread
- [] RAG should keep files up to date
- [] RAG should understand about the model
- [] Agent Skills
