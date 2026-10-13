# mqtt-ui: Desktop GUI (replace TUI)

Turn this repo from a Ratatui MQTT explorer into a downloadable desktop app for the same users as [MQTT Explorer](https://mqtt-explorer.com/): IoT/home-automation people who want a structured view of a broker, not a CLI.

**Product name:** `mqtt-ui`  
**v1 scope (locked):** one broker per window, hierarchical topic tree, saved connection profiles including TLS client certs, GitHub Release installers for Windows / macOS / Linux.  
**Dropped from the current TUI (do not port):** CBOR, pluggable/custom protocol decoders, the clap CLI, the Ratatui UI.

Existing mqttui strengths that *do* ship in v1: JSON pretty-print, jq, fuzzy search, fresh/stale/retain coloring, per-topic in-memory history.

**v2 is not built in this plan.** The architecture below is shaped so those features plug in without a rewrite. The current V2 design (RAM budget, detach, record, broker replay — publish UI is V3) is [docs/plans/planV2.md](planV2.md).

---

## Why this stack

| Layer | Choice | Why |
|---|---|---|
| App shell | **Tauri 2** | Native WebView, small download, first-class Rust backend, official bundler for `.dmg` / `.msi` / `.exe` / `.AppImage` / `.deb`, built-in updater. MQTT Explorer is Electron; Tauri is the same UX class at a fraction of the size. |
| Backend | **Rust workspace crate `mqttui-core` + `rumqttc`** | Keep the current MQTT/jq code. `rumqttc` 0.24 already in tree; enable `use-rustls` + `websocket` for mqtt / mqtts / ws / wss and mTLS. |
| Frontend | **Svelte 5 + TypeScript + Vite** | Fine-grained reactivity fits 10–20 Hz tree patches better than React. Small bundle. Enough ecosystem (CodeMirror 6, TanStack Virtual, uPlot later). |
| Styling | **CSS variables + a small design token file** | Dark-first (MQTT Explorer users live in dark mode), light theme from day one. No heavy component library; this UI is a tree + inspector, not a dashboard kit. |
| JSON view | **CodeMirror 6** (`@codemirror/lang-json`) | Highlight + fold + copy. Monaco is too heavy. |
| Packaging | **`tauri-apps/tauri-action`** on tag `v*` | One GitHub Release with signed/notarized artifacts + auto-update `latest.json`. |

**Rejected:** egui/iced (packaging, notarization, and “real app” chrome are worse; JSON editor / tree / forms look like a debug tool). Electron (what we are replacing, size and update pain). Dioxus/Leptos (weaker tree/editor ecosystem). React (fine, but more re-render work for live trees).

---

## Repo shape after the cut

Cargo workspace. The TUI, clap CLI, ratatui, crossterm, syntect-tui, clipboard-crate, ciborium, custom-protocol spawn, and XDG-only paths go away.

```
mqtt-ui/
  Cargo.toml                 # workspace
  crates/mqttui-core/        # UI-agnostic: broker session, tree, store, json decode, jq
  src-tauri/                 # Tauri 2: commands, events, window, updater
  ui/                        # Vite + Svelte 5
  .github/workflows/ci.yml
  .github/workflows/release.yml
```

**Keep and move into `mqttui-core`:** JSON interpretation from `message.rs` (strip `Format::Cbor` / `Format::Custom` / `execute()`), `jq.rs` (`Jaqqer` logic without TUI cursor), freshness / buffer-size policy from `model.rs` + `config.rs`.

**Delete:** `src/ui.rs`, `src/crumbs.rs`, `src/highlight.rs`, `src/events.rs` (crossterm keymap), clap `Args` in `main.rs`, CBOR (`ciborium`) and `ProtocolConfig` / `[[protocols]]`. Highlighting moves to CodeMirror. History crumbs become a Svelte control.

**Rewrite:** config. Today it is `~/.config/mqttui/config.toml` via the `xdg` crate (Linux-centric, no connection profiles). Replace with `directories` crate:

- macOS: `~/Library/Application Support/mqtt-ui/`
- Windows: `%APPDATA%\mqtt-ui\`
- Linux: `~/.config/mqtt-ui/`

---

## Architecture

One live session per window. Switching profile disconnects, drops the in-memory tree, connects the new profile. Multiple OS windows later can each own a session; v1 does not need an in-app tab strip.

Ingress is **not** hardcoded to rumqttc. `TopicTree::upsert` is the only write path. Live MQTT is one `Source`; a recording file will be another in v2. That is the main architectural concession to upcoming features.

```
┌──────────────────────────────────────────────────────────┐
│  ui/ (Svelte)                                            │
│  ConnectionPicker | Explorer (tree + inspector + jq)     │
└────────────▲─────────────────────────┬───────────────────┘
             │ events (push)           │ commands (req/res)
             │  session/status         │  connect / disconnect
             │  tree/batch             │  tree_children / tree_search
             │  topic/message          │  get_message / apply_jq
             │  session/stats          │  profiles CRUD
┌────────────┴─────────────────────────▼───────────────────┐
│  src-tauri  (thin)                                        │
│  Tauri commands + coalesced event emitter                 │
└────────────▲─────────────────────────┬───────────────────┘
             │                         │
┌────────────┴─────────────────────────▼───────────────────┐
│  mqttui-core                                              │
│  ProfileStore │ Session │ TopicTree │ JsonDecode │ Jq     │
│                    │                                      │
│                    ▼                                      │
│              Source: rumqttc EventLoop   (v1)             │
│              Source: recording reader    (v2, not built)  │
│              decode: JSON on blocking pool                │
└──────────────────────────────────────────────────────────┘
```

### Session

```rust
pub struct Session {
    pub id: ProfileId,
    pub status: Status,          // Connecting | Connected | Reconnecting { since } | Disconnected | Error { msg }
    client: rumqttc::AsyncClient, // present for live sources; unused for playback
    tree: TopicTree,
    selected: Option<String>,    // full topic; inspector follows this
    subscriptions: Vec<Sub>,
    source: Source,              // Live { .. }  |  (v2) Playback { path, cursor }
}

/// Canonical inbound event. Live MQTT and (later) recordings both produce this.
pub struct Inbound {
    pub topic: String,
    pub payload: Bytes,          // raw, lossless — required for v2 recording
    pub retain: bool,
    pub qos: QoS,
    pub timestamp: SystemTime,   // broker-arrival in live mode; recorded stamp in playback
}
```

Keep **raw payload bytes** on every stored `Message`, even though v1 only displays JSON/text. Recordings must be lossless; re-deriving bytes from pretty-printed JSON is not acceptable later.

MQTT event loop task (v1 live source):

1. `poll()` rumqttc.
2. On `ConnAck`, subscribe profile filters (default `#` QoS 0), emit `session/status`.
3. On `Publish`, map to `Inbound` (keep raw bytes, stamp `now`) and send to a **decode worker**. Do not JSON-parse on the event-loop task.
4. Worker returns `Message` → `TopicTree::upsert`. Empty payload + `retain=true` **deletes** the leaf (MQTT retained-clear).
5. Coalesce tree mutations for 50 ms (20 Hz cap) then emit `tree/batch`.
6. If the topic is the current selection, emit `topic/message` immediately (no coalesce).
7. IO errors → `Disconnected` / backoff reconnect (keep current mqttui behaviour, minus `--quit`).

`AsyncClient` stays on `Session` in v1 even though nothing publishes yet. v2 `publish` is a command on that client plus an append to a per-profile publish log — not a new connection type.

### Topic tree (the v1 feature)

mqttui today is a **flat `BTreeMap<String, Messages>`**. MQTT Explorer’s differentiator is a path-segment tree. That becomes the source of truth:

```rust
pub struct TopicTree {
    root: Node,
    /// topic → segments, for O(1) leaf updates
    index: HashMap<String, Arc<str>>,
}

pub struct Node {
    pub segment: String,
    pub children: BTreeMap<String, Node>,
    pub leaf: Option<Leaf>,     // Some if a message was published on this exact path
}

pub struct Leaf {
    pub latest: Message,
    pub history: VecDeque<Message>, // cap = app buffer_size (0 = unbounded, same as today)
    pub received: u64,
}

pub struct Message {
    pub inbound: Inbound,       // raw bytes + meta
    pub data: Result<Value, String>,
    pub format: Format,         // Json | Text | Binary  — no Cbor, no Custom
    pub text: String,           // pretty JSON, UTF-8, or "<binary>"
}
```

`home/living/lamp` → `home` → `living` → `lamp`. Intermediate nodes exist only as structure; they get `leaf` only if something published on that exact topic.

Frontend does **not** hold the full tree. Rust does. The UI asks for children of expanded nodes and applies patches. This is how we stay usable on brokers with tens of thousands of topics (MQTT Explorer’s own performance claim).

In-memory per-topic `history` is **not** a recording. It is a ring buffer for the inspector. v2 recordings are a session-level, append-only log of every `Inbound` (see Upcoming).

---

## IPC contract (v1)

All payloads are JSON. Commands are camelCase on the wire. Binary payloads never cross IPC as raw bytes; the core produces `payloadText` / parsed JSON / error. Raw bytes stay in Rust for a future recorder.

### Connection profile (on disk + `saveProfile`)

Stored as `connections.json`. Passwords and key-file passphrases go in the OS keychain via `keyring` (service `mqtt-ui`, account = profile id). MQTT Explorer keeps secrets in a local JSON file; keychain is the “better” part.

```json
{
  "id": "4f2c…",
  "name": "Home Assistant",
  "protocol": "mqtts",
  "host": "ha.local",
  "port": 8883,
  "clientId": "mqtt-ui-quiet-otter",
  "username": "hauke",
  "tls": {
    "validate": true,
    "caCertPath": "/Users/hauke/certs/ca.pem",
    "clientCertPath": "/Users/hauke/certs/client.crt",
    "clientKeyPath": "/Users/hauke/certs/client.key",
    "alpn": null
  },
  "session": {
    "clean": false,
    "keepAliveSecs": 60,
    "maxPacketSize": 10000000
  },
  "subscriptions": [{ "topic": "#", "qos": 0 }],
  "lastWill": null
}
```

`protocol`: `mqtt` | `mqtts` | `ws` | `wss`.  
`ws`/`wss` are in v1 because rumqttc already has the feature and cloud brokers often only expose WSS — cheap compared to a second connection-form later.

TLS mapping to rumqttc:

```rust
Transport::Tls(TlsConfiguration::Simple {
    ca: read(ca_path)?,                    // empty → webpki roots
    alpn: tls.alpn.clone(),
    client_auth: match (cert, key) {
        (Some(c), Some(k)) => Some((c, k)),
        _ => None,
    },
})
```

`validate: false` → custom rustls `ClientConfig` with `dangerous().with_custom_certificate_verifier` (needed for homelab self-signed brokers; MQTT Explorer has “reject unauthorized”). Gate it behind a visible warning on the form.

Do **not** add `recordOnConnect` or publish-defaults to this schema in v1. Leave the JSON extensible (unknown fields ignored via serde `deny_unknown_fields` **off**).

### Commands

| Command | Args | Result |
|---|---|---|
| `listProfiles` | — | `ProfileSummary[]` (`id`, `name`, `protocol`, `host`, `port`) |
| `getProfile` | `{ id }` | `ConnectionProfile` (password omitted; `hasPassword: bool`) |
| `saveProfile` | `ConnectionProfile & { password?: string }` | `{ id }` |
| `deleteProfile` | `{ id }` | `ok` |
| `connect` | `{ id }` | `{ status: "connecting" }` (further updates via event) |
| `disconnect` | — | `{ status: "disconnected" }` |
| `treeChildren` | `{ path: string[] }` | `TreeNodeDto[]` |
| `treeSearch` | `{ query: string, mode: "keep" \| "skip" }` | `SearchHit[]` `{ path, highlights: number[] }` |
| `selectTopic` | `{ topic: string \| null }` | `ok` — starts/stops `topic/message` stream |
| `getMessage` | `{ topic, index: number \| null }` | `MessageDto` (`null` index = latest) |
| `getHistoryMeta` | `{ topic }` | `{ count, latestIndex }` |
| `applyJq` | `{ topic, index, filter }` | `{ text, json?, errors: JqError[] }` |
| `jqHistory` | `{ topic }` | `string[]` — same on-disk history as today, grouped by topic suffix |
| `pickFile` | `{ kind: "ca" \| "cert" \| "key" }` | `{ path }` — native file dialog |

v2 will add `publish`, `listPublishHistory`, `startRecording`, `stopRecording`, `openRecording` on the same command bus. Do not implement stubs; just don't name-collide and keep `Session` the owner of the client + tree.

### Events

```ts
type SessionStatus = {
  profileId: string | null
  status: "connecting" | "connected" | "reconnecting" | "disconnected" | "error"
  error?: string
  broker: string            // display URL, password stripped
}

type TreeBatch = {
  upserts: TreeNodeDto[]    // changed nodes only (the node, not its descendants)
  deletes: string[]         // full topic paths
}

type TreeNodeDto = {
  segment: string
  path: string              // "home/living/lamp"
  childCount: number
  hasPayload: boolean
  retain: boolean
  freshness: "fresh" | "intime" | "stale" | "retain"
  format: "json" | "text" | "binary"
  lastMs: number
  historyLen: number
}

type MessageDto = {
  topic: string
  payloadText: string
  payloadJson?: unknown
  format: TreeNodeDto["format"]
  retain: boolean
  qos: 0 | 1 | 2
  timestamp: number
  size: number
  error?: string
}
```

`session/stats` every 1 s: `{ topics, messagesTotal, messagesPerSec }` for the header.

### Frontend tree state

```ts
type TreeState = {
  nodes: Map<string, TreeNodeDto>  // path → dto
  expanded: Set<string>
  children: Map<string, string[]>  // parent path → child paths, insertion order from BTreeMap
  selected: string | null
}
```

Flatten `expanded` into a virtualized row list (`@tanstack/svelte-virtual`). On `tree/batch`, patch `nodes`; if a delete hits an expanded path, drop descendants. On first expand of a path, `treeChildren` once; later patches keep it current.

Search: debounce 50 ms → `treeSearch` → expand ancestors of hits → highlight matching graphemes in the row label (same skim fuzzy matcher as today). Skip-mode (`?` in the TUI) is a second input or a toggle on the same field.

---

## UI drafts

Dark-first, compact, MQTT Explorer’s three-zone layout with mqttui’s freshness language.

v1 explorer is **tree | inspector**. Do not fill the bottom of the window with chrome. v2’s publish pane docks under the inspector; leave the layout as a vertical split that can gain a third row without rearranging the tree.

### 1. Connection picker (startup / after disconnect)

```
┌─ mqtt-ui ──────────────────────────────────────────────┐
│  Connections                              [ + New ]    │
│                                                        │
│  ┌──────────────────────┐  ┌──────────────────────┐    │
│  │ Home Assistant    ⋯  │  │ Mosquitto local   ⋯  │    │
│  │ mqtts://ha.local:8883│  │ mqtt://localhost:1883│    │
│  │                      │  │                      │    │
│  │            [Connect] │  │            [Connect] │    │
│  └──────────────────────┘  └──────────────────────┘    │
│                                                        │
│  Last used · Home Assistant                            │
└────────────────────────────────────────────────────────┘
```

`⋯` → Edit / Duplicate / Delete. First run shows a single empty card prefilled with `mqtt://localhost:1883` (today’s CLI default).

### 2. Profile editor (modal)

```
Name            [ Home Assistant              ]
Protocol        ( mqtt  mqtts  ws  wss )
Host / Port     [ ha.local ]  [ 8883 ]
Client ID       [ mqtt-ui-quiet-otter     ]  (↻)
Username        [ hauke ]
Password        [ •••••• ]  (stored in keychain)

── TLS ─────────────────────────────────────
☑ Validate certificate
CA              [ ~/certs/ca.pem          ] [Browse]
Client cert     [ ~/certs/client.crt      ] [Browse]
Client key      [ ~/certs/client.key      ] [Browse]

── Session ─────────────────────────────────
☐ Clean session     Keepalive [ 60 ] s
Subscriptions       [ #            ] QoS (0)

                    [ Cancel ]  [ Save ]  [ Save & Connect ]
```

Advanced (collapsed): last will, extra subscription rows, max packet size, `validate` off warning.

### 3. Explorer (the app)

```
┌─ mqtt-ui ─────────────────────────────────────────────────────────┐
│ [Home Assistant ▾]  mqtts://ha.local:8883  ● connected  12.4k/s ⚙ │
├────────────────────────┬──────────────────────────────────────────┤
│ 🔍  living                        │ home/living/lamp      JSON  ● │
│                                   ├───────────────────────────────┤
│ ▾ home                    4       │ {                             │
│   ▾ living                2       │   "on": true,                 │
│       lamp     JSON  retain       │   "bri": 180                  │
│       temp     JSON  fresh        │ }                             │
│   ▾ kitchen               1       │                               │
│       fridge   JSON  stale        │                               │
│                                   ├───────────────────────────────┤
│                                   │ jq  ❯ .bri                    │
│                                   │ · · ● · · ·     latest · 12/12│
└────────────────────────┴──────────────────────────────────────────┘
```

- Left: virtualized tree. Chevron expands. Badge = `JSON` when parsed, otherwise omitted. Color of the topic name = freshness (`retain` cyan, `fresh` white, `intime` gray, `stale` dim) — same semantics and default durations as today (`fresh_until` 500 ms, `stale_after` 5 s). Child count on collapsed parents.
- Header profile dropdown switches connection (disconnect + picker, or switch in place after confirm).
- Right: CodeMirror (read-only) for `payloadText`. If `payloadJson` is present, pretty JSON + fold. If `error`, show raw text + warning banner (current TUI “Warning” pane).
- History strip replaces `crumbs.rs`: click a dot or use `h`/`l` (keep vim keys as optional shortcuts; mouse is primary).
- jq bar: same jaq engine, same on-disk history. Enter applies, Esc clears. Errors render under the prompt (no codesnake; a simple message list).
- Copy: Cmd/Ctrl+C copies payload when inspector focused, copies topic when tree focused. No `y` as the only path.

Settings (gear): theme, buffer size, fresh/stale durations. No protocol-decoder list.

Native menu: mqtt-ui / Connections / Edit / View / Help. `Cmd+K` focuses search. `Cmd+,` settings.

---

## What “better than MQTT Explorer” means in v1

Parity is not the goal of v1; **tree + profiles** is. The extras that fall out of the existing core, and that Explorer does poorly or not at all:

1. **jq** on payloads (Explorer has no equivalent).
2. **Fuzzy topic search** with match highlighting (Explorer’s filter is substring).
3. **Fresh / stale / retain** as a live signal, not just a retain icon.
4. **Rust topic store + batched IPC** — Explorer’s Electron path copies every message into the renderer. We never ship the full tree or raw payloads except for the selected topic.
5. **Auto-update** and a ~10 MB download instead of a 150 MB Electron build.

---

## Distribution

GitHub Releases only for v1. Assets per tag:

| Platform | Artifact |
|---|---|
| macOS | Universal `.dmg` (arm64 + x86_64), notarized + stapled |
| Windows | `.msi` and NSIS `.exe`, Authenticode-signed |
| Linux | `.AppImage` and `.deb` for `amd64` and `aarch64` |

Workflow: matrix `macos-latest` / `windows-latest` / `ubuntu-22.04` → `tauri-apps/tauri-action` → draft release. Follow-up job writes `latest.json` for the updater plugin.

**Required secrets (without these, macOS/Windows “easy download” fails Gatekeeper/SmartScreen):**

- Apple Developer ID Application cert + App Store Connect API key (`notarytool`)
- Windows code-signing PFX
- Tauri updater keypair (`TAURI_SIGNING_PRIVATE_KEY`)

Linux AppImage is double-clickable; `.deb` for Debian/Ubuntu. No Snap/Flatpak/Homebrew/winget in v1.

First-run unsigned macOS is not acceptable for this audience — MQTT Explorer’s site already spends a paragraph on “how to open untrusted apps”. Notarize from the first public tag.

---

## App config (replaces current `config.toml`)

```toml
[ui]
theme = "dark"                 # dark | light | system
buffer_size = 0                # 0 = keep all (current default)
fresh_until = "500ms"
stale_after = "5s"

[keys]
search = "/"
ignore = "?"
# copy is system clipboard; vim 'y' remains a shortcut
```

No `[[protocols]]`. Unknown keys are ignored so v2 can add `[record]` / `[publish]` later without a migration scare.

---

## Implementation order

Work is stacked so each step is a runnable app, not a big-bang rewrite.

1. **Workspace split.** `mqttui-core` with `Inbound` / `TopicTree` / JSON `Message` / `Jq` / `ProfileStore`. Unit tests for tree insert / retain-delete / freshness. Current binary can keep working against the library until step 3.
2. **Tauri skeleton.** Empty window, dark shell, `listProfiles` / `saveProfile` round-trip to disk + keychain.
3. **Delete the TUI.** Remove ratatui/crossterm/clap/syntect-tui/ciborium/protocol config. Binary is the Tauri app. README becomes a GUI product page.
4. **Connection picker + editor.** mqtt/mqtts/ws/wss, TLS file pickers, validate toggle, subscribe `#`.
5. **Session + rumqttc.** Status events, reconnect, stats in the header. Ingress goes through `Inbound` → `TopicTree::upsert`.
6. **Tree.** `treeChildren` + `tree/batch` + virtualized Svelte tree + freshness colors + JSON badge.
7. **Inspector.** CodeMirror, history strip, copy, error banner.
8. **Search + jq.** Port remaining mqttui behaviour.
9. **CI + release.** Unsigned CI builds on PR; signed release workflow on `v*` (blocked on certs).
10. **Polish.** Menus, light theme, empty states (connecting, no topics, decode error), updater prompt.

---

## PR Plan

Stacked PRs that implement the v1 app. Each PR is a runnable increment.

### PR 1: Extract mqttui-core library

- **Description:** Turn the TUI into a Cargo workspace and extract `crates/mqttui-core`: `Inbound`, `TopicTree`, JSON-only `Message`/`Format`, `Jq`, `ProfileStore`, and app config via the `directories` crate. Drop CBOR and pluggable decoders from the library API. Unit-test tree insert, empty-retain delete, and freshness. Keep the existing TUI binary compiling against the library so the cutover is not a big bang.
- **Files/components affected:** Cargo.toml, crates/mqttui-core/, src/lib.rs, src/main.rs, src/message.rs, src/model.rs, src/jq.rs, src/config.rs
- **Dependencies:** None

### PR 2: Tauri shell and drop TUI

- **Description:** Scaffold Tauri 2 + Svelte 5 + TypeScript + Vite (`src-tauri/`, `ui/`). Dark-first CSS tokens. Wire `listProfiles` / `getProfile` / `saveProfile` / `deleteProfile` / `pickFile` so the empty window can round-trip a profile to disk + keychain. Delete the Ratatui/clap TUI (`src/ui.rs`, `src/crumbs.rs`, `src/highlight.rs`, `src/events.rs`, clap `Args`). README becomes a GUI product page. Binary is the Tauri app.
- **Files/components affected:** Cargo.toml, src/, src-tauri/, ui/, README.md, package.json
- **Dependencies:** PR 1

### PR 3: Connection picker and profile editor

- **Description:** Implement the connection-picker screen and profile-editor modal from the UI drafts: named profiles, mqtt/mqtts/ws/wss, TLS cert file pickers, validate-certificate toggle with warning, subscriptions defaulting to `#`, last-will in an advanced section, keychain-backed passwords with Linux file fallback.
- **Files/components affected:** ui/src/, src-tauri/src/, crates/mqttui-core/src/
- **Dependencies:** PR 2

### PR 4: Live MQTT session

- **Description:** Add `Session` with rumqttc (`use-rustls` + `websocket`), `Source::Live`, connect/disconnect/reconnect, `session/status` and `session/stats` events. Map publishes to `Inbound` (raw bytes + timestamp) on a decode worker, then `TopicTree::upsert`. Empty retain deletes the leaf. Keep `AsyncClient` on `Session` for v2 publish. No publish UI.
- **Files/components affected:** crates/mqttui-core/src/, src-tauri/src/, ui/src/
- **Dependencies:** PR 2

### PR 5: Explorer tree, inspector, search, and jq

- **Description:** Build the explorer: virtualized topic tree (`treeChildren` + coalesced `tree/batch`), freshness colors, JSON badge, CodeMirror inspector, history strip, copy, error banner, fuzzy/skip search, jq bar with on-disk history. Layout is tree | inspector with the bottom free for a future publish dock.
- **Files/components affected:** ui/src/, src-tauri/src/, crates/mqttui-core/src/
- **Dependencies:** PR 3, PR 4

### PR 6: CI, release, and polish

- **Description:** GitHub Actions CI (unsigned PR builds) and `v*` release workflow (`tauri-action` + `latest.json` updater endpoint). Native menus, light theme, empty states, updater prompt. Signing secrets are documented; unsigned artifacts are acceptable until certs exist.
- **Files/components affected:** .github/workflows/, src-tauri/, ui/src/, README.md
- **Dependencies:** PR 5

---

## Upcoming (v2)

Not scheduled, not designed in full, not in the v1 UI. Listed so v1 does not block them.

### Must not paint over

| v2 feature | What v1 must already have |
|---|---|
| **Recording** | `Inbound` keeps raw bytes + timestamp; single `TopicTree::upsert` write path; session-level hook *after* upsert where a recorder can append. |
| **Playback** | `Source` is an enum; playback is a second source that feeds the same `Inbound` pipeline (no rumqttc). Tree/inspector/jq work unchanged. |
| **Publish + publish history** | `AsyncClient` remains on `Session`; explorer layout is a split that can grow a bottom publish dock; per-profile on-disk log can mirror the jq-history file pattern. |
| Recursive retained delete | Empty retain already deletes a leaf; recursive delete is tree walk + publish empty retain. |
| JSON diff / numeric plots | Per-topic `history` already holds previous payloads; plots/diff are inspector views on that buffer. |
| MQTT 5 property inspector | rumqttc already speaks MQTT 5; v1 connects as 3.1.1. Properties hang on `Inbound` later. |
| Sparkplug / extra codecs | `Format` is an enum with a JSON-first decoder; new variants don't require IPC changes beyond `format`. (CBOR/pluggable decoders stay dropped unless revisited.) |
| Homebrew / winget / Flatpak | Release artifacts already exist; extra channels are packaging only. |

### Recording and playback (shape only)

Append-only file of `Inbound` frames, not a dump of the tree. Suggested on-disk envelope (not implemented):

```json
{ "v": 1, "profileId": "4f2c…", "startedAt": "2026-08-18T12:00:00Z" }
{ "t": 0, "topic": "home/living/lamp", "retain": false, "qos": 0, "payload": "<base64>" }
{ "t": 17, "topic": "home/living/temp", "retain": true, "qos": 0, "payload": "<base64>" }
```

`t` is milliseconds since `startedAt`. Playback sleeps on `t` (or runs as-fast-as-possible) and pushes frames into `TopicTree`. Live connect and playback are mutually exclusive in a window — same “one source at a time” rule as v1’s one broker.

Do not reuse the inspector’s ring buffer as the recording. The ring buffer is capped and per-topic; a recording is the full session stream.

v1 consequence: every `Message` stores `payload: Bytes`, even for JSON. Memory cost is real on `buffer_size = 0`; that is acceptable and already true today (the TUI keeps `text` + `data`).

### Publish including history (shape only)

```ts
// v2 commands, not in v1
publish({ topic, payload: string, qos, retain })
listPublishHistory({ profileId, limit })
replayPublish({ historyId })   // re-send a previous payload
clearRetained({ path, recursive })
```

Publish history is **outgoing**, per profile, persisted (unlike the inspector ring buffer). Typical fields: timestamp, topic, payload text, qos, retain, success/error. The publish dock reuses CodeMirror; a history dropdown sits next to the Publish button, same idea as jq history.

v1 consequence: do not put a throwaway “send” somewhere that we would throw away. Leave the bottom of the inspector free. Keep `client` on `Session`.

---

## Risks

- **`#` on huge brokers.** MQTT Explorer warns about this. Profile subscriptions exist so v1 can subscribe to `home/#` instead. No extra UI beyond the profile field.
- **rumqttc vs MQTT 5 / WebSocket edge brokers.** v1 speaks 3.1.1 over the four transports. If a broker is MQTT 5-only, we bump protocol later; do not block v1 on a property inspector.
- **Keychain on Linux.** `keyring` needs Secret Service; fall back to encrypted file in the config dir if the daemon is missing, and say so in the editor.
- **Code signing lead time.** Architecture does not depend on it; public “easy download” does. Start Apple/Windows certs in parallel with step 1.
- **Raw bytes × unbounded history.** Needed for v2 recording fidelity. If memory becomes an issue before v2, cap `buffer_size` in settings; do not drop raw bytes in favor of pretty-printed text.

---

## Success bar for v1

A Home Assistant / Mosquitto user can download a signed installer for their OS, add a TLS profile with optional client certs, connect, browse a live topic tree, search, inspect JSON/text payloads, run jq, and leave the app running against a noisy broker without the UI locking up. No terminal. No publish, no recording, no CBOR, no plugin decoders.
