# Linux and macOS release readiness

**Historical assessment, followed by implementation on 2026-09-27.** Plans now live in `docs/plans`. The implementation follows the owner’s decision to use a curl installer and ad-hoc Mac signatures without an Apple Developer ID. See [the current release runbook](releasing.md); Apple notarization items below are superseded by that decision. Final verification results are recorded at the end of this document.

Originally reviewed 2026-09-27. **Recommendation: fix the release blockers, then validate a signed release candidate before publishing for general use.** The existing Tauri/Rust/Svelte structure is suitable; this review did not identify a need for a rewrite.

## Scope and evidence

Local checkout: `c97c3e44102414edac4cb77261194e2dd0962586`, application version `0.2.0`. GitHub `main` was three commits ahead at `8da8a90a0926fd501f38039472d2b80ec3b7ad1d`; the comparison showed changes only to playback UI/types and its screenshot, not the build, installer, or release files. Bring the checkout up to date before implementing changes.

GitHub API checks returned no releases, including drafts, and no repository-level Actions secrets. Environment/organization secrets and locally held certificates were not inspected. The latest release-workflow run was [failed](https://github.com/hauke-gathmann/mqx/actions/runs/33526946412), while CI passed. The exact remote validation diagnostic was not retrieved; the invalid secret expressions below are independently confirmed against GitHub's documented rules.

Local validation on macOS, using installed dependencies (Node 26.4.0, Rust 1.93.0):

| Check | Result |
| --- | --- |
| Rust formatting | Passed |
| Core and CLI Clippy, warnings treated as errors | Passed |
| Core tests | 106 passed |
| CLI tests | 14 passed |
| Desktop library tests | 3 passed |
| Desktop `cargo check --locked --offline -p mqx` | Passed |
| Frontend Svelte/TypeScript checks | Passed, one accessibility warning |
| Standard production frontend build | **Failed: missing esbuild** |
| Build with temporary `--minify oxc` override | Passed; output written outside the project |
| Installer shell syntax | Passed |

No production DMG/AppImage/DEB/RPM was built or installed, and no Linux runtime, Gatekeeper, or end-to-end update test was performed. Existing `node_modules` were used; clean `npm ci` remains part of release verification. Application and workflow code were left unchanged.

## P0: release blockers

### 1. Production frontend build fails

`ui/vite.config.ts:33` explicitly selects `esbuild`, but neither the package manifest nor lockfile includes it. `npm run build` fails with `Cannot find package 'esbuild'`. Tauri's configured `beforeBuildCommand` invokes this same build, so packaging is blocked on every platform.

Use Vite 8's default/Oxc minifier, keeping minification disabled for debug builds if desired. The command-line Oxc override already passed locally. Alternatively explicitly install esbuild, but it retains a deprecated configuration. See the [Vite migration guide](https://vite.dev/guide/migration#javascript-minification-by-oxc).

**Acceptance:** clean dependency installation, frontend checks, production frontend build, and native bundle builds pass.

### 2. Release workflow contains invalid secret conditions

`.github/workflows/release.yml:72` and `:77` reference `secrets.*` directly inside step `if:` expressions. GitHub does not permit this. Put the relevant secrets in job-level environment variables and test `env.*`, or check their presence inside a script. The Windows condition must also be fixed even if the intended release focuses on Mac/Linux, because it is part of the same workflow definition. See [GitHub's secret usage rules](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/use-secrets).

**Acceptance:** validate the workflow with actionlint, then successfully run all required matrix jobs and produce a complete draft release.

### 3. The advertised installation path has no release to download

`install.sh` depends on GitHub's latest published release; the updater depends on its `latest.json`. No release currently exists. Even a successful build creates a draft, which will not make the public installation route work until it is published.

**Acceptance:** after verification, publish a complete stable release containing the advertised installers, update metadata/signatures, checksums, and useful release notes. Test the README instructions from a clean user account.

## P1: prepare a dependable first public release

### macOS distribution

- Keep native Apple Silicon and Intel DMGs; the existing matrix already describes both. Explicitly choose and document the minimum supported macOS version, then test it. Cross-compiling the Intel bundle is not an Intel launch test.
- Configure a Developer ID Application certificate and Apple notarization credentials. The workflow already forwards the relevant `APPLE_*` variables. Make missing signing material a failure for stable releases; keep unsigned development builds separate. See [Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/).
- Verify signing, notarization, and stapling on the final artifacts. Test a browser-downloaded DMG with quarantine intact, offline first launch, installation into Applications, and launching on each supported architecture.
- Remove `xattr -cr` from `install.sh:135`. It clears extended attributes, including quarantine when present, and does not establish publisher trust. The normal installation experience should work through signing and notarization.
- Replace the delete-before-copy flow at `install.sh:129` with a staged replacement and recovery path. Currently a failed copy can remove the working installation. Support a user-owned `~/Applications` destination when system-wide installation is unavailable, and handle an already-running app.
- Make a direct DMG download and drag-to-Applications the primary graphical path. A personal Homebrew tap/cask can provide convenient installation and upgrades later; it is not required for the first release. See the [Homebrew Cask guide](https://docs.brew.sh/Cask-Cookbook).

### Linux distribution

- Start with an explicit x86_64 support matrix. Linux ARM64 is neither built nor accepted by the installer; either add native build/test coverage or clearly retain that limitation.
- Offer tested DEB packages for selected Debian/Ubuntu versions and RPM packages for selected Fedora-family versions, alongside the portable AppImage. `bundle.targets = "all"` requests broad packaging, but does not prove these artifacts build or run. Make desired formats explicit in release configuration. Tauri supports [DEB](https://v2.tauri.app/distribute/debian/) and [RPM](https://v2.tauri.app/distribute/rpm/) packaging.
- Build against an appropriate older supported Linux baseline and test each promised distribution. Check runtime WebKitGTK/GTK dependencies, package metadata, installation, removal, and upgrade behavior. Users of binaries should not need Node, Rust, or development headers.
- The current installer only places an AppImage at `~/.local/bin/mqx`. Add a desktop launcher and icon so users can find it in their application menu. Honor user/XDG paths, offer an uninstall path, and avoid requiring PATH edits merely to open a desktop application.
- Detect or clearly diagnose AppImage FUSE requirements, with a documented extraction alternative where appropriate. Test no-FUSE environments, Wayland and X11, and systems without development packages. See [AppImage FUSE troubleshooting](https://docs.appimage.org/user-guide/troubleshooting/fuse.html).
- Choose update behavior per package format: use the existing Tauri route for AppImage; use package installation/repositories for DEB/RPM. Do not present an AppImage self-update as an update to a package-managed installation. Flatpak/Flathub can follow once its sandbox permissions and release maintenance are planned.

### Installation integrity and update reliability

- Generate release checksums and verify downloads before installation. For publisher authenticity, use artifact signatures or attestations with a defined trust mechanism; a checksum downloaded beside a binary only provides an integrity comparison. The current installer verifies neither checksums nor signatures.
- Verify possession of the private updater key matching the public key committed in `src-tauri/tauri.conf.json:44`. Securely back it up before shipping. If it cannot be recovered, establish the keypair before the first public release. Do not casually rotate a key after clients have shipped.
- Require updater signing and validate `latest.json` for both Mac architectures and Linux x86_64. Apple signing and updater signing serve different purposes. Test an actual old-to-new update, signature failure, missing metadata, offline operation, write permissions, and profile retention. See the [Tauri updater documentation](https://v2.tauri.app/plugin/updater/).
- Review restart safety: `src-tauri/src/updater.rs:52` restarts directly without checking the application's unsaved-recording state. Add a shared shutdown/update guard and verify recording flush/save and active replay behavior before allowing a restart. This is a code-review concern, not a reproduced data-loss incident.
- Distinguish download errors, API limits, and missing releases in the installer. It currently labels any release API failure as “no published GitHub Release yet.” Add a version-pinned installation option for reproducibility and rollback.

### CI and release controls

- Add production frontend builds to ordinary CI. Currently CI type-checks the frontend but misses the reproduced bundling failure.
- Validate workflows and installer syntax/behavior. Add installer tests using mocked releases for architecture selection, failed downloads, failed replacements, and unsupported platforms.
- Add native packaging checks on macOS and Linux, including the desktop library tests that CI currently omits. Retain the existing core/CLI checks.
- Make release packaging depend on successful quality checks for the exact tagged commit. The current independent CI and release workflows do not enforce that relationship.
- Pin the chosen Rust/Node versions and release runner images; retain lockfiles, use `npm ci` and locked Cargo builds, and update pins deliberately. Consider pinning third-party Actions to reviewed commits.
- Check tag version against `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`. They currently agree at `0.2.0`, but `v*` accepts arbitrary tags. Internal unpublished crates can have independent versions.
- Handle prerelease tags explicitly: `prerelease: false` currently applies even to a future `v0.3.0-rc.1`. Keep candidates out of the stable latest/update channel.
- Verify all expected artifacts and updater platform entries together before publication. Replace the generic draft body with actual changes, requirements, installation steps, and known limitations.

## Product checks worth completing before wider adoption

- **Linux password storage:** `keyring_available()` in `crates/mqx-core/src/profiles.rs:373` excludes Linux, and Cargo enables no Linux keyring backend. Linux therefore always uses plaintext secret files protected by directory/file permissions, even on desktops with a secret service. Add native Linux credential storage or describe the actual behavior clearly; the README's general fallback wording obscures this distinction.
- **Memory defaults:** `crates/mqx-core/src/config.rs:14` defaults to a 12 GiB topic-store budget and permits no value below 4 GiB. This is not preallocated memory, but on busy brokers it can exceed the resources of an ordinary laptop. Prefer a conservative or system-aware default and allow smaller budgets. The accounting is an estimate and excludes other process memory; test long-running traffic and large recordings.
- **Webview hardening:** `src-tauri/tauri.conf.json:23` disables CSP. Introduce and test a restrictive policy compatible with Tauri IPC and CodeMirror's styles. No HTML-injection exploit was established by this review. See [Tauri CSP guidance](https://v2.tauri.app/security/csp/).
- **Diagnostics:** core code emits tracing events and defines a log path, but the desktop app does not initialize a tracing subscriber or logging plugin. Provide rotating diagnostic logs and an easy way to find them; exclude credentials and sensitive payloads.
- **Polish/documentation:** resolve the context-menu accessibility warning in `PayloadView.svelte:314`; document data locations, backups, uninstall behavior, supported OS/architectures, and troubleshooting. Include application licensing and dependency notices in distributed artifacts, and run dependency/advisory checks before release; this review did not perform a dependency vulnerability audit.

## Release acceptance checklist

- [ ] Update the working checkout, fix both build/workflow blockers, and pass clean CI.
- [ ] Decide supported OS versions, architectures, formats, and application identity/metadata.
- [ ] Configure Apple and updater signing; verify the committed updater public key matches.
- [ ] Improve installer integrity, Mac replacement behavior, and Linux desktop integration.
- [ ] Produce a draft candidate with all required artifacts and checksums.
- [ ] On clean Mac and Linux systems: download, install, launch, create a profile, save credentials, connect with TLS, reconnect, inspect messages, record, replay against a test broker, quit, and relaunch.
- [ ] Verify upgrades preserve profiles/settings and protect recordings; test offline and failed updates, package-managed upgrades, and uninstall behavior.
- [ ] Verify browser-downloaded Mac artifacts pass Gatekeeper with quarantine intact, on Apple Silicon and Intel.
- [ ] Publish the verified stable release and check every public installation/download/update link.

The code/build/installer preparation can be implemented locally. Apple account/certificate access, durable signing-key custody, and access to the promised OS/architecture test environments must also be arranged.

## Implemented preparation — 2026-09-27

The assessment above is retained as the original review, not the current task list. The owner chose distribution without an Apple Developer ID. The implementation now provides:

- Oxc production builds, pinned toolchains, locked dependencies, full desktop/core/CLI checks, and native packaging CI on both Mac architectures and Linux.
- A curl installer with checked downloads, Mac staged replacement and recovery, Linux desktop integration and no-FUSE fallback, version selection, and data-preserving uninstall. The DMG includes the MIT license as a resource without an interactive license-acceptance prompt.
- An approved updater key stored outside the repository and uploaded to the GitHub Actions secret, with only the public key committed. Release drafts require matching versions, complete artifacts, verified update signatures, checksums, and release notes before manual publication.
- Recording/replay guards for updates and native Quit protection for unsaved recordings; package-managed Linux installs receive package-update guidance.
- Linux Secret Service support, a 512 MiB default history budget, a restrictive webview policy, private rotating diagnostics, bundled dependency notices, and installation/maintenance documentation.
- Moved design plans under `docs/plans`.

Local verification on Apple Silicon: clean npm install and production build; frontend checks with no warnings; all 124 Rust tests; workspace Clippy with warnings denied; 18 installer/release tests; shellcheck and actionlint. Built the actual DMG and updater archive, verified ad-hoc app signing and the updater signature, and installed, replaced, and uninstalled the DMG through the real installer in a temporary Applications directory. Only release downloads were supplied from local fixtures during that installation test.

Dependency updates eliminated the directly applicable audited vulnerabilities. Four advisories against an unused legacy certificate-validation dependency have narrowly checked exceptions; informational upstream maintenance/unsoundness warnings remain documented in [dependency-review.md](dependency-review.md). The audit is not a claim that all dependencies are free of risk.

Cross-platform CI and application acceptance results are recorded as they become available. Publication, clean-machine Gatekeeper/desktop checks, and a real previous-version update remain release acceptance steps. Back up the updater key securely before the first public release.
