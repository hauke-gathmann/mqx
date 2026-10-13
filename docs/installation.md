# Installation and troubleshooting

Use the [release downloads](https://github.com/hauke-gathmann/mqx/releases) or the curl command in the README. Binary releases need no development tools. The first public download is available only after a verified release is published.

## Choose a version or installation directory

Download the script for review before executing it:

```sh
curl -fsSL https://raw.githubusercontent.com/hauke-gathmann/mqx/main/install.sh -o install-mqx.sh
sh install-mqx.sh --version v0.2.0
```

`--version` accepts a published stable or prerelease tag. Omit it to use the latest stable release. For fully version-pinned installation, download the script from the same version tag instead of `main`.

macOS defaults to `/Applications` and falls back to `~/Applications` if it cannot write there. Set `MQX_APP_DIR="$HOME/Applications" sh install-mqx.sh` to choose explicitly. Quit mqx before replacing it. The new app is copied and checked before the previous app is moved; failed replacement restores the previous installation.

Linux uses `${XDG_DATA_HOME:-$HOME/.local/share}/mqx/app/mqx.AppImage` and a launcher at `${MQX_BIN:-$HOME/.local/bin}/mqx`. It also installs a desktop entry and icon under XDG data directories. Use the application menu even if `~/.local/bin` is not on PATH. Installation directories must be absolute and cannot contain newlines, quotes, backticks, dollar signs, percent signs, or backslashes.

## macOS trust

The app targets macOS 13.3 or later on Intel and Apple Silicon. It is ad-hoc signed, not notarized or signed by an Apple Developer ID. Checksums and a valid ad-hoc signature do not create an Apple-verified publisher identity. If macOS blocks launch, follow its **System Settings → Privacy & Security → Open Anyway** flow after reviewing the source of the download. Organization policy may prevent this. See [Apple's guidance](https://support.apple.com/en-us/102445).

The installer deliberately does not clear quarantine attributes or disable Gatekeeper. For managed distribution that requires notarization, a Developer ID release would be needed.

## Linux

The initial architecture is x86_64. DEB/RPM files install through the system package manager; AppImages are installed per user. Use your distribution's package manager to install local DEB/RPM files so runtime dependencies are resolved. Do not use development package instructions intended for building mqx.

AppImage compatibility still depends on the system's graphics libraries and supported glibc baseline. A build on Ubuntu 22.04 is intended to retain compatibility with that baseline; release testing determines the supported distributions.

If FUSE is absent or disallowed, use:

```sh
APPIMAGE_EXTRACT_AND_RUN=1 ~/.local/bin/mqx
```

The launcher automatically selects extraction when `/dev/fuse` is inaccessible. If a FUSE library is missing even though `/dev/fuse` exists, use the explicit command above. Extraction needs writable temporary disk space. Alternatively use a DEB/RPM package. See [AppImage's FUSE guidance](https://docs.appimage.org/user-guide/troubleshooting/fuse.html).

DEB/RPM users install new packages to update. AppImage and macOS installations support **Help → Check for Updates**. Stop playback and save/discard recordings before updating.

## Data, credentials, and logs

| Data | macOS default | Linux default |
| --- | --- | --- |
| Profiles and settings | `~/Library/Application Support/mqx/` | `${XDG_CONFIG_HOME:-~/.config}/mqx/` |
| Recordings | `~/Library/Application Support/mqx/recordings/` | `${XDG_DATA_HOME:-~/.local/share}/mqx/recordings/` |
| Diagnostic logs | `~/Library/Caches/mqx/logs/` | `${XDG_CACHE_HOME:-~/.cache}/mqx/logs/` |

The recordings directory can be changed in Settings. Back up configuration and recordings while the app is closed. Keychain credentials are separate and will generally need re-entry on another machine. TLS profiles reference certificate/key files; preserve those files and paths separately.

Linux uses a running Secret Service, such as GNOME Keyring or a compatible desktop service. If keychain operations fail, mqx can fall back to plaintext files under the configuration `secrets/` directory, protected with directory mode 0700 and file mode 0600. Treat configuration backups as sensitive. Re-save a password after restoring keychain access to store it there.

Diagnostic logs rotate daily and retain up to five files. They exclude MQTT payloads and jq expressions. Review logs before sharing because errors can include local paths and connection details.

## Uninstall

For installations made by the script, use the same directory overrides as installation:

```sh
sh install-mqx.sh --uninstall
```

This removes the installed application, launcher and icon, and preserves profiles, settings, passwords, and recordings. For a Mac installation in a custom directory, set `MQX_APP_DIR` accordingly. DEB/RPM users uninstall with their package manager. You can remove remaining user data separately after backing it up.
