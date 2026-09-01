# CHANGELOG

## 0.2.0

* RAM slider (4–128 GB, default 12 GB) with oldest-extra eviction; each topic keeps its latest message
* Live / Detached: freeze the topic tree without disconnecting the broker
* Go live applies traffic received while Detached (under the RAM cap)
* Record live traffic to mqtt-trace-compatible JSONL
* Playback tab replays a capture onto the connected broker, including retain flags
* Arrow keys step the selected topic’s message history

## 0.1.0

* Desktop MQTT explorer (Tauri + Svelte)
* Connection profiles with TLS files and keychain passwords
* Live topic tree, payload inspector, jq, and fuzzy search
