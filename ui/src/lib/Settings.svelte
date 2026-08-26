<script lang="ts">
  import { onMount } from "svelte";
  import { THEMES, type Theme } from "./theme";

  let {
    theme,
    busy = false,
    error = null,
    oncancel,
    ontheme,
  }: {
    theme: Theme;
    busy?: boolean;
    error?: string | null;
    oncancel: () => void;
    ontheme: (theme: Theme) => void;
  } = $props();

  let dialog = $state<HTMLDivElement | null>(null);

  onMount(() => {
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
    </div>
    {#if error}
      <p class="error" role="alert">{error}</p>
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
    width: min(24rem, 100%);
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

  .themes {
    border: 0;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
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
