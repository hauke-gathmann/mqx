<script lang="ts">
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import {
    connect as connectSession,
    deleteProfile,
    disconnect as disconnectSession,
    getProfile,
    getSettings,
    listProfiles,
    saveProfile,
    setRamLimit,
    setTheme,
  } from "./lib/api";
  import ConnectionPicker from "./lib/ConnectionPicker.svelte";
  import Explorer from "./lib/Explorer.svelte";
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
    formatRate,
    formatStoreUsage,
    generateClientId,
    ramLimitGb,
    getLastUsedProfileId,
    idleSessionStatus,
    setLastUsedProfileId,
    type ProfileDraft,
    type ProfileSummary,
    type SessionStats,
    type SessionStatus,
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

  applyTheme(readCachedTheme());

  const live = $derived(
    sessionStatus.status === "connecting" ||
      sessionStatus.status === "connected" ||
      sessionStatus.status === "reconnecting",
  );
  const showPicker = $derived(!live && sessionStatus.status !== "error");

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

  async function chooseTheme(next: Theme) {
    theme = next;
    applyTheme(next);
    try {
      const settings = await setTheme(next);
      if (isTheme(settings.theme)) {
        theme = settings.theme;
        applyTheme(settings.theme);
      }
      if (typeof settings.ramLimitBytes === "number") {
        ramLimitBytes = settings.ramLimitBytes;
      }
    } catch (err) {
      settingsError = errorMessage(err);
    }
  }

  async function chooseRamLimit(bytes: number) {
    ramLimitBytes = bytes;
    try {
      const settings = await setRamLimit(bytes);
      if (typeof settings.ramLimitBytes === "number") {
        ramLimitBytes = settings.ramLimitBytes;
      }
    } catch (err) {
      settingsError = errorMessage(err);
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
    wantedProfileId = null;
    wantedEpoch = null;
    busy = true;
    try {
      await disconnectSession();
      sessionStatus = idleSessionStatus();
      stats = null;
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
          const payload = event.payload;
          if (payload.profileId !== wantedProfileId) {
            return;
          }
          if (wantedEpoch != null && payload.epoch !== wantedEpoch) {
            return;
          }
          sessionStatus = payload;
          if (payload.status === "disconnected" || payload.status === "error") {
            stats = null;
          }
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
          settingsErrorUnlisten,
        );
      } catch (err) {
        error = errorMessage(err);
      }

      try {
        const settings = await getSettings();
        if (isTheme(settings.theme)) {
          theme = settings.theme;
          applyTheme(settings.theme);
        }
        if (typeof settings.ramLimitBytes === "number") {
          ramLimitBytes = settings.ramLimitBytes;
        }
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
      if ((event.metaKey || event.ctrlKey) && event.key === ",") {
        event.preventDefault();
        openSettings();
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
  {#if live || sessionStatus.status === "error"}
    <header>
      <h1>mqx</h1>
      <div class="session" aria-live="polite">
        <span class="dot {sessionStatus.status}" title={sessionStatus.status}></span>
        <span class="status">{sessionStatus.status}</span>
        {#if sessionStatus.broker}
          <span class="broker">{sessionStatus.broker}</span>
        {/if}
        {#if stats}
          <span class="stats">{stats.topics} topics · {formatRate(stats.messagesPerSec)}</span>
          {#if storeUsage}
            <span class="stats">{storeUsage}</span>
          {/if}
        {/if}
        {#if sessionStatus.error}
          <span class="session-error">{sessionStatus.error}</span>
        {/if}
      </div>
      <div class="actions">
        <button type="button" class="gear" aria-label="Settings" onclick={openSettings}>⚙</button>
        <button type="button" disabled={busy} onclick={() => void disconnect()}>Disconnect</button>
      </div>
    </header>
  {/if}

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

    {#if showPicker}
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
    {:else if live}
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
      error={settingsError}
      oncancel={closeSettings}
      ontheme={(next) => void chooseTheme(next)}
      onramlimit={(bytes) => void chooseRamLimit(bytes)}
    />
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

  .dot.connecting,
  .dot.reconnecting {
    background: var(--warn);
  }

  .dot.error {
    background: var(--danger);
  }

  .status {
    text-transform: lowercase;
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
</style>
