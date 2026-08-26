# mqx

mqx is a desktop MQTT explorer. Connect to a broker, watch the topic tree fill in as messages arrive, inspect payloads, and keep connection profiles (including TLS files) on disk.

One window is one broker. Saved profiles remember host, protocol, subscriptions, and TLS. JSON payloads pretty-print in place; jq filters and fuzzy topic search sit next to the tree. Passwords go in the OS keychain (or a private file when no keychain is available).

## Develop

Requires [Rust stable](https://rustup.rs/) and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

```bash
npm install
npm run tauri dev
```

Layout:

- `ui/` — Vite + Svelte 5 frontend
- `src-tauri/` — Tauri 2 shell and IPC (`mqx` package)
- `crates/mqx-core/` — UI-agnostic profiles, tree, and decode

```bash
npm run check
cargo test -p mqx-core
cargo check -p mqx
```

Profiles are stored under the `mqx` application directory. Theme is stored in `config.toml` (`[ui] theme = "dark" | "light" | "system"`).

Native menus: **mqx**, **Connections**, **Edit**, **View**, **Help**. `Cmd/Ctrl+,` opens settings. `Cmd/Ctrl+K` focuses topic search.

## Releases

Push a `v*` tag to run [`.github/workflows/release.yml`](.github/workflows/release.yml). The workflow builds `aarch64-apple-darwin` and `x86_64-apple-darwin` on `macos-latest`, plus `windows-latest` and `ubuntu-22.04`, with [`tauri-apps/tauri-action`](https://github.com/tauri-apps/tauri-action) and opens a **draft** GitHub Release.

Unsigned artifacts are acceptable until signing certificates exist. macOS/Windows “easy download” (no Gatekeeper/SmartScreen warning) needs the secrets below.

### Signing secrets

Configure these repository secrets when you have the material. The workflows do **not** require them; missing secrets skip that kind of signing.

| Secret | Purpose |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded Apple Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | Password for that `.p12` |
| `APPLE_SIGNING_IDENTITY` | Codesign identity, e.g. `Developer ID Application: …` |
| `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` | Notarization (`notarytool`) Apple ID + app-specific password. These `APPLE_*` vars are consumed by the Tauri CLI as-is. |
| `WINDOWS_CERTIFICATE` | Base64-encoded Authenticode PFX. **Not** read by `tauri-action` itself — the Windows job decodes it, imports it into `Cert:\CurrentUser\My`, and writes the certificate thumbprint into a merge config so `signtool` can sign. |
| `WINDOWS_CERTIFICATE_PASSWORD` | Password for that PFX (used only by the import step) |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater minisign private key (string contents) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Optional password for that key |

`tauri.conf.json` already has Windows `digestAlgorithm` / `timestampUrl`. The thumbprint is filled in at release time from the imported PFX. Without `WINDOWS_CERTIFICATE`, the Windows installers stay unsigned.

Generate an updater keypair:

```bash
npm run tauri signer generate -- -w ~/.tauri/mqx.key
```

Put the public key in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`. Store the private key as `TAURI_SIGNING_PRIVATE_KEY`. If you rotate the pair, replace the committed pubkey.

When `TAURI_SIGNING_PRIVATE_KEY` is set, the release job enables `createUpdaterArtifacts` and `tauri-action` uploads `latest.json` for the updater endpoint:

`https://github.com/<owner>/<repo>/releases/latest/download/latest.json`

Match `plugins.updater.endpoints` in `tauri.conf.json` to that URL. **Help → Check for Updates** uses the plugin and fails gracefully when the build is unsigned or `latest.json` is missing.
