<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { checkForUpdates, pickFolder } from "./api";
  import { errorMessage } from "./types";
  import { THEMES, type Theme } from "./theme";
  import { RAM_LIMIT_MAX_GB, RAM_LIMIT_MIN_GB, ramLimitGb } from "./types";

  const GIB = 1024 * 1024 * 1024;

  let {
    theme,
    ramLimitBytes,
    recordDirectory,
    busy = false,
    error = null,
    oncancel,
    ontheme,
    onramlimit,
    onrecordDirectory,
  }: {
    theme: Theme;
    ramLimitBytes: number;
    recordDirectory: string;
    busy?: boolean;
    error?: string | null;
    oncancel: () => void;
    ontheme: (theme: Theme) => void;
    onramlimit: (bytes: number) => void;
    onrecordDirectory: (directory: string) => void;
  } = $props();

  let ramGb = $state(12);
  let dialog = $state<HTMLDivElement | null>(null);
  let directory = $state("");
  let pickError = $state<string | null>(null);
  let version = $state("");
  let checking = $state(false);

  $effect(() => {
    directory = recordDirectory;
  });

  function persistDirectory() {
    onrecordDirectory(directory.trim());
  }

  async function browse() {
    pickError = null;
    try {
      const { path } = await pickFolder();
      directory = path;
      onrecordDirectory(path);
    } catch (err) {
      if (errorMessage(err) !== "cancelled") {
        pickError = errorMessage(err);
      }
    }
  }

  $effect(() => {
    ramGb = Math.min(RAM_LIMIT_MAX_GB, Math.max(RAM_LIMIT_MIN_GB, ramLimitGb(ramLimitBytes)));
  });

  async function checkUpdates() {
    if (checking) {
      return;
    }
    pickError = null;
    checking = true;
    try {
      await checkForUpdates();
    } catch (err) {
      pickError = errorMessage(err);
    } finally {
      checking = false;
    }
  }

  onMount(() => {
    void getVersion()
      .then((value) => {
        version = value;
      })
      .catch((err) => {
        pickError = errorMessage(err);
      });
    const first = dialog?.querySelector<HTMLElement>("button, input");
    first?.focus();
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        oncancel();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
</script>

<div class="backdrop">
  <div
    bind:this={dialog}
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-labelledby="settings-title"
  >
    <header>
      <h2 id="settings-title">Settings</h2>
    </header>
    <div class="body">
      <fieldset class="themes">
        <legend>Theme</legend>
        <div class="segments">
          {#each THEMES as option}
            <label class:active={theme === option}>
              <input
                type="radio"
                name="theme"
                value={option}
                checked={theme === option}
                disabled={busy}
                onchange={() => ontheme(option)}
              />
              {option}
            </label>
          {/each}
        </div>
      </fieldset>
      <fieldset class="ram">
        <legend>Topic history limit</legend>
        <div class="slider-row">
          <input
            type="range"
            min={RAM_LIMIT_MIN_GB}
            max={RAM_LIMIT_MAX_GB}
            step="1"
            value={ramGb}
            disabled={busy}
            oninput={(event) => {
              ramGb = Number(event.currentTarget.value);
            }}
            onchange={() => onramlimit(ramGb * GIB)}
          />
          <span class="ram-value">{ramGb} GB</span>
        </div>
        <p class="hint">
          Drops older messages when the in-memory store exceeds this. Each topic keeps at least its
          latest payload. This is not the whole app’s RAM (the window uses extra).
        </p>
      </fieldset>
      <label class="directory">
        Recordings folder
        <span class="file">
          <input
            bind:value={directory}
            spellcheck="false"
            disabled={busy}
            placeholder="app data / recordings"
            onblur={persistDirectory}
            onchange={persistDirectory}
          />
          <button type="button" disabled={busy} onclick={() => void browse()}>Browse</button>
        </span>
        <span class="hint">Empty uses the default. Captures include raw payloads.</span>
      </label>
      <fieldset class="updates">
        <legend>Updates</legend>
        {#if version}
          <p class="version">Version {version}</p>
        {/if}
        <button type="button" disabled={busy || checking} onclick={() => void checkUpdates()}>
          Check for Updates…
        </button>
        <p class="hint">
          Unsigned builds or a missing latest.json fail with the existing dialog.
        </p>
      </fieldset>
    </div>
    {#if pickError || error}
      <p class="error" role="alert">{pickError ?? error}</p>
    {/if}
    <footer>
      <button type="button" onclick={oncancel}>Close</button>
    </footer>
  </div>
</div>

<style>
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
    width: min(32rem, 100%);
    display: flex;
    flex-direction: column;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
  }

  header {
    padding: var(--space-4) var(--space-5) var(--space-3);
    border-bottom: 1px solid var(--border);
  }

  h2 {
    font-size: 1.05rem;
    font-weight: 650;
    letter-spacing: -0.02em;
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5);
  }

  .themes,
  .ram,
  .updates {
    border: 0;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .version {
    margin: 0;
    color: var(--fg);
    font-size: 13px;
    font-weight: 500;
  }

  .updates button {
    align-self: flex-start;
    border: 1px solid var(--border-strong);
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
    padding: 0.4rem 0.7rem;
    font-size: 13px;
    font-weight: 600;
    color: var(--fg);
  }

  .slider-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .slider-row input[type="range"] {
    flex: 1;
    min-width: 0;
  }

  .ram-value {
    flex-shrink: 0;
    min-width: 3.5rem;
    font-variant-numeric: tabular-nums;
    font-weight: 600;
  }

  .hint {
    margin: 0;
    color: var(--fg-muted);
    font-size: 0.8rem;
    line-height: 1.4;
  }

  legend {
    margin: 0 0 var(--space-1);
    color: var(--fg-faint);
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .segments {
    display: flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow: hidden;
    background: var(--bg-input);
  }

  .segments label {
    flex: 1;
    margin: 0;
    padding: 0.4rem 0.2rem;
    text-align: center;
    color: var(--fg-muted);
    font-size: 13px;
    font-weight: 600;
    text-transform: capitalize;
    cursor: pointer;
  }

  .segments label + label {
    border-left: 1px solid var(--border);
  }

  .segments label.active {
    background: var(--accent);
    color: var(--accent-fg);
  }

  .segments input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .directory {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--fg-faint);
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .file {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .file input {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--border);
    background: var(--bg-input);
    border-radius: var(--radius-sm);
    padding: 0.4rem 0.55rem;
    color: var(--fg);
    font-size: 13px;
    font-weight: 500;
    letter-spacing: 0;
    text-transform: none;
  }

  .file button {
    border: 1px solid var(--border-strong);
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
    padding: 0.4rem 0.7rem;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0;
    text-transform: none;
    color: var(--fg);
  }

  .hint {
    color: var(--fg-faint);
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0;
    text-transform: none;
  }

  .error {
    margin: 0;
    padding: 0 var(--space-5) var(--space-2);
    color: var(--danger);
    font-weight: 500;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5) var(--space-4);
    border-top: 1px solid var(--border);
  }

  footer button {
    border: 1px solid var(--border-strong);
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
    padding: 0.45rem 0.75rem;
  }
</style>
