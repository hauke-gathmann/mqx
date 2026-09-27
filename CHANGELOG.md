# CHANGELOG

## 0.2.0

* Verified curl installation, recoverable Mac replacement, and Linux desktop integration
* Ad-hoc macOS signing without an Apple Developer ID; separately signed in-app updates
* Release validation, explicit native packages, checksums, and bundled dependency notices
* Linux Secret Service credential storage, private diagnostic logs, and webview CSP
* Updates postpone installation while recordings or playback are active
* Default history budget reduced to 512 MiB; minimum reduced to 64 MiB

* RAM slider (64 MiB–128 GiB, default 512 MiB) with oldest-extra eviction; each topic keeps its latest message
* Live / Detached: freeze the topic tree view without disconnecting the broker
* Go live applies traffic received while Detached (under the RAM cap)
* Check for Updates in Settings
* Record live traffic to mqtt-trace-compatible JSONL
* Playback tab replays a capture onto the connected broker, including retain flags
* Arrow keys step the selected topic’s message history

## 0.1.0

* Desktop MQTT explorer (Tauri + Svelte)
* Connection profiles with TLS files and keychain passwords
* Live topic tree, payload inspector, jq, and fuzzy search
