# mqx V2: RAM budget, detach, record, and broker replay

| | |
|---|---|
| **Author** | mqx maintainers |
| **Date** | 2026-08-26 |
| **Status** | Draft — awaiting approval |
| **Product** | `mqx` 0.2.0 (follows shipped 0.1.0) |
| **Audience** | Engineers who already know this repo |

This replaces the previous V2 draft. Publish UI, publish history, and recursive retained delete move to **V3**. V2 is the daily-use loop: stay on a live broker without OOM, freeze the tree without disconnecting, capture traffic like `mqtt-trace`, and play that capture back onto a broker from a dedicated Playback tab.

---

## Overview

v1 is a working explorer (Tauri 2 + Svelte 5, one window / one broker, profiles, tree, inspector, jq, topic search). It has three daily-use holes:

1. **Memory.** `UiConfig.buffer_size` defaults to `0` (unbounded). Every payload is kept in `Leaf.history`. A noisy `#` subscription will grow until the process is killed.
2. **No freeze.** The only way to stop the tree from moving is **Disconnect**, which unmounts Explorer and returns to the connection picker (`ui/src/App.svelte` `showPicker = !live`).
3. **No capture in the app.** `tools/mqtt-trace` already records JSONL and replays it onto a broker. That workflow belongs in the UI.

V2 adds a **topic-store RAM budget** with oldest-extra eviction, an Explorer **Live / Detached** toggle that keeps the session and the tree, in-app **recording** (mqtt-trace JSONL), and a **Playback tab** that lists captures and replays them onto a broker — same semantics as `mqtt-trace replay`.

The window still has at most one live MQTT session. Playback does not invent a second in-memory tree. If Explorer is connected, replay publishes on that session’s `AsyncClient`; the tree updates from the broker echo only while Live. If Explorer is not connected, Play connects the chosen profile first, then replays. Play never toggles Live / Detached.

---

## Goals & Non-Goals

### Goals (0.2.0)

- **RAM budget.** Default 12 GiB topic-store cap, Settings slider 4–128 GiB. Periodically drop the oldest *extra* messages. Never drop a topic’s last remaining message. If nothing is eligible and we are still over budget, stop ingesting and warn.
- **Detach.** Explorer stays mounted. MQTT stays connected. Incoming publishes are not ingested. Header shows Live vs Detached. One click either way. Disconnect still returns to the picker.
- **Record.** Settings: default recordings directory. While connected, a Record control starts a capture. Stop opens a name prompt, then saves into that directory. Recording state and the stop action are obvious.
- **Playback tab.** Scrollable list of files in that directory (name, recording time, message count, topic count). Start / Stop + progress. No pause, no skip, no scrubber. Default target is the connected broker; if no Explorer session is open, any saved profile is selectable.
- **Arrow keys** for message history, with an obvious focus model (partially already implemented).

### Non-goals (this version)

| Item | Why | V3? |
|---|---|---|
| Publish dock, publish history, “Publish to topic” | Explicitly later. | Yes |
| Recursive retained delete | Requires publish. | Yes (with publish) |
| Stream JSONL on Play (do not load the whole file) | V2 copies mqtt-trace’s `load_events` → `Vec`. 4 GiB cap is the safety rail. | Yes |
| Multiple inspector tabs on one Explorer | V2 keeps one selected topic. | Yes |
| In-app playback into `TopicTree` with no broker | Replay is mqtt-trace-style: publish onto a broker. | No (unless revisited) |
| Snapshot-on-record (dump current tree into the file) | mqtt-trace records from Start to Stop only. | No |
| Pause / skip / speed / loop on replay | Start, Stop, progress only. Real-time inter-arrival (`speed = 1`). | Maybe with streaming |
| JSON diff / plots, MQTT 5 (`rumqttc::v5`), Sparkplug, Homebrew/winget/Flatpak | Backlog. | Later |
| Multi-broker in one window | One MQTT session per window. Explorer topic tabs are not a second session. | No |

See [Upcoming (V3)](#upcoming-v3) for what V2 must not paint over.

---

## Locked product calls

These were the open forks. They are locked so the rest of the design is implementable. Change them on approval if needed.

1. **Replay publishes onto a broker** (mqtt-trace `replay`), not into a second in-process tree.
2. **If Explorer is not connected, Play connects the chosen profile**, then replays on that session. The Playback tab stays selected so transport controls remain visible; switch to Explorer to watch the tree.
3. **If Explorer is connected, the replay target is that profile.** No second broker, no second MQTT client.
4. **Replay timing is original inter-arrival only.** Stop cancels remaining frames.
5. **Detach keeps MQTT up and drops ingest.** No catch-up buffer. History on screen is frozen. Reconnect-while-detached stays detached.
6. **Recording continues while Detached.** Detach freezes the tree, not the wire capture.
7. **Play does not change Explorer ingest state.** Detached stays Detached; Live stays Live. Replay still publishes to the broker. The tree only moves if ingest is on (Live) and the topic is subscribed. To watch a replay, be Live; to replay without touching the tree, be Detached.
8. **Cancel on the save-name dialog discards** the temp capture.
9. **RAM slider is a hard topic-store budget**, not process RSS. If every remaining message is a topic’s last one, ingest stops and a banner explains why. Record still writes to disk.
10. **Arrow keys:** Up/Down (and j/k) step the selected topic’s history when the messages or inspector pane is focused. Tree keeps its own arrows. Selecting a topic focuses the message list.

---

## Background (what v1 actually shipped)

| Path | Role |
|---|---|
| `crates/mqx-core/` | Profiles, tree, decode, jq, live session |
| `src-tauri/` | IPC, menus, updater |
| `ui/` | Vite + Svelte 5 |
| `tools/mqtt-trace/` | CLI record / replay onto a broker |

`Inbound` already keeps raw `Bytes` + timestamp (`crates/mqx-core/src/message.rs`). `TopicTree::upsert` is the only tree write path. Empty retain deletes the leaf.

`Session` holds `AsyncClient` (comment: “Kept for v2 publish”). Subscribe/disconnect already use it via `LiveHandle` (`session/live.rs`). Replay publish will use the same client. `Session::client()` has no Tauri callers.

Explorer is three columns: tree | message list | inspector (`ui/src/lib/Explorer.svelte`). jq sits in the inspector dock. Window min size 720×480.

`App.svelte` mounts Explorer only for `connecting | connected | reconnecting`. Disconnect unmounts it.

`UiConfig.buffer_size = 0` means no per-topic count cap (`tree.rs` 165–166). That is the OOM.

`mqtt-trace` event line (`tools/mqtt-trace/src/event.rs`):

```json
{ "t_ms": 1787735165782, "topic": "fleet/…", "qos": 1, "retain": true, "dup": false, "payload": "<standard-base64>" }
```

No header. `t_ms` is Unix milliseconds. Replay uses `wait_duration` (`tools/mqtt-trace/src/replay.rs`).

Message history keys already exist when `pane !== "tree"`: `h`/`l`, `ArrowDown`/`j` older, `ArrowUp`/`k` newer (`Explorer.svelte` 570–581). Tree arrows expand/collapse. Users do not discover this because the message list is not focused on topic select and has no listbox keyboard contract.

---

## Proposed design

### Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│  ui/                                                             │
│  Tab: Connections | Explorer | Playback                          │
│  Header (session open): Live/Detached · Record · Disconnect      │
└────────────▲──────────────────────────────────┬──────────────────┘
             │ events                           │ commands
┌────────────┴──────────────────────────────────▼──────────────────┐
│  src-tauri  (thin; clone AsyncClient, drop locks, then await)     │
└────────────▲──────────────────────────────────┬──────────────────┘
             │                                  │
┌────────────┴──────────────────────────────────▼──────────────────┐
│  mqx-core                                                         │
│  Session (tree, ingest_enabled, ram budget, optional Recorder)    │
│  LiveHandle (rumqttc EventLoop + AsyncClient)                     │
│  Replayer (wait_duration → client.publish)                        │
│  decode worker → TopicTree::upsert   (only if ingest_enabled)     │
└───────────────────────────────────────────────────────────────────┘
```

One window, one `LiveHandle`. Playback is a command on that handle’s client, not a second `Source` feeding the tree.

### App chrome: tabs

**Disconnected**

```
[ Connections ]  [ Playback ]
```

Connections is today’s picker. Playback lists files and a profile dropdown (default = last used). Play connects that profile, then starts replay.

**Connected (or connecting / reconnecting / detached)**

```
[ Explorer ]  [ Playback ]          ● connected  mqtt://…  12.4k/s
Live  |  Rec  ● Recording… Stop     [ Disconnect ]
```

Playback target is the open profile (read-only chip, no dropdown).

While replay is running, Explorer header also shows a compact transport: `Replaying lamp.jsonl  0:12 / 0:48  [Stop]`. Clicking the label selects the Playback tab. Progress is independent of Live / Detached; the tree only follows the replay while Live.

`App.svelte` predicates:

```ts
const sessionOpen =
  status === "connecting" || status === "connected" ||
  status === "reconnecting" || status === "detached"
const showHeader = sessionOpen || status === "error"
const showExplorer = sessionOpen && tab === "explorer"
const showPlayback = tab === "playback"
const showPicker = !sessionOpen && status !== "error" && tab === "connections"
```

Disconnected default tab is `connections`. Connected default tab is `explorer`. Playback is always reachable.

Native menus (`src-tauri/src/menu.rs`): View → Explorer / Playback; Connections → Live/Detached, Start/Stop Recording, Disconnect.

---

## RAM budget

### Config

```toml
[ui]
theme = "dark"
buffer_size = 0          # per-topic count cap; 0 = off (unchanged)
fresh_until = "500ms"
stale_after = "5s"
ram_limit_bytes = 12884901888   # 12 GiB default
```

Settings: slider **4 GB … 128 GB**, step 1 GB, label “Topic history limit”, helper text: “Drops older messages when the in-memory store exceeds this. Each topic keeps at least its latest payload. This is not the whole app’s RAM (the window uses extra).”

`getSettings` / `setRamLimit({ bytes })` persist immediately (same pattern as `persist_theme`). Changing the slider on a live session calls `Session::set_ram_limit` and runs one eviction pass.

`buffer_size` stays as an optional extra per-topic count cap for power users who edit `config.toml`. Not in Settings. RAM budget is the product control.

### Accounting

On `TopicTree`, keep `stored_bytes: u64` and a global extra-message index.

```rust
fn message_bytes(message: &Message) -> u64 {
    const OVERHEAD: u64 = 192;
    let payload = message.inbound.payload.len() as u64;
    let text = message.text.len() as u64;
    match message.format {
        Format::Json => payload + text + text + OVERHEAD, // text stands in for serde_json::Value
        Format::Text => payload + text + OVERHEAD,
        Format::Binary => payload + OVERHEAD,
    }
}
```

This is a **store estimate**, deliberately a bit high so we evict before RSS explodes. It does not include the WebView, decode queue, or recorder.

### Eviction rule

- Eligible to drop: messages in `Leaf.history` (not `latest`).
- A topic with `history.is_empty()` is untouchable. That is “never drop a topic that only has a single message.”
- Drop globally oldest extra first (`Inbound.timestamp`, then monotonic `seq` for ties).
- Run after each ingest batch (existing 50 ms coalesce in `live.rs`) and immediately when the slider moves down.
- Hysteresis: once over limit, evict down to **90%** to avoid thrashing.

Do **not** walk all leaves per dropped message. Maintain:

```rust
/// Extra (non-latest) messages, oldest first.
extra_index: BTreeMap<(SystemTime, u64 /*seq*/), String /*topic*/>
next_seq: u64
```

On `insert_leaf` when a previous `latest` is moved into `history`, assign `seq` and insert into `extra_index`. On `pop_front`, remove that key and subtract `message_bytes`. Eviction:

```rust
while self.stored_bytes > target {
    let Some((key, topic)) = self.extra_index.pop_first() else {
        self.ram_exhausted = true;
        break;
    };
    // pop_front on that leaf; subtract bytes; if the selected inspector
    // message was this seq, the UI clamps on the next listHistory
}
if self.stored_bytes <= limit {
    self.ram_exhausted = false;
}
```

`seq` lives on `Message` (or only on extras — simplest: every stored `Message` gets a tree-local `seq` at insert). `HistoryItemDto.index` today is a position into `history` and **shifts on pop_front**. V2 switches `index` to this stable `seq`. `getMessage(topic, index)` looks up by `seq`. `latestIndex` is `latest.seq`. After eviction, if the inspector’s selected `seq` is gone, clamp to the oldest remaining extra, or to latest if none.

`messages_total` / `Leaf.received` stay as “ever seen”, not “currently stored.”

### Exhausted

When `extra_index` is empty and `stored_bytes > limit`:

- Set `ram_exhausted`.
- Skip `TopicTree::upsert` (and skip decode once we know we would not ingest — still record).
- Emit `session/status` with `ramExhausted: true` and a banner: “Topic history is at the 12 GB limit. Every topic already has only its latest message. Raise the limit in Settings or disconnect.”
- Tree patches stop. Freshness still ticks on what is stored.
- Record, if active, still appends wire frames.
- Detach does not clear `ram_exhausted`; raising the slider or eviction becoming possible (user cannot free extras if there are none — only Disconnect / new session clears the tree) is the way out. Lowering unique-topic payload size is not possible without dropping last messages, which we refuse.

### Tests (`tree.rs`)

- Two topics, three messages each, tiny limit → each topic keeps `latest`, extras gone, oldest extras first.
- One message per topic, over limit → no drops, `ram_exhausted`.
- Empty retain delete subtracts bytes and removes extras.
- `seq` stable across eviction; `get_message` by missing seq returns none.

---

## Live / Detached

### Session

```rust
pub enum Status {
    Connecting,
    Connected,
    Detached { since: Instant },
    Reconnecting { since: Instant },
    Disconnected,
    Error { msg: String },
}
```

`StatusKind` gains `"detached"`. Header copy: **Live** vs **Detached**. Button label is the *action*: when Live, “Detach”; when Detached, “Go live”. Same control, two states. Dot color: connected = green, detached = amber, reconnecting = pulsing.

`Session.ingest_enabled: bool`. `Connected` ⇒ true, `Detached` ⇒ false.

Live loop (`session/live.rs` Publish arm): always build `Inbound` (needed for recording). If a recorder is active, `try_append`. If `!ingest_enabled`, do **not** send to the decode queue (saves CPU/RAM while frozen). If `ram_exhausted && ingest_enabled`, skip decode as well.

No catch-up: frames that arrived while detached are gone from the tree (they may be on disk if recording).

Disconnect is unchanged: drop handle, `Disconnected`, picker. Detach must not call `disconnect`.

Connection drop while detached: existing reconnect backoff; stay `Detached` on ConnAck (do not auto-live).

### IPC

| Command | Result |
|---|---|
| `setIngest({ enabled: boolean })` | `{ status }` — refuse if no live handle |

Events: `session/status` includes `detached`. `App.svelte` treats `detached` as `sessionOpen` so Explorer stays mounted.

### Tests

- Detached: publish does not change `topic_count` / `stored_bytes`; recorder still receives the frame (unit-test with a fake recorder).
- Go live: subsequent publish upserts.
- `setIngest` without a session errors.

---

## Recording

### Format

Same event line as mqtt-trace. In-app files add a **header line written at save time** (not at start), so stats exist without a full scan:

```json
{ "kind": "mqx-recording", "v": 1, "startedAt": "2026-08-26T12:00:00.000Z", "endedAt": "2026-08-26T12:00:32.100Z", "profileId": "…", "profileName": "Home Assistant", "broker": "mqtts://ha.local:8883", "messages": 4412, "topics": 318, "appVersion": "0.2.0" }
{ "t_ms": 1787735165782, "topic": "home/lamp", "qos": 0, "retain": false, "dup": false, "payload": "eyJvbiI6dHJ1ZX0=" }
```

- `RecordingHeader`: `#[serde(rename_all = "camelCase")]`.
- `RecordEvent`: field names identical to `TraceEvent` (`t_ms`, `topic`, `qos`, `retain`, `dup`, `payload`). No rename. Standard base64.
- App playback list sniffs `kind`. Headerless mqtt-trace files still list (stats from a scan, cached in memory).
- `mqtt-trace replay` skips a first line that deserializes as a header (`kind` present) so CLI can replay in-app files onto a broker. mqtt-trace does **not** depend on mqx-core. Duplicate the skip + `wait_duration` tests.

No snapshot of the current tree. Capture = frames from Start until Stop, like the CLI.

### Directory

`AppDirs` gains `data_dir` (already `Application Support/mqx` on macOS via `directories`). Default recordings path: `{data_dir}/recordings/`.

```toml
[record]
directory = "/Users/hauke/mqtt-captures"   # empty / omitted = default
```

Settings: path field + Browse (`pickFolder`). Created on first Record if missing.

### UX

1. Connected (or detached): header **Record** button appears.
2. Start: create `{directory}/.mqx-{epoch}.jsonl` (dotfile, 0600). Writer thread appends event lines. Button becomes **Stop recording**, red pill in the header: `● Recording  00:12  8.4k msgs`. Obvious. Click Stop, or View/Connections menu, or `Cmd+Shift+R` toggle.
3. Stop: close the writer (flush). Modal: filename (prefilled `ha-local-2026-08-26-120000.jsonl`, no path). Confirm → prepend header, rename into `directory`, refresh Playback list. Cancel → delete temp. Invalid names (`/`, `..`, empty) rejected.
4. Disconnect / app quit while recording: treat as Stop, then the name modal (quit waits). If the modal cannot run (process kill), leave the dotfile; next launch offers “Finish unsaved recording?” or ignores dotfiles in the list.

State: `record/status` event `{ active, startedMs, messages, bytes, path? }` through `SessionEvent::Record` and the existing epoch-gated forwarder in `commands.rs`.

Queue: recorder thread, `sync_channel(8192)`. On full, increment `dropped` and surface it on the pill. Do not block the MQTT loop.

Detach: still `try_append`. RAM exhausted: still `try_append`.

### IPC

| Command | Notes |
|---|---|
| `startRecording` | Live handle required. |
| `stopRecording` | Returns `{ tempPath, messages, topics, startedMs, endedMs }` for the modal. |
| `saveRecording({ tempPath, name })` | Validates name, writes header, moves. |
| `discardRecording({ tempPath })` | Unlink. |
| `listRecordings` | Files in directory, skip dotfiles. |
| `pickFolder` | Native directory dialog. |
| `getSettings` / `setRecordDirectory` | Persist `[record].directory`. |

### Tests

- Binary payload round-trip (copy mqtt-trace’s fixture).
- Header camelCase fixture.
- Headerless parse.
- `saveRecording` rejects `../x.jsonl`.
- Drop-on-full increments `dropped`.

---

## Playback

### Tab UI

Left (or top): virtualized list of `*.jsonl` in the recordings directory.

Each row:

| Field | Source |
|---|---|
| Name | filename stem |
| Time | header `startedAt`, else first event `t_ms` |
| Messages | header `messages`, else scanned count |
| Topics | header `topics`, else scanned unique topics |

Scan happens once per headerless file, result cached until mtime changes. Do not rescan a 512 MiB mqtt-trace dump on every tab focus.

Empty state: “No recordings yet. Connect and hit Record, or copy a mqtt-trace JSONL into {directory}.”

### Transport

Selected row: **Play** / **Stop**, progress bar, elapsed / duration (`last.t_ms - first.t_ms` at speed 1). No pause, no skip, no speed, no loop.

Play algorithm (always on a live `AsyncClient`):

1. Resolve target profile (open session, or dropdown then `connect`). A fresh connect is Live, as any Connect is; Play must not call `setIngest` or flip Detached ↔ Live on an already-open session.
2. Load events (skip header if present). Refuse files `> 4 GiB` or lines `> 16 MiB`. V2 materializes `Vec<RecordEvent>` like `mqtt-trace` `load_events`; the 4 GiB cap is that loader’s safety rail, not an MQTT limit. V3 streams.
3. Clone `AsyncClient`, drop `AppState` locks, run on a tokio task.
4. For each event: `sleep(wait_duration(t0, t_ms, speed=1, elapsed))` (duplicate of `mqtt-trace` `replay.rs` 27–34, including the `speed <= 0` branch unused here). Then `client.publish(topic, qos, retain, payload)`. `ok` = enqueue, not PubAck.
5. Progress event every 100 ms or every 32 frames: `{ file, index, total, tMs, tEndMs, state: "playing" | "stopped" | "ended" }`.
6. Stop / Disconnect / replace session: abort the task. `state = stopped`.
7. EOF: `state = ended`. If Explorer was Live, echoed frames are already in the tree; if it was Detached, the tree is unchanged.

Explorer sees frames only if it is **Live** and subscribed. Default `#` covers everything. Custom filters mean replayed topics outside the filter will not appear in the tree. Detached: broker still receives the replay; the tree does not move. Status line on the Playback tab: “Replaying onto {broker}. Explorer shows subscribed topics while Live.”

`$SYS` publishes: rumqttc `matches` is irrelevant here (we are the publisher). Brokers usually ignore client `$SYS` publishes. No special case.

### IPC

| Command | Notes |
|---|---|
| `startReplay({ path, profileId? })` | `profileId` required if no session; ignored if session open (must match). |
| `stopReplay` | Abort task. |
| `listRecordings` | Shared with recording. |

Events: `SessionEvent::Playback` through the same forwarder.

### Tests

- `wait_duration` copied from mqtt-trace (real-time gap, pause not applicable).
- Header skip.
- `startReplay` while connected to another profile id → error.
- `startReplay` while Detached does not call `setIngest` and leaves status `detached`.
- Abort mid-replay.
- Files larger than 4 GiB are refused (V2 loader).

---

## Keyboard

Today (`Explorer.svelte` 570–581): with messages/inspector focused, Up/k = newer, Down/j = older; `h`/`l` always step history; tree uses arrows for expand/collapse.

V2:

- Selecting a topic focuses the message list (`pane = "messages"`, focus the listbox).
- Message list is a real listbox: `aria-activedescendant`, roving focus, Up/Down/Home/End/PageUp/PageDown.
- Visible focus ring on the current row (already `.current`).
- Tree arrows unchanged when the tree pane is focused.
- Ignore when typing in search / jq (`isTypingTarget` already).
- README: “↑/↓ steps message history.”

No new global keymap beyond Record toggle (`Cmd+Shift+R`) and Detach (`Cmd+.` or a menu item). Do not steal Cmd+K (search).

---

## API summary (additive)

Commands: `setIngest`, `setRamLimit`, `startRecording`, `stopRecording`, `saveRecording`, `discardRecording`, `listRecordings`, `pickFolder`, `setRecordDirectory`, `startReplay`, `stopReplay`. Expand `getSettings` with `ramLimitBytes`, `recordDirectory`.

Events: existing four, plus `record/status`, `playback/progress`. Both are `SessionEvent` variants, epoch-gated like `commands.rs` 263–277.

`SessionStatus` gains `detached` and `ramExhausted`.

`HistoryItemDto.index` becomes stable `seq` (breaking for anything that assumed `latestIndex === count - 1` as a position — the UI already treats it as an identifier; update `get_history_meta.latest_index` to `latest.seq`).

---

## Data model / migration

- `config.toml`: new `[ui].ram_limit_bytes` (default 12 GiB), optional `[record].directory`. Unknown keys still ignored.
- `Inbound.dup: bool` (from rumqttc `publish.dup`) for lossless JSONL.
- `Message.seq: u64`.
- `TopicTree`: `stored_bytes`, `extra_index`, `ram_limit`, `ram_exhausted`.
- `AppDirs.data_dir` + `recordings_dir()`.
- No `connections.json` change.
- File mode 0600 on recordings (`write_private_file` pattern already used for secrets).

---

## Security & privacy

- Recordings contain raw payloads (credentials, tokens, HA snapshots). Default dir is app data, 0600. Settings may point at a user folder — document that.
- Filename is a basename only; save path is `directory.join(name)` after rejecting separators.
- Replay publishes retained messages as recorded — can mutate broker retained state. Playback tab copy: “Replay publishes to the broker, including retain flags.”
- RAM slider up to 128 GiB: allow it; helper text warns this is the store cap, not a promise the OS can provide it.

---

## Observability

- Existing `mqx.log` (`AppDirs.log_file`): record start/stop/save/discard, replay start/stop/ended, ram_exhausted transitions, recorder drops.
- Header stats pill: topics, msg/s (already). Add stored estimate `1.8 / 12 GB` when over 50%.
- No new telemetry.

---

## Rollout

Version **0.2.0**. GitHub Release + existing updater. No feature flags. Recording `v: 1`. If header `v` > 1, refuse with “upgrade mqx.” mqtt-trace files remain readable.

Rollback: 0.1.0 ignores `[record]` and `ram_limit_bytes`; extra JSONL files are inert.

---

## Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Conservative `message_bytes` still undercounts `serde_json::Value` + WebView | high | Overcount JSON; surface estimate vs limit; ram_exhausted hard stop |
| Eviction index bug drops `latest` | high | `latest` never inserted into `extra_index`; tests |
| Replay + retain mutates production broker | high | Copy on Playback tab; no auto-play |
| Same-client publish while subscribed: echo storms on `#` | medium | Intended when Live (watch replay). Stay Detached to publish without moving the tree. |
| Name-on-stop loses capture on kill | low | Dotfile recovery on next launch |
| 50k unique retained topics × large JSON still exceeds 12 GiB with one msg each | medium | Banner + slider; refuse to drop last message |
| `std::sync::Mutex` across `publish().await` | high | Clone `AsyncClient`, drop guards, then await (same as any future V3 publish) |

---

## Alternatives considered

1. **In-app playback into `TopicTree` (no broker).** Previous draft. Rejected: user asked for mqtt-trace-like replay and a broker target.
2. **Second MQTT client for replay while Explorer is live.** Extra TLS/session state. Rejected: one session per window; publish on the existing client.
3. **Process RSS as the budget.** Unstable (WebView, jemalloc). Rejected: count the store we own.
4. **Disk spill for old messages.** User required stay-in-RAM for performance. Rejected.
5. **Pause = disconnect MQTT, keep tree.** Then Live is a full reconnect (gap, session expiry). Rejected: keep the socket, drop ingest.
6. **Name the file at Record start.** User asked for the prompt on stop.
7. **South publish dock in V2.** Out of scope.
8. **Auto-Live when Play starts.** Rejected: Play must not change Explorer ingest state. Watching a replay is an explicit Go live.

---

## Upcoming (V3)

Not scheduled, not designed in full, not in the V2 UI. Listed so V2 does not block them.

### In scope for V3 (intent)

1. **Publish.** User-composed publish from Explorer: topic, payload (JSON/text/base64), QoS, retain. Per-profile outgoing history and reuse. Recursive retained delete (walk local `retain=true` latest leaves, publish empty retain) ships with this, not before.
2. **Stream replay files.** Play reads JSONL incrementally. Drop the 4 GiB whole-file cap (keep a per-line cap). Pause / skip / speed become feasible once we are not holding the capture in a `Vec`.
3. **Explorer topic tabs.** Several topics inspectable at once (each tab: message list + inspector + jq) on the **same** live session and tree. Not multi-broker, not a second MQTT client.

### Must not paint over

| V3 feature | What V2 must already have / not freeze |
|---|---|
| **Publish** | `LiveHandle` keeps `AsyncClient`. Publish clones the client, drops `AppState` locks, then `.await` (replay already does this). Explorer south edge stays free for a publish dock — do not park Record/Playback chrome there. Do not add a throwaway “send” in V2. `TopicTree::upsert` stays echo-only; publish does not locally upsert. |
| **Recursive retained delete** | Empty retain already deletes a leaf. `iter_leaves` / prefix walk exists. Needs publish + confirmation UI. |
| **Stream replay** | Event lines stay mqtt-trace JSONL (`t_ms` absolute). Header is one first line, skippable; do not require a full index file in V2, but do not invent a format that needs the whole file in RAM to know duration (`startedAt` / `endedAt` already in the header). `wait_duration` takes `(t0, t_ms, elapsed)` — works one event at a time. Keep `startReplay` abortable without draining a preloaded `Vec`. |
| **Explorer topic tabs** | App chrome tabs are `Connections \| Explorer \| Playback`. Topic tabs live **inside** Explorer; do not overload app tabs for this. `selectTopic` / `topic/message` is singular in V2 — do not bake “one inspector forever” into `Session` as a hard invariant (keep `selected: Option<String>` as current UI state, not as “the tree has one viewer”). `getMessage` / `listHistory` / `applyJq` are already keyed by topic, so extra tabs are extra UI subscriptions, not a new store. Layout is tree \| messages \| inspector; a tab strip on the right column should not require moving the tree. |
| **One session per window** | Unchanged. Extra OS windows can each own a session later. Topic tabs share one `LiveHandle`. |

### Explicitly not implied

- Streaming does not mean spilling the topic store to disk (RAM budget stays in-memory).
- Topic tabs do not mean a second broker or a second tree.
- Publish does not mean V2 grows a hidden publish API beyond replay’s `client.publish`.

---

## Open questions (non-blocking)

Settled above. Only implementation-time knobs:

- Exact Record shortcut (`Cmd+Shift+R` vs menu-only).
- Whether “Finish unsaved recording?” on next launch is in 0.2.0 or we just hide dotfiles.
- Whether stored-bytes in the header pill is GiB to one decimal.

If any locked call above is wrong, change it before PR 1.

---

## References

- `docs/plans/planV1.md` § Upcoming (v2)
- `crates/mqx-core/src/tree.rs` — `upsert`, `buffer_size`, `Leaf.history`
- `crates/mqx-core/src/session/live.rs` — event loop, decode queue, `LiveHandle`
- `ui/src/App.svelte` — `live` / `showPicker`
- `ui/src/lib/Explorer.svelte` — panes, existing history keys
- `tools/mqtt-trace/src/event.rs`, `replay.rs`
- MQTT Explorer — not the playback model; we follow mqtt-trace

---

## PR Plan

Each PR is independently reviewable. App remains a usable v1 explorer until record/replay UI lands. RAM and Detach can ship first — they fix crashes and the disconnect-to-picker problem without waiting on JSONL.

### PR 1: Topic-store RAM budget

- **PR title:** `core: RAM budget with oldest-extra eviction`
- **Files:** `crates/mqx-core/src/tree.rs`, `message.rs` (`seq`), `session/mod.rs`, `session/dto.rs` (`HistoryItemDto` seq, `ramExhausted`), `config.rs`, `src-tauri/src/commands.rs` (`getSettings`, `setRamLimit`), `ui/src/lib/Settings.svelte`, `types.ts`, `api.ts`, `App.svelte` (banner)
- **Dependencies:** none
- **Description:** 12 GiB default, slider 4–128 GiB, `extra_index`, stable `seq`, hysteresis to 90%, exhausted banner. `buffer_size` unchanged. Tests as in RAM section.

### PR 2: Live / Detached

- **PR title:** `feat: detach ingest without disconnecting`
- **Files:** `session/mod.rs`, `live.rs`, `dto.rs` (`StatusKind::Detached`), `commands.rs` (`setIngest`), `App.svelte` (predicates, header toggle), `Explorer.svelte`, `menu.rs`
- **Dependencies:** none (can overlap PR 1)
- **Description:** MQTT stays up; skip decode/upsert when detached; reconnect preserves detached; Disconnect still wipes. Header Live/Detached.

### PR 3: Message-list keyboard

- **PR title:** `feat: focus message list and arrow-key history`
- **Files:** `ui/src/lib/Explorer.svelte`, `MessageList.svelte`, `README.md`
- **Dependencies:** none
- **Description:** Focus listbox on topic select; Home/End/PageUp/PageDown; visible focus. Keep tree arrows. Document ↑/↓.

### PR 4: Recording JSONL types + mqtt-trace header skip

- **PR title:** `core: mqtt-trace-compatible recording format`
- **Files:** `crates/mqx-core/src/record.rs` (new), `message.rs` (`Inbound.dup`), `live.rs` (copy `dup`), `tools/mqtt-trace/src/replay.rs` (`load_events` skip header), mqtt-trace tests, mqx-core fixture excerpt of `tools/recordings/127.0.0.1-1883-30s.jsonl`
- **Dependencies:** none
- **Description:** Header camelCase, event lines = `TraceEvent`. Duplicate `wait_duration`. mqtt-trace does not depend on mqx-core. No UI.

### PR 5: In-app recording

- **PR title:** `feat: record live traffic to JSONL`
- **Files:** `record.rs`, `session/live.rs`, `config.rs` (`RecordConfig`, `AppDirs.data_dir`), `commands.rs` (start/stop/save/discard, `pickFolder`, `setRecordDirectory`, `SessionEvent::Record`), `menu.rs`, `App.svelte` (Record pill + name modal), `Settings.svelte` (directory)
- **Dependencies:** PR 2 (detach still records), PR 4
- **Description:** Temp dotfile, name on stop, cancel discards, 0600, recorder thread, continues while detached and while ram_exhausted.

### PR 6: Playback tab and broker replay

- **PR title:** `feat: Playback tab replays JSONL onto the connected broker`
- **Files:** `ui/src/lib/Playback.svelte` (new), `App.svelte` (tabs, compact Explorer transport), `session/replay.rs` (new) or tauri-side task, `commands.rs` (`startReplay`/`stopReplay`/`listRecordings`), `menu.rs`, `api.ts`, `types.ts`
- **Dependencies:** PR 2 (session / Detached must exist; Play must not call `setIngest`), PR 4, PR 5 (list/directory; headerless mqtt-trace files are enough to test)
- **Description:** File list + stats, Play/Stop/progress, connect-then-replay when disconnected (a new session is Live, like Connect). Do **not** flip Live/Detached on an existing session. Lock target to open session, clone client then await publishes, `wait_duration` speed 1. Load-all with a **4 GiB** file cap (V3 streams). Copy about retain, subscriptions, and Detached. Leave Explorer south edge empty.

### PR 7: 0.2.0 polish

- **PR title:** `chore: 0.2.0 changelog, menus, README`
- **Files:** `CHANGELOG.md`, versions, `README.md` (record/replay, RAM slider, detach, mqtt-trace caveat that CLI replay of in-app files is a traffic dump onto a broker), `docs/plans/planV1.md` pointer
- **Dependencies:** PRs 1–6

**Parallelization:** PR 1, 2, 3, 4 are independent. PR 5 after 2+4. PR 6 after 2+4 (5 preferred). PR 7 last.
