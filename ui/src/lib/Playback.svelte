<script lang="ts">
  import { onMount } from "svelte";
  import { deleteRecording, listRecordings, openRecordingsFolder } from "./api";
  import {
    brokerLabel,
    errorMessage,
    formatClock,
    formatRecordingTime,
    recordingLabel,
    playbackSpanMs,
    type PlaybackProgress,
    type ProfileSummary,
    type RecordingInfo,
  } from "./types";

  let {
    profiles,
    lastUsedId = null,
    sessionOpen = false,
    sessionProfileId = null,
    sessionBroker = "",
    ingestEnabled = true,
    playback = null,
    busy = false,
    t0Ms = null,
    onplay,
    onstop,
  }: {
    profiles: ProfileSummary[];
    lastUsedId?: string | null;
    sessionOpen?: boolean;
    sessionProfileId?: string | null;
    sessionBroker?: string;
    ingestEnabled?: boolean;
    playback?: PlaybackProgress | null;
    busy?: boolean;
    t0Ms?: number | null;
    onplay: (path: string, profileId: string) => void;
    onstop: () => void;
  } = $props();

  let recordings = $state<RecordingInfo[]>([]);
  let directory = $state("");
  let selectedPath = $state<string | null>(null);
  let profileId = $state("");
  let loadError = $state<string | null>(null);
  let loaded = $state(false);
  let changing = $state(false);
  let menu = $state<{ recording: RecordingInfo; x: number; y: number } | null>(null);

  const selected = $derived(recordings.find((item) => item.path === selectedPath) ?? null);
  const playing = $derived(playback?.state === "playing");
  const targetProfileId = $derived(
    sessionOpen ? (sessionProfileId ?? "") : profileId || lastUsedId || profiles[0]?.id || "",
  );
  const targetLabel = $derived.by(() => {
    if (sessionOpen && sessionBroker) {
      return sessionBroker;
    }
    const profile = profiles.find((item) => item.id === targetProfileId);
    return profile ? brokerLabel(profile) : "";
  });
  const span = $derived(playback ? playbackSpanMs(playback, t0Ms) : { elapsed: 0, duration: 0 });
  const canPlay = $derived(
    !!selected && !!targetProfileId && !busy && !changing && (!sessionOpen || !!sessionProfileId),
  );

  async function refresh() {
    try {
      const listed = await listRecordings();
      directory = listed.directory;
      recordings = listed.recordings;
      if (selectedPath && !recordings.some((item) => item.path === selectedPath)) {
        selectedPath = recordings[0]?.path ?? null;
      } else if (!selectedPath) {
        selectedPath = recordings[0]?.path ?? null;
      }
      loadError = null;
    } catch (err) {
      loadError = errorMessage(err);
    } finally {
      loaded = true;
    }
  }

  async function openFolder() {
    try {
      await openRecordingsFolder();
      loadError = null;
    } catch (err) {
      loadError = errorMessage(err);
    }
  }

  function showMenu(event: MouseEvent, recording: RecordingInfo) {
    event.preventDefault();
    selectedPath = recording.path;
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const x = event.clientX || rect.left + 20;
    const y = event.clientY || rect.top + rect.height;
    menu = {
      recording,
      x: Math.max(0, Math.min(x, window.innerWidth - 190)),
      y: Math.max(0, Math.min(y, window.innerHeight - 60)),
    };
  }

  async function removeRecording(recording: RecordingInfo) {
    menu = null;
    if (!confirm(`Permanently delete “${recordingLabel(recording.name)}”?`)) return;
    changing = true;
    try {
      await deleteRecording(recording.path);
      await refresh();
    } catch (err) {
      loadError = errorMessage(err);
    } finally {
      changing = false;
    }
  }

  function duration(recording: RecordingInfo): string {
    if (recording.firstTMs == null || recording.lastTMs == null) return "Duration unavailable";
    const ms = Math.max(0, recording.lastTMs - recording.firstTMs);
    if (ms > 0 && ms < 1000) return `${ms} ms`;
    return formatClock(ms);
  }

  $effect(() => {
    if (!profileId && lastUsedId && profiles.some((profile) => profile.id === lastUsedId)) {
      profileId = lastUsedId;
    } else if (!profileId && profiles[0]) {
      profileId = profiles[0].id;
    }
  });

  onMount(() => {
    void refresh();
  });
</script>

<svelte:window
  onfocus={() => { if (loaded) void refresh(); }}
  onpointerdown={(event) => {
    if (!(event.target instanceof Element) || !event.target.closest("[data-playback-menu]")) menu = null;
  }}
  onkeydown={(event) => { if (event.key === "Escape") menu = null; }}
/>

<section class="playback">
  <header>
    <h2>Playback</h2>
    {#if sessionOpen}
      <p class="target" title={targetLabel}>
        Target <span class="chip">{targetLabel || "connected broker"}</span>
      </p>
    {:else}
      <label class="target">
        Target
        <select bind:value={profileId} disabled={busy || playing || profiles.length === 0}>
          {#if profiles.length === 0}
            <option value="">No profiles</option>
          {:else}
            {#each profiles as profile (profile.id)}
              <option value={profile.id}>{profile.name} · {brokerLabel(profile)}</option>
            {/each}
          {/if}
        </select>
      </label>
    {/if}
  </header>

  <div class="folder-actions">
    <button type="button" onclick={openFolder}>Open recordings folder</button>
    <button type="button" onclick={refresh}>Refresh</button>
  </div>

  {#if loadError}
    <p class="banner" role="alert">{loadError}</p>
  {/if}

  {#if !loaded}
    <p class="muted">Loading…</p>
  {:else if recordings.length === 0}
    <p class="empty">
      No recordings yet. Connect and hit Record, or copy a mqtt-trace capture into
      <span class="dir">{directory || "the recordings folder"}</span>.
    </p>
  {:else}
    <ul class="list" role="listbox" aria-label="Recordings">
      {#each recordings as recording (recording.path)}
        {@const active = selectedPath === recording.path}
        {@const current = playback?.file === recording.fileName}
        <li>
          <button
            type="button"
            class="row"
            class:active
            class:current
            role="option"
            aria-selected={active}
            onclick={() => (selectedPath = recording.path)}
            oncontextmenu={(event) => showMenu(event, recording)}
          >
            <span class="name">{recordingLabel(recording.name)}</span>
            <span class="time">{formatRecordingTime(recording)}</span>
            <span class="meta">{duration(recording)} · {recording.messages} messages · {recording.topics} topics</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  {#if menu}
    <div class="menu" data-playback-menu style:left="{menu.x}px" style:top="{menu.y}px" role="menu">
      <button
        type="button"
        role="menuitem"
        disabled={changing || busy || (playing && playback?.file === menu.recording.fileName)}
        onclick={() => { if (menu) void removeRecording(menu.recording); }}
      >Delete trace…</button>
    </div>
  {/if}

  <div class="transport">
    <div class="buttons">
      {#if playing}
        <button type="button" class="stop" disabled={busy} onclick={onstop}>Stop</button>
      {:else}
        <button
          type="button"
          class="play"
          disabled={!canPlay}
          onclick={() => {
            if (selected && targetProfileId) {
              onplay(selected.path, targetProfileId);
            }
          }}
        >
          Play
        </button>
      {/if}
    </div>
    <div class="progress">
      <progress max={Math.max(span.duration, 1)} value={playback ? span.elapsed : 0}></progress>
      <span class="clock">
        {formatClock(playback ? span.elapsed : 0)}
        /
        {formatClock(span.duration)}
      </span>
    </div>
  </div>

  <p class="copy">
    Replay publishes to the broker, including retain flags.
    {#if sessionOpen}
      Replaying onto {sessionBroker || "the connected broker"}. Explorer shows subscribed topics
      while Live{ingestEnabled ? "" : " — Detached, so the tree will not move"}.
    {:else}
      Play connects the selected profile first (Live), then publishes onto that broker.
    {/if}
  </p>
</section>

<style>
  .playback {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 56rem;
    width: 100%;
    margin: 0 auto;
    padding: var(--space-5);
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  h2 {
    font-size: 1.15rem;
    font-weight: 650;
    letter-spacing: -0.02em;
  }

  .target {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--fg-muted);
    font-size: 13px;
    min-width: 0;
  }

  .folder-actions {
    display: flex;
    gap: var(--space-2);
  }

  .folder-actions button {
    border: 1px solid var(--border-strong);
    background: var(--bg-elevated);
    border-radius: var(--radius-sm);
    padding: 0.3rem 0.6rem;
    font-size: 12px;
  }

  .menu {
    position: fixed;
    z-index: 10;
    min-width: 11rem;
    padding: 0.2rem;
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    box-shadow: var(--shadow);
  }

  .menu button {
    display: block;
    width: 100%;
    text-align: left;
    padding: 0.4rem 0.55rem;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--danger);
    font-size: 12px;
  }

  .menu button:hover:not(:disabled) { background: var(--bg-hover); }

  .chip {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--fg);
    background: var(--bg-hover);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0.15rem 0.6rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 28rem;
  }

  select {
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
    border-radius: var(--radius-sm);
    padding: 0.3rem 0.5rem;
    max-width: 24rem;
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    flex: 1;
    min-height: 0;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-elevated);
  }

  .row {
    width: 100%;
    display: grid;
    grid-template-columns: minmax(8rem, 1.4fr) minmax(8rem, 1fr) auto;
    gap: var(--space-3);
    align-items: baseline;
    text-align: left;
    border: 0;
    border-bottom: 1px solid var(--border);
    background: transparent;
    padding: 0.65rem 0.85rem;
    border-radius: 0;
  }

  .row:last-child {
    border-bottom: 0;
  }

  .row:hover,
  .row.active {
    background: var(--bg-hover);
  }

  .row.current {
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .time,
  .meta {
    color: var(--fg-muted);
    font-size: 12px;
    font-family: var(--mono);
  }

  .transport {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .play,
  .stop {
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--accent-fg);
    border-radius: var(--radius-sm);
    padding: 0.4rem 0.9rem;
    font-weight: 600;
    min-width: 4.5rem;
  }

  .stop {
    border-color: var(--danger);
    background: var(--danger);
    color: var(--danger-fg);
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .progress {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 12rem;
  }

  progress {
    flex: 1;
    height: 0.45rem;
    accent-color: var(--accent);
  }

  .clock {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--fg-muted);
    white-space: nowrap;
  }

  .copy,
  .muted,
  .empty {
    color: var(--fg-muted);
    font-size: 13px;
  }

  .empty {
    flex: 1;
    line-height: 1.55;
  }

  .dir {
    font-family: var(--mono);
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .banner {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
</style>
