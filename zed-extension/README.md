# Isabelle Zed Extension

Isabelle theorem prover support for the Zed code editor.

## Installation

1. Build the extension:
   ```bash
   cargo build --release
   ```

2. Load in Zed:
   - Open Zed
   - Run command: `extension: dev`
   - Select the `zed-extension` directory

## Features

- **Language Support**: `.thy` files (Isabelle theory files)
- **Diagnostics**: Error/warning markers from Isabelle
- **Hover**: Theorem information on mouse hover
- **Commands**:
  - `isabelle.start_session` - Start Isabelle session
  - `isabelle.stop_session` - Stop Isabelle session
  - `isabelle.run_check` - Run document check

## Development

### Prerequisites

- Rust 1.70+
- Zed (for loading the extension)

### Build

```bash
cd zed-extension
cargo build --release
```

### Testing

```bash
cargo test
```

## Architecture

```
┌─────────────┐     NDJSON      ┌──────────┐     NDJSON      ┌──────────────┐
│    Zed      │ ──────────────►│  Bridge  │ ──────────────►│ Scala Adapter│
│  Extension  │◄──────────────│  (Rust)  │◄────────────────│   (Scala)    │
└─────────────┘                └──────────┘                 └──────────────┘
       │                              │                              │
       │ Diagnostics                 │                              │
       │◄───────────────────────────│                              │
```

## Configuration

The extension connects to a Unix socket at `/tmp/isabelle.sock` by default.

To configure:
1. Start the bridge: `bridge --socket /tmp/isabelle.sock --mock`
2. Start the Scala adapter (if using real Isabelle): `isabelle scala`
3. Load the extension in Zed
