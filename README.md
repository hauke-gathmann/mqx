![mqx icon](https://raw.githubusercontent.com/hauke-gathmann/mqx/main/src-tauri/icons/128x128.png)

# mqx

mqx is a desktop MQTT explorer. Connect to a broker, browse its topic tree, inspect message history and payloads, and record or replay traffic. Each window connects to one broker. Connection profiles and TLS file paths are saved locally.

## Explorer

The topic tree fills in as publishes arrive. Search filters paths; pick a topic to scroll its history and inspect the payload. JSON pretty-prints in place, with jq on the same pane.

**Live / Detached.** Detach freezes the **view**; Go live applies traffic received in the meantime, still under the RAM cap. Recording continues while Detached.

![Explorer — topic tree, history, and payload inspector](https://raw.githubusercontent.com/hauke-gathmann/mqx/main/docs/images/explorer.png)

## Playback

While connected, Record writes mqtt-trace JSONL (raw payloads). Playback publishes a capture onto the broker, including retain flags — that can change retained state on the target. Stay Live to watch the tree, or Detach to freeze the view; replay itself does not flip Live / Detached.

Playback shows each trace's duration from its first to last message. Right-click a trace to delete it. Use **Open recordings folder** to share a JSONL file or copy a capture into the folder, then return to mqx or select **Refresh** to see it.

![Playback — replaying a capture onto the connected broker](https://raw.githubusercontent.com/hauke-gathmann/mqx/main/docs/images/playback.png)

## Configuration

Open **Settings** with `Cmd/Ctrl+,` to choose a theme, set the topic history budget, and change the recordings folder.

**Topic history limit.** The RAM slider ranges from 64 MiB to 128 GiB, with a default of 512 MiB. Older extra messages drop first; each topic keeps at least its latest payload. This limit covers the in-memory topic store, not the whole process.

Profiles and settings are stored under the `mqx` application directory. Theme is stored in `config.toml` (`[ui] theme = "dark" | "light" | "system"`). See [data locations and backups](docs/installation.md#data-credentials-and-logs) for platform-specific paths.

## Install

On **macOS 13.3 or newer** (Apple Silicon or Intel) and **Linux x86_64**, run:

```sh
curl -fsSL https://raw.githubusercontent.com/hauke-gathmann/mqx/main/install.sh | sh
```

The installer downloads the latest stable release and verifies its SHA-256 checksum. On macOS it installs to `/Applications`, or `~/Applications` if necessary. On Linux it installs an AppImage and adds an application-menu entry. Rust and Node are not required.

**Opening on macOS:** Open mqx from Applications. The app is ad-hoc signed and is not notarized by Apple. If macOS blocks it, review the warning under **System Settings → Privacy & Security → Open Anyway**. Managed Macs may prohibit opening it.

Direct DMG, AppImage, DEB and RPM downloads are available on the [releases page](https://github.com/hauke-gathmann/mqx/releases). Linux needs GTK 3 and WebKitGTK 4.1; Linux ARM64 is not currently built. See the [installation guide](docs/installation.md) for runtime requirements, FUSE, custom locations, specific versions, backups and removal.

**Windows:** download the setup EXE or MSI from the releases page. These installers are unsigned and may trigger SmartScreen.

Passwords use the macOS or Windows keychain or Linux Secret Service when available. If the keychain is unavailable, mqx falls back to a permission-protected local file. Profiles, settings and recordings are preserved when the app is uninstalled.

## Develop

Requires [Rust stable](https://rustup.rs/) and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

```bash
npm ci
npm run tauri dev
```

The development command generates the bundled dependency notices before compiling the desktop app.

Layout:

- `ui/` — Vite + Svelte 5 frontend
- `src-tauri/` — Tauri 2 shell and IPC (`mqx` package)
- `crates/mqx-core/` — UI-agnostic profiles, tree, and decode
- `tools/mqtt-trace/` — record/replay MQTT traffic for testing

```bash
npm run check
cargo test -p mqx-core
cargo check -p mqx
```

### MQTT traces

`mqtt-trace` records every publish on a broker to a JSONL file, then replays it with the original inter-arrival times:

```bash
cargo run -p mqtt-trace -- record --broker mqtt://127.0.0.1:1883 -o traffic.jsonl
cargo run -p mqtt-trace -- replay traffic.jsonl --broker mqtt://127.0.0.1:1883
```

Subscribe defaults to `#`. Add `--sys` to include `$SYS/#`. Replay `--speed 1` is real time; `--speed 0` is as fast as possible; `--loop` repeats the trace.

CLI replay of an in-app recording is the same thing as the Playback tab: a traffic dump onto the broker, not a private in-memory view. In-app files start with a header line; `mqtt-trace replay` skips it. Retained publishes in the file are sent as retained.

Native menus: **mqx**, **Connections**, **Edit**, **View**, **Help**. `Cmd/Ctrl+,` opens settings. `Cmd/Ctrl+K` focuses topic search. `Cmd/Ctrl+Shift+R` toggles recording. Connections has Detach / Go Live / Start/Stop Recording. View switches Explorer and Playback. ↑/↓ steps message history.

## Releases

See the [release runbook](docs/releasing.md) for signing-key setup, release checks, supported-platform verification, and publishing. No Apple Developer ID is required. Release builds use ad-hoc macOS signatures and a separate free key for verified in-app updates.

Every version tag runs the quality checks before packaging and creates a draft. The final verification job checks artifact completeness and updater signatures, and uploads checksums. Publish only after installation tests pass.

Historical design plans are in [docs/plans](docs/plans/). The [readiness assessment](docs/release-readiness.md) records the original review and implementation status.
