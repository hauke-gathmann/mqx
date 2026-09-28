# Releasing mqx

mqx uses GitHub Releases for installation and updates. macOS builds use **ad-hoc code signing**, without an Apple Developer ID, a paid account, or notarization. Updater signatures are independent of Apple signing and are required on every release.

## One-time setup

1. Run `npm ci` and `sh scripts/setup-updater.sh` from the repository root. This generates a private key in `~/.config/mqx-release/updater.key` if absent, writes its public key into the app config, and uploads the private key to the repository's `TAURI_SIGNING_PRIVATE_KEY` Actions secret. It does not print the private key. Inspect the script first; it intentionally changes the app's trusted key and the repository secret.
2. Back up that directory in a secure password manager or encrypted offline backup. Keep the private key out of git, logs, release assets, and chat. Retain the same pair for future updates. Losing it prevents updating already-installed clients with this trust root.
3. Commit the public-key configuration. No `APPLE_*` secrets are needed. The current Windows installers are also unsigned.

For an existing password-protected key, configure `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` separately and use the matching public key. Never run setup to rotate an already-published key without a migration plan. Repository administrators and workflows able to access the private key can sign trusted updates.

## Prepare a candidate

- Update `package.json`, `package-lock.json`'s application version, `src-tauri/Cargo.toml`, `Cargo.lock`'s application version, and `src-tauri/tauri.conf.json`. Add the corresponding `CHANGELOG.md` entry.
- Use the Node version in `.node-version` and Rust in `rust-toolchain.toml`.
- Run `npm ci`, `npm run build`, `npm run check`, `npm run test:release`, `cargo fmt --all -- --check`, `cargo test --locked --workspace`, and `cargo clippy --locked --workspace --all-targets -- -D warnings`.
- Audit with `npm audit` and `cargo audit`. Review upstream unmaintained-dependency warnings separately from vulnerabilities; do not blindly suppress advisories.
- `npm run build` generates `THIRD_PARTY_NOTICES.md` from locked dependency sources and installed npm packages. Tauri bundles it and the project license. Generate it before running desktop Cargo commands in a fresh checkout.
- Inspect installer tests and run shellcheck/actionlint. CI also builds native bundles on Linux, Apple Silicon, and Intel Mac runners, and runs desktop tests on Windows.
- Tag the exact reviewed commit `vX.Y.Z` and push the tag. Prereleases require matching versions such as `0.3.0-rc.1`; the workflow marks those as prereleases. The stable installer/updater deliberately uses only the latest stable release.

The release workflow reruns CI for the tagged commit, checks version consistency and key availability, builds explicit package formats, and uploads to a **draft** release. A final job checks all required installers and update entries, verifies updater signatures against the committed public key, generates `SHA256SUMS`, and attaches installation instructions, the installer, icon, license, and notices. Any missing platform prevents verification from succeeding.

The release stays a draft for review. A green CI check alone does not prove the final release artifact was installed or that broker interactions work.

## Installation acceptance

Target matrix: macOS 13.3+ on Apple Silicon and Intel; Linux x86_64 with a compatible GTK/WebKitGTK runtime (Ubuntu 22.04/24.04 and current Debian/Fedora are candidate test environments). Confirm each version before claiming it as verified. Linux ARM64 is not currently built. Users need no Rust, Node, or build tools to run a release.

On clean user accounts and machines:

- Install, launch, quit, relaunch, and uninstall; confirm retained profiles/settings and user recordings.
- Test Mac installation into both `/Applications` and `~/Applications`, replacement of an old app, failed download/copy recovery, and refusal to replace a running app.
- Check the app's ad-hoc signature with `codesign --verify --deep --strict mqx.app`. Test the actual browser-downloaded/quarantined app too. Ad-hoc signing does not establish an Apple-verified identity: macOS may require **System Settings → Privacy & Security → Open Anyway**, and managed devices may prohibit it. Do not disable Gatekeeper or clear quarantine in the installer.
- Test Linux AppImage with and without FUSE, desktop-menu launch, Wayland and X11, DEB/RPM install/removal/upgrade, and missing-runtime errors. The user installer places only its binaries under the app-data `mqx/app` subdirectory; uninstall preserves sibling user files.
- Connect to a disposable MQTT broker with credentials, TLS and a custom CA; inspect JSON/binary payloads; record, save, and replay a test capture. Retained replay changes broker state, so use a test broker.
- Test the Linux Secret Service backend with a working and a locked/unavailable keyring; verify documented private-file fallback and reopening saved profiles.
- Run sustained traffic and check memory use. The default history budget is 512 MiB, with a 64 MiB minimum; it estimates the topic store, not total process memory.
- Test an actual prior-version → candidate update. Confirm signature mismatch/offline failures preserve the app. Confirm active recording, unsaved recording, and active replay postpone updates. Linux DEB/RPM installations should direct users to package updates rather than overwrite themselves with an AppImage.

Mock installer tests run in CI and verify failure recovery and architecture selection. Mac CI also mounts the actual DMG and tests installation, replacement, signatures and removal in a temporary directory. Linux CI installs and replaces the actual AppImage, requires it to open an X11 window without FUSE, and uninstalls it. These tests supply release downloads locally, so they do not test GitHub delivery or Gatekeeper quarantine. They do not replace the complete application checks above.

Run `python3 scripts/smoke-broker.py` with Mosquitto, its password utility and OpenSSL installed to test the real MQTT core against an isolated TLS broker bound to localhost. It checks custom-CA validation, rejected certificates/passwords, JSON and binary payloads, recording, saving, playback and reconnection. Linux CI runs this automatically. It does not use saved profiles or external brokers.

## Publish

Only after the draft's final verification job and installation acceptance pass, publish the draft through GitHub. Then test the README command, direct assets, `SHA256SUMS`, and `latest.json` as an unauthenticated user. Prerelease assets can be installed with `--version vX.Y.Z-rc.N` after the prerelease is published; they will not become the stable latest endpoint.

Checksums fetched over HTTPS detect corruption and mismatched downloads; they are not independent proof of publisher identity. Updater signatures use the public key embedded in the installed application. The curl bootstrap trusts the GitHub repository and TLS connection.
