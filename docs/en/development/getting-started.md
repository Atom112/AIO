# Development Environment

## Required Dependencies

- Git;
- Node.js 20.19 or higher with npm;
- Rust 1.97.1 (the repository verification script uses this pinned toolchain, and requires `rustfmt` and `clippy`);
- [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform;
- The `aio-models-data` repository at the same level as AIO.

`package.json` uses `file:../aio-models-data`, and requires its `dist/data/models.json` during packaging:

```text
workspace/
├── AIO/
└── aio-models-data/
    └── dist/data/models.json
```

## Getting the Code

```bash
git clone https://github.com/Atom112/aio-models-data.git
git clone https://github.com/Atom112/AIO.git
cd AIO
npm ci
```

## Development and Checks

```bash
# Frontend development server
npm run dev

# Tauri desktop development
npm run tauri dev

# Frontend production build
npm run build

# Full verification before commit
npm run verify
```

`npm run dev` only starts Vite; it cannot verify Tauri invoke, file selection, system credential store, local engines or update functionality. Daily feature development should use `npm run tauri dev`.

The repository's `.cargo/config.toml` replaces crates.io with the rsproxy sparse mirror to avoid TLS handshake failures caused by IPv6 + Schannel on some Windows networks. Dependency versions and checksums are still constrained by `Cargo.lock`; if your organization requires direct access to the official registry, delete the local source replacement before building.

## Linux

The release workflow installs on Ubuntu 22.04:

```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libappindicator3-dev \
  librsvg2-dev \
  patchelf \
  rpm
```

For other distributions, use the equivalent package names from the Tauri documentation.

## Release Build

```bash
npm run tauri build
```

The Tauri configuration declares `deb`, `rpm`, `nsis`, `msi`, `app` and `dmg` targets. Actual releases are built by `.github/workflows/release.yml` on Windows, Ubuntu and both macOS architectures serially, also generating update metadata.

Local builds do not automatically acquire release signing keys. Do not use local packages as substitutes for official Releases when signing environment variables are missing.

## Before Committing

Run uniformly:

```bash
npm run verify
```

`verify` sequentially checks i18n dictionaries, documentation links, Prettier, ESLint, TypeScript, frontend production build, Rustfmt, Clippy strict mode and Rust tests. To isolate specific issues, run `npm run format:check`, `npm run lint`, `npm run typecheck`, `npm run check:frontend` or `npm run check:rust`.

When adding a new Rust Tauri command, also confirm module export, `lib.rs` registration and command doc comments. Use Tailwind utility classes for new UI styles; do not include emoji in code text.
