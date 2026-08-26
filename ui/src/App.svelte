<script lang="ts">
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import {
    connect as connectSession,
    deleteProfile,
    discardRecording,
    disconnect as disconnectSession,
    exitApp,
    getProfile,
    getSettings,
    listProfiles,
    saveProfile,
    saveRecording,
    setIngest,
    setRamLimit,
    setRecordDirectory,
    setTheme,
    startRecording,
    startReplay,
    stopRecording,
    stopReplay as stopReplaySession,
  } from "./lib/api";
  import ConnectionPicker from "./lib/ConnectionPicker.svelte";
  import Explorer from "./lib/Explorer.svelte";
  import Playback from "./lib/Playback.svelte";
  import ProfileEditor from "./lib/ProfileEditor.svelte";
  import Settings from "./lib/Settings.svelte";
  import { applyTheme, isTheme, readCachedTheme, type Theme } from "./lib/theme";
  import {
    brokerLabel,
    draftFromProfile,
    draftToSaveInput,
    duplicateName,
    emptyDraft,
    DEFAULT_RAM_LIMIT_BYTES,
    errorMessage,
    formatClock,
    formatCount,
    formatElapsed,
    formatRate,
    formatStoreUsage,
    generateClientId,
    ramLimitGb,
    getLastUsedProfileId,
    idleSessionStatus,
    isStaleSessionStatus,
    playbackSpanMs,
    sessionOpen as isSessionOpen,
    setLastUsedProfileId,
    statusLabel,
    suggestedRecordingName,
    type AppTab,
    type PlaybackProgress,
    type ProfileDraft,
    type ProfileSummary,
    type RecordStatus,
    type SessionStats,
    type SessionStatus,
    type StoppedRecording,
    type UiSettings,
  } from "./lib/types";

  let profiles = $state<ProfileSummary[]>([]);
  let lastUsedId = $state<string | null>(getLastUsedProfileId());
  let draft = $state<ProfileDraft | null>(null);
  let error = $state<string | null>(null);
  let editorError = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let busy = $state(false);
  let loaded = $state(false);
  let opener = $state<HTMLElement | null>(null);
  let sessionStatus = $state<SessionStatus>(idleSessionStatus());
  let stats = $state<SessionStats | null>(null);
  let wantedProfileId = $state<string | null>(null);
  let wantedEpoch = $state<number | null>(null);
  let theme = $state<Theme>(readCachedTheme());
  let ramLimitBytes = $state(DEFAULT_RAM_LIMIT_BYTES);
  let settingsOpen = $state(false);
  let settingsError = $state<string | null>(null);
  let recordDirectory = $state("");
  let recordStatus = $state<RecordStatus | null>(null);
  let savePrompt = $state<{ tempPath: string; name: string; error: string | null } | null>(null);
  let saveNameInput = $state<HTMLInputElement | null>(null);
  let pendingDisconnect = $state(false);
  let pendingQuit = $state(false);
  let nowMs = $state(Date.now());
  const recording = $derived(recordStatus?.active === true);
  let tab = $state<AppTab>("connections");
  let playback = $state<PlaybackProgress | null>(null);
  let replayT0 = $state<number | null>(null);
  let wantedGeneration = $state<number | null>(null);

  $effect(() => {
    if (!recording) {
      return;
    }
    nowMs = Date.now();
    const id = setInterval(() => {
      nowMs = Date.now();
    }, 250);
    return () => clearInterval(id);
  });

  $effect(() => {
    if (savePrompt && saveNameInput) {
      saveNameInput.focus();
      saveNameInput.select();
    }
  });

  applyTheme(readCachedTheme());

  const sessionOpen = $derived(isSessionOpen(sessionStatus.status));
  const showHeader = $derived(sessionOpen || sessionStatus.status === "error");
  const showExplorer = $derived(sessionOpen && tab === "explorer");
  const showPlayback = $derived(tab === "playback");
  const showPicker = $derived(!sessionOpen && sessionStatus.status !== "error" && tab === "connections");
  const ingestEnabled = $derived(sessionStatus.ingestEnabled ?? sessionStatus.status !== "detached");
  const replaying = $derived(playback?.state === "playing");
  const replaySpan = $derived(playback ? playbackSpanMs(playback, replayT0) : { elapsed: 0, duration: 0 });

  $effect(() => {
    if (sessionOpen) {
      if (tab === "connections") {
        tab = "explorer";
      }
    } else if (sessionStatus.status !== "error" && tab === "explorer") {
      tab = "connections";
    }
  });

  function selectTab(next: AppTab) {
    if (next === "explorer" && !sessionOpen) {
      tab = "connections";
      return;
    }
    if (next === "connections" && sessionOpen) {
      tab = "explorer";
      return;
    }
    tab = next;
  }

  const lastUsedName = $derived(
    lastUsedId ? (profiles.find((profile) => profile.id === lastUsedId)?.name ?? null) : null,
  );
  const storeUsage = $derived(
    stats ? formatStoreUsage(stats.storedBytes ?? 0, stats.ramLimitBytes ?? ramLimitBytes) : null,
  );
  const ramLimitLabel = $derived(ramLimitGb(ramLimitBytes));

  function clearMessages() {
    error = null;
    notice = null;
  }

  async function refresh() {
    profiles = await listProfiles();
    if (lastUsedId && !profiles.some((profile) => profile.id === lastUsedId)) {
      lastUsedId = null;
      setLastUsedProfileId(null);
    }
  }

  function rememberOpener() {
    const active = document.activeElement;
    opener = active instanceof HTMLElement ? active : null;
  }

  function closeEditor() {
    draft = null;
    editorError = null;
    const target = opener;
    opener = null;
    queueMicrotask(() => target?.focus());
  }

  function openSettings() {
    settingsError = null;
    settingsOpen = true;
  }

  function closeSettings() {
    settingsOpen = false;
    settingsError = null;
  }

  function applySettings(settings: UiSettings) {
    if (isTheme(settings.theme)) {
      theme = settings.theme;
      applyTheme(settings.theme);
    }
    if (typeof settings.recordDirectory === "string") {
      recordDirectory = settings.recordDirectory;
    }
    if (typeof settings.ramLimitBytes === "number") {
      ramLimitBytes = settings.ramLimitBytes;
    }
  }

  async function chooseTheme(next: Theme) {
    theme = next;
    applyTheme(next);
    try {
      applySettings(await setTheme(next));
    } catch (err) {
      settingsError = errorMessage(err);
    }
  }

  async function chooseRamLimit(bytes: number) {
    ramLimitBytes = bytes;
    try {
      applySettings(await setRamLimit(bytes));
    } catch (err) {
      settingsError = errorMessage(err);
    }
  }

  async function chooseRecordDirectory(directory: string) {
    try {
      applySettings(await setRecordDirectory(directory));
    } catch (err) {
      settingsError = errorMessage(err);
      settingsOpen = true;
    }
  }

  function openNew() {
    clearMessages();
    editorError = null;
    rememberOpener();
    draft = emptyDraft();
  }

  async function openEdit(id: string) {
    clearMessages();
    editorError = null;
    rememberOpener();
    if (!id) {
      draft = emptyDraft();
      return;
    }
    try {
      draft = draftFromProfile(await getProfile(id));
    } catch (err) {
      error = errorMessage(err);
    }
  }

  async function duplicate(id: string) {
    clearMessages();
    busy = true;
    try {
      const profile = await getProfile(id);
      const input = draftToSaveInput({
        ...draftFromProfile(profile),
        id: "",
        name: duplicateName(profile.name),
        clientId: generateClientId(profile.clientId),
        password: "",
        hasPassword: false,
      });
      await saveProfile(input);
      await refresh();
      notice = "Profile duplicated. Re-enter the password if the broker requires one.";
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busy = false;
    }
  }

  async function remove(id: string, name: string) {
    if (!confirm(`Delete “${name}”?`)) {
      return;
    }
    clearMessages();
    busy = true;
    try {
      await deleteProfile(id);
      if (lastUsedId === id) {
        lastUsedId = null;
        setLastUsedProfileId(null);
      }
      if (draft?.id === id) {
        closeEditor();
      }
      await refresh();
      notice = "Profile deleted.";
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busy = false;
    }
  }

  async function persist(next: ProfileDraft): Promise<string> {
    const { id } = await saveProfile(draftToSaveInput(next));
    await refresh();
    return id;
  }

  async function requestConnect(id: string) {
    lastUsedId = id;
    setLastUsedProfileId(id);
    const profile = profiles.find((item) => item.id === id);
    wantedProfileId = id;
    wantedEpoch = null;
    sessionStatus = {
      profileId: id,
      status: "connecting",
      ingestEnabled: true,
      broker: profile ? brokerLabel(profile) : "",
    };
    stats = null;
    const { epoch } = await connectSession(id);
    wantedEpoch = epoch;
    sessionStatus = { ...sessionStatus, epoch };
  }

  async function connect(id: string) {
    clearMessages();
    busy = true;
    try {
      if (!id) {
        const created = await persist(emptyDraft());
        await requestConnect(created);
        return;
      }
      await requestConnect(id);
    } catch (err) {
      error = errorMessage(err);
      wantedProfileId = null;
      wantedEpoch = null;
      sessionStatus = idleSessionStatus();
      stats = null;
    } finally {
      busy = false;
    }
  }

  async function disconnect() {
    error = null;
    notice = null;
    if (recording) {
      pendingDisconnect = true;
      await stopAndPrompt();
      return;
    }
    if (savePrompt) {
      pendingDisconnect = true;
      return;
    }
    await actuallyDisconnect();
  }

  async function actuallyDisconnect() {
    wantedProfileId = null;
    wantedEpoch = null;
    playback = null;
    replayT0 = null;
    wantedGeneration = null;
    busy = true;
    try {
      await disconnectSession();
      sessionStatus = idleSessionStatus();
      stats = null;
      if (!savePrompt) {
        recordStatus = null;
      }
      playback = null;
      replayT0 = null;
      wantedGeneration = null;
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busy = false;
    }
  }

  async function requestQuit() {
    if (recording) {
      pendingQuit = true;
      await stopAndPrompt();
      return;
    }
    if (savePrompt) {
      pendingQuit = true;
      return;
    }
    await exitApp();
  }

  async function finishPendingAction() {
    if (pendingQuit) {
      pendingQuit = false;
      pendingDisconnect = false;
      await exitApp();
      return;
    }
    if (pendingDisconnect) {
      pendingDisconnect = false;
      await actuallyDisconnect();
    }
  }

  function acceptRecordStatus(payload: RecordStatus) {
    if (wantedEpoch != null && payload.epoch != null && payload.epoch !== wantedEpoch) {
      return;
    }
    const wasRecording = recordStatus?.active === true;
    recordStatus = payload.active ? payload : recordStatus;
    if (wasRecording && !payload.active && payload.path && !savePrompt) {
      openSavePrompt({
        tempPath: payload.path,
        messages: payload.messages,
        topics: payload.topics,
        startedMs: payload.startedMs,
        endedMs: payload.endedMs ?? Date.now(),
      });
      return;
    }
    if (!payload.active && !savePrompt) {
      recordStatus = null;
    }
  }

  function openSavePrompt(stopped: Pick<StoppedRecording, "tempPath" | "messages" | "topics" | "startedMs" | "endedMs">) {
    savePrompt = {
      tempPath: stopped.tempPath,
      name: suggestedRecordingName(sessionStatus.broker, stopped.startedMs),
      error: null,
    };
    recordStatus = {
      active: false,
      startedMs: stopped.startedMs,
      endedMs: stopped.endedMs,
      messages: stopped.messages,
      bytes: 0,
      dropped: 0,
      topics: stopped.topics,
      path: stopped.tempPath,
    };
  }

  async function toggleRecording() {
    if (!isSessionOpen(sessionStatus.status) || savePrompt || busy) {
      return;
    }
    if (recording) {
      await stopAndPrompt();
    } else {
      await beginRecording();
    }
  }

  async function beginRecording() {
    error = null;
    busy = true;
    try {
      recordStatus = await startRecording();
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busy = false;
    }
  }

  async function stopAndPrompt() {
    error = null;
    busy = true;
    try {
      openSavePrompt(await stopRecording());
    } catch (err) {
      if (recordStatus) {
        recordStatus = { ...recordStatus, active: false };
      }
      error = errorMessage(err);
      pendingDisconnect = false;
      pendingQuit = false;
    } finally {
      busy = false;
    }
  }

  async function confirmSave() {
    const prompt = savePrompt;
    if (!prompt) {
      return;
    }
    const name = prompt.name.trim();
    if (!name || name.includes("/") || name.includes("\\") || name.includes("..")) {
      savePrompt = { ...prompt, error: "Name must be a file name, not a path." };
      return;
    }
    busy = true;
    try {
      await saveRecording(prompt.tempPath, name);
      savePrompt = null;
      recordStatus = null;
      await finishPendingAction();
    } catch (err) {
      savePrompt = { ...prompt, error: errorMessage(err) };
    } finally {
      busy = false;
    }
  }

  async function cancelSave() {
    const prompt = savePrompt;
    if (!prompt) {
      return;
    }
    busy = true;
    try {
      await discardRecording(prompt.tempPath);
      savePrompt = null;
      recordStatus = null;
      await finishPendingAction();
    } catch (err) {
      savePrompt = { ...prompt, error: errorMessage(err) };
    } finally {
      busy = false;
    }
  }

  function acceptStatus(payload: SessionStatus): boolean {
    if (payload.profileId !== wantedProfileId) {
      return false;
    }
    if (wantedEpoch != null && payload.epoch !== wantedEpoch) {
      return false;
    }
    if (isStaleSessionStatus(sessionStatus, payload)) {
      return false;
    }
    sessionStatus = payload;
    if (payload.status === "disconnected" || payload.status === "error") {
      stats = null;
    }
    return true;
  }

  function acceptPlayback(payload: PlaybackProgress) {
    if (wantedEpoch == null) {
      return;
    }
    if (payload.epoch != null && payload.epoch !== wantedEpoch) {
      return;
    }
    const generation = payload.generation;
    if (payload.state === "stopped") {
      if (wantedGeneration != null && generation != null && generation !== wantedGeneration) {
        return;
      }
    } else if (wantedGeneration != null && generation != null && generation < wantedGeneration) {
      return;
    }
    if (payload.state === "playing" && generation != null) {
      wantedGeneration = generation;
    }
    playback = payload;
    if (payload.state === "playing" && (replayT0 == null || payload.index === 0)) {
      replayT0 = payload.tMs;
    }
    if (payload.state === "stopped") {
      replayT0 = payload.index === 0 ? null : replayT0;
    }
  }

  async function playRecording(path: string, profileId: string) {
    clearMessages();
    busy = true;
    try {
      if (!isSessionOpen(sessionStatus.status)) {
        await requestConnect(profileId);
      }
      const reply = await startReplay(path, profileId);
      if (wantedEpoch == null) {
        wantedEpoch = reply.epoch;
      }
      wantedGeneration = reply.generation;
      replayT0 = reply.tMs;
      playback = {
        file: reply.file,
        index: 0,
        total: reply.total,
        tMs: reply.tMs,
        tEndMs: reply.tEndMs,
        state: "playing",
        epoch: reply.epoch,
        generation: reply.generation,
      };
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busy = false;
    }
  }

  async function stopPlayback() {
    error = null;
    busy = true;
    try {
      await stopReplaySession();
      if (playback?.state === "playing") {
        playback = { ...playback, state: "stopped" };
      }
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busy = false;
    }
  }

  async function applyIngest(enabled: boolean) {
    if (!isSessionOpen(sessionStatus.status)) {
      return;
    }
    error = null;
    busy = true;
    try {
      acceptStatus(await setIngest(enabled));
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busy = false;
    }
  }

  async function save() {
    if (!draft) {
      return;
    }
    clearMessages();
    editorError = null;
    busy = true;
    try {
      await persist(draft);
      closeEditor();
      notice = "Profile saved.";
    } catch (err) {
      editorError = errorMessage(err);
    } finally {
      busy = false;
    }
  }

  async function saveAndConnect() {
    if (!draft) {
      return;
    }
    clearMessages();
    editorError = null;
    busy = true;
    try {
      const id = await persist(draft);
      closeEditor();
      await requestConnect(id);
    } catch (err) {
      const message = errorMessage(err);
      if (draft) {
        editorError = message;
      } else {
        error = message;
      }
    } finally {
      busy = false;
    }
  }

  function cancel() {
    closeEditor();
  }

  onMount(() => {
    const unlistens: UnlistenFn[] = [];
    let cancelled = false;

    void (async () => {
      try {
        const statusUnlisten = await listen<SessionStatus>("session/status", (event) => {
          acceptStatus(event.payload);
        });
        const statsUnlisten = await listen<SessionStats>("session/stats", (event) => {
          const payload = event.payload;
          if (payload.profileId && payload.profileId !== wantedProfileId) {
            return;
          }
          if (wantedEpoch != null && payload.epoch !== wantedEpoch) {
            return;
          }
          stats = payload;
        });
        const themeUnlisten = await listen<string>("settings/theme", (event) => {
          if (isTheme(event.payload)) {
            theme = event.payload;
            applyTheme(event.payload);
          }
        });
        const settingsUnlisten = await listen("menu/settings", () => {
          openSettings();
        });
        const newConnectionUnlisten = await listen("menu/new-connection", () => {
          openNew();
        });
        const disconnectUnlisten = await listen("menu/disconnect", () => {
          void disconnect();
        });
        const ingestUnlisten = await listen<boolean>("menu/set-ingest", (event) => {
          void applyIngest(event.payload);
        });
        const recordUnlisten = await listen<RecordStatus>("record/status", (event) => {
          acceptRecordStatus(event.payload);
        });
        const toggleRecordingUnlisten = await listen("menu/toggle-recording", () => {
          void toggleRecording();
        });
        const closeRequestedUnlisten = await listen("app/close-requested", () => {
          void requestQuit();
        });
        const playbackUnlisten = await listen<PlaybackProgress>("playback/progress", (event) => {
          acceptPlayback(event.payload);
        });
        const tabUnlisten = await listen<string>("menu/tab", (event) => {
          if (event.payload === "playback") {
            selectTab("playback");
          } else if (event.payload === "explorer") {
            selectTab("explorer");
          }
        });
        const settingsErrorUnlisten = await listen<string>("settings/error", (event) => {
          settingsError = event.payload;
          settingsOpen = true;
        });
        if (cancelled) {
          statusUnlisten();
          statsUnlisten();
          themeUnlisten();
          settingsUnlisten();
          newConnectionUnlisten();
          disconnectUnlisten();
          ingestUnlisten();
          recordUnlisten();
          toggleRecordingUnlisten();
          closeRequestedUnlisten();
          playbackUnlisten();
          tabUnlisten();
          settingsErrorUnlisten();
          return;
        }
        unlistens.push(
          statusUnlisten,
          statsUnlisten,
          themeUnlisten,
          settingsUnlisten,
          newConnectionUnlisten,
          disconnectUnlisten,
          ingestUnlisten,
          recordUnlisten,
          toggleRecordingUnlisten,
          closeRequestedUnlisten,
          playbackUnlisten,
          tabUnlisten,
          settingsErrorUnlisten,
        );
      } catch (err) {
        error = errorMessage(err);
      }

      try {
        applySettings(await getSettings());
      } catch {
        applyTheme(theme);
      }

      try {
        await refresh();
      } catch (err) {
        error = errorMessage(err);
      } finally {
        loaded = true;
      }
    })();

    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape" && savePrompt) {
        event.preventDefault();
        void cancelSave();
        return;
      }
      if ((event.metaKey || event.ctrlKey) && event.key === ",") {
        event.preventDefault();
        openSettings();
      }
      if (
        (event.metaKey || event.ctrlKey) &&
        event.shiftKey &&
        (event.key === "R" || event.key === "r")
      ) {
        event.preventDefault();
        void toggleRecording();
      }
    }
    window.addEventListener("keydown", onKey);

    return () => {
      cancelled = true;
      window.removeEventListener("keydown", onKey);
      for (const unlisten of unlistens) {
        unlisten();
      }
    };
  });
</script>

<div class="shell">
  {#if showHeader}
    <header>
      <h1>mqx</h1>
      <div class="session" aria-live="polite">
        <span class="dot {sessionStatus.status}" title={statusLabel(sessionStatus.status)}></span>
        <span class="status">{statusLabel(sessionStatus.status)}</span>
        {#if sessionStatus.broker}
          <span class="broker">{sessionStatus.broker}</span>
        {/if}
        {#if stats}
          <span class="stats">{stats.topics} topics · {formatRate(stats.messagesPerSec)}</span>
          {#if storeUsage}
            <span class="stats">{storeUsage}</span>
          {/if}
        {/if}
        {#if replaying && tab === "explorer" && playback}
          <button type="button" class="replay-label" onclick={() => selectTab("playback")}>
            Replaying {playback.file}
            <span class="replay-time">
              {formatClock(replaySpan.elapsed)} / {formatClock(replaySpan.duration)}
            </span>
          </button>
          <button type="button" class="replay-stop" disabled={busy} onclick={() => void stopPlayback()}>
            Stop
          </button>
        {/if}
        {#if sessionStatus.error}
          <span class="session-error">{sessionStatus.error}</span>
        {/if}
        {#if recording && recordStatus}
          <span class="recording-pill" title="Recording to disk">
            <span class="rec-dot">●</span>
            Recording
            {formatElapsed(nowMs - recordStatus.startedMs)}
            {formatCount(recordStatus.messages)} msgs
            {#if recordStatus.dropped > 0}
              · {formatCount(recordStatus.dropped)} dropped
            {/if}
          </span>
        {/if}
      </div>
      <div class="actions">
        {#if sessionOpen}
          <button type="button" disabled={busy} onclick={() => void applyIngest(!ingestEnabled)}>
            {ingestEnabled ? "Detach" : "Go live"}
          </button>
          <button
            type="button"
            class:stop-rec={recording}
            disabled={busy || savePrompt !== null}
            onclick={() => void toggleRecording()}
          >
            {recording ? "Stop recording" : "Record"}
          </button>
        {/if}
        <button type="button" class="gear" aria-label="Settings" onclick={openSettings}>⚙</button>
        <button type="button" disabled={busy} onclick={() => void disconnect()}>Disconnect</button>
      </div>
    </header>
  {/if}

  <nav class="tabs" aria-label="App">
    {#if sessionOpen}
      <button
        type="button"
        class:active={tab === "explorer"}
        onclick={() => selectTab("explorer")}
      >
        Explorer
      </button>
    {:else}
      <button
        type="button"
        class:active={tab === "connections"}
        onclick={() => selectTab("connections")}
      >
        Connections
      </button>
    {/if}
    <button
      type="button"
      class:active={tab === "playback"}
      onclick={() => selectTab("playback")}
    >
      Playback
    </button>
    {#if !showHeader}
      <button type="button" class="gear" aria-label="Settings" onclick={openSettings}>⚙</button>
    {/if}
  </nav>

  <div class="chrome" inert={draft !== null}>
    {#if error}
      <p class="banner error" role="alert">{error}</p>
    {:else if notice}
      <p class="banner ok">{notice}</p>
    {/if}
    {#if sessionStatus.ramExhausted}
      <p class="banner error" role="alert">
        Topic history is at the {ramLimitLabel} GB limit. Every topic already has only its latest
        message. Raise the limit in Settings or disconnect.
      </p>
    {/if}

    {#if showPlayback}
      <Playback
        {profiles}
        lastUsedId={lastUsedId}
        {sessionOpen}
        sessionProfileId={sessionStatus.profileId}
        sessionBroker={sessionStatus.broker}
        {ingestEnabled}
        {playback}
        {busy}
        t0Ms={replayT0}
        onplay={(path, profileId) => void playRecording(path, profileId)}
        onstop={() => void stopPlayback()}
      />
    {:else if showPicker}
      <ConnectionPicker
        {profiles}
        {lastUsedName}
        {busy}
        {loaded}
        onnew={openNew}
        onedit={(id) => void openEdit(id)}
        onduplicate={(id) => void duplicate(id)}
        ondelete={(id, name) => void remove(id, name)}
        onconnect={(id) => void connect(id)}
        onsettings={openSettings}
      />
    {:else if showExplorer}
      {#key `${wantedProfileId ?? ""}:${wantedEpoch ?? sessionStatus.epoch ?? 0}`}
        <Explorer
          profileId={wantedProfileId}
          epoch={wantedEpoch ?? sessionStatus.epoch ?? null}
          status={sessionStatus.status}
        />
      {/key}
    {/if}
  </div>

  {#if draft}
    <ProfileEditor
      bind:draft
      {busy}
      error={editorError}
      oncancel={cancel}
      onsave={() => void save()}
      onsaveAndConnect={() => void saveAndConnect()}
    />
  {/if}

  {#if settingsOpen}
    <Settings
      {theme}
      {ramLimitBytes}
      {recordDirectory}
      error={settingsError}
      oncancel={closeSettings}
      ontheme={(next) => void chooseTheme(next)}
      onramlimit={(bytes) => void chooseRamLimit(bytes)}
      onrecordDirectory={(directory) => void chooseRecordDirectory(directory)}
    />
  {/if}

  {#if savePrompt}
    <div class="backdrop">
      <div class="modal" role="dialog" aria-modal="true" aria-labelledby="save-recording-title">
        <header class="modal-header">
          <h2 id="save-recording-title">Save recording</h2>
        </header>
        <form
          class="modal-body"
          onsubmit={(event) => {
            event.preventDefault();
            void confirmSave();
          }}
        >
          <label>
            Filename
            <input bind:this={saveNameInput} bind:value={savePrompt.name} spellcheck="false" />
          </label>
          <p class="modal-hint">Cancel discards the capture.</p>
          {#if savePrompt.error}
            <p class="modal-error" role="alert">{savePrompt.error}</p>
          {/if}
          <footer class="modal-footer">
            <button type="button" disabled={busy} onclick={() => void cancelSave()}>Cancel</button>
            <button type="submit" class="primary" disabled={busy}>Save</button>
          </footer>
        </form>
      </div>
    </div>
  {/if}
</div>

<style>
  .shell,
  .chrome {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .shell {
    height: 100%;
    background: var(--bg);
  }

  .chrome {
    flex: 1;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-5);
    border-bottom: 1px solid var(--border);
    background: var(--bg-elevated);
  }

  h1 {
    font-size: 1.1rem;
    font-weight: 650;
    letter-spacing: -0.02em;
  }

  .session {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    min-width: 0;
    flex: 1;
    color: var(--fg-muted);
    font-size: 0.85rem;
  }

  .dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 50%;
    background: var(--fg-faint);
    flex-shrink: 0;
  }

  .dot.connected {
    background: var(--ok);
  }

  .dot.detached,
  .dot.connecting {
    background: var(--warn);
  }

  .dot.reconnecting {
    background: var(--warn);
    animation: pulse 1.2s ease-in-out infinite;
  }

  .dot.error {
    background: var(--danger);
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.35;
    }
  }

  .broker,
  .stats {
    font-family: var(--mono);
    font-size: 12px;
  }

  .session-error {
    color: var(--danger);
    max-width: 24rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .actions {
    display: flex;
    gap: var(--space-2);
    margin-left: auto;
  }

  .tabs {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: 0 var(--space-5);
    border-bottom: 1px solid var(--border);
    background: var(--bg-elevated);
  }

  .tabs button {
    border: 0;
    background: transparent;
    padding: 0.55rem 0.7rem;
    color: var(--fg-muted);
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
    border-radius: 0;
  }

  .tabs button.active {
    color: var(--fg);
    border-bottom-color: var(--accent);
  }

  .tabs .gear {
    margin-left: auto;
    width: 2rem;
    padding: 0.35rem 0;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
  }

  .replay-label {
    display: inline-flex;
    align-items: baseline;
    gap: var(--space-2);
    border: 1px solid var(--border);
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
    padding: 0.2rem 0.55rem;
    font-size: 12px;
    font-family: var(--mono);
    color: var(--fg);
  }

  .replay-time {
    color: var(--fg-muted);
  }

  .replay-stop {
    border: 1px solid var(--danger);
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    color: var(--danger);
    border-radius: var(--radius-sm);
    padding: 0.2rem 0.55rem;
    font-size: 12px;
  }

  header button {
    border: 1px solid var(--border-strong);
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
    padding: 0.35rem 0.7rem;
  }

  header button.gear {
    width: 2rem;
    padding: 0.35rem 0;
    color: var(--fg-muted);
  }

  header button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .banner {
    margin: var(--space-3) var(--space-5) 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
  }

  .banner.error {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }

  .banner.ok {
    color: var(--ok);
    background: color-mix(in srgb, var(--ok) 12%, transparent);
  }

  .recording-pill {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0.15rem 0.55rem;
    border-radius: 999px;
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    color: var(--danger);
    font-family: var(--mono);
    font-size: 12px;
    font-weight: 600;
  }

  .rec-dot {
    color: var(--danger);
  }

  header button.stop-rec {
    border-color: var(--danger);
    color: var(--danger);
  }

  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 10;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: var(--space-6) var(--space-4);
    overflow: auto;
    background: color-mix(in srgb, #000 45%, transparent);
  }

  .modal {
    width: min(24rem, 100%);
    display: flex;
    flex-direction: column;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
  }

  .modal-header {
    padding: var(--space-4) var(--space-5) var(--space-3);
    border-bottom: 1px solid var(--border);
  }

  .modal-header h2 {
    font-size: 1.05rem;
    font-weight: 650;
    letter-spacing: -0.02em;
  }

  .modal-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5) var(--space-4);
  }

  .modal-body label {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--fg-muted);
    font-size: 12px;
    font-weight: 600;
  }

  .modal-body input {
    border: 1px solid var(--border);
    background: var(--bg-input);
    border-radius: var(--radius-sm);
    padding: 0.45rem 0.6rem;
  }

  .modal-hint {
    color: var(--fg-faint);
    font-size: 12px;
  }

  .modal-error {
    color: var(--danger);
    font-weight: 500;
  }

  .modal-footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .modal-footer button {
    border: 1px solid var(--border-strong);
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
    padding: 0.45rem 0.75rem;
  }

  .modal-footer .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-fg);
    font-weight: 600;
  }
</style>
