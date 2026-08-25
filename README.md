# DeepSeek Harness Desktop

A desktop shell for the [DeepSeek Harness](https://github.com/deepseek-ai) (`@deepseek-ai/dsh`),
built with [Tauri 2](https://v2.tauri.app).

This repository wraps the DeepSeek Harness engine (a Node.js runtime plus the
`@deepseek-ai/dsh` packages) inside a Tauri application, providing a native desktop
experience — system tray, native webview, and an installer — around the harness.

## Architecture

- `src-tauri/` — the Rust/Tauri application shell (product name **DeepSeek Harness**,
  identifier `com.deepseek.harness.desktop`).
- `src-tauri/resources/engine/` — the bundled Node.js engine and harness packages.
  Large binaries here are git-ignored and re-fetched via the helper scripts below.
- `src-tauri/assets/` — loading/splash UI and theme watcher.
- Root scripts (`fetch-pkgs.mjs`, `fetch-missing.mjs`, `extract-tgz.py`) — download and
  unpack the `@deepseek-ai/*` engine packages from the npm registry.

## Prerequisites

- Node.js (>= 18) and [pnpm](https://pnpm.io)
- Rust (stable) and Cargo
- Tauri 2 platform prerequisites: WebView2 (Windows) and the platform build tools
  (Windows SDK / C++ Build Tools)
- Python 3 (used by `extract-tgz.py` to unpack engine tarballs)

## Getting started

```bash
# 1. Install JS dependencies
pnpm install

# 2. Fetch the DeepSeek Harness engine packages into node_modules
node fetch-pkgs.mjs        # pulls @deepseek-ai/dsh and related packages
node fetch-missing.mjs     # fills in any additionally required packages

# 3. Run in development
pnpm tauri dev

# 4. Build a production installer (MSI on Windows)
pnpm tauri build
```

The production installer is produced under `src-tauri/target/release/bundle/`.

## Repository contents

Build artifacts and large binaries are intentionally excluded from version control via
`.gitignore`:

- `node_modules/`, `target/`, `dist/`
- `*.tgz`, `*.zip`
- `portable/`, `DeepSeek-Harness-portable/`
- `src-tauri/resources/engine/node.exe` and other engine binaries

The engine runtime is re-fetched with the scripts above, so a clean clone can be rebuilt
without committing the large binaries. **No secrets, keys, or `.env` files are tracked.**

## Configuration

Tauri configuration lives in `src-tauri/tauri.conf.json`. The harness engine is configured
through its own settings, which are stored on the local machine at runtime.

## License

No license file is included yet. Add a `LICENSE` before distributing.
