<script lang="ts">
  import { onMount } from "svelte";
  import { pickFile } from "./api";
  import {
    defaultPort,
    emptyLastWill,
    emptySubscription,
    errorMessage,
    generateClientId,
    type FileKind,
    type ProfileDraft,
    type Protocol,
  } from "./types";

  let {
    draft = $bindable(),
    busy = false,
    error = null,
    oncancel,
    onsave,
    onsaveAndConnect,
  }: {
    draft: ProfileDraft;
    busy?: boolean;
    error?: string | null;
    oncancel: () => void;
    onsave: () => void;
    onsaveAndConnect: () => void;
  } = $props();

  let pickError = $state<string | null>(null);
  let modalEl = $state<HTMLDivElement | null>(null);
  let formEl = $state<HTMLFormElement | null>(null);
  let nameInput = $state<HTMLInputElement | null>(null);
  let advancedOpen = $state(draft.lastWill !== null || draft.subscriptions.length > 1);
  const protocols: Protocol[] = ["mqtt", "mqtts", "ws", "wss"];
  const qosValues = [0, 1, 2];
  const primary = $derived(draft.subscriptions[0] ?? emptySubscription());
  const extraSubs = $derived(draft.subscriptions.slice(1));
  const will = $derived(draft.lastWill ?? emptyLastWill());

  function focusables(): HTMLElement[] {
    if (!modalEl) {
      return [];
    }
    return [...modalEl.querySelectorAll<HTMLElement>(
      "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled])",
    )];
  }

  function saveIfValid() {
    if (formEl && !formEl.reportValidity()) {
      return;
    }
    onsave();
  }

  function setProtocol(next: Protocol) {
    const previousDefault = defaultPort(draft.protocol);
    if (draft.port === previousDefault) {
      draft.port = defaultPort(next);
    }
    draft.protocol = next;
  }

  function regenerateClientId() {
    draft.clientId = generateClientId(draft.clientId);
  }

  function ensurePrimary() {
    if (draft.subscriptions.length === 0) {
      draft.subscriptions = [emptySubscription()];
    }
  }

  function updatePrimary(patch: Partial<{ topic: string; qos: number }>) {
    ensurePrimary();
    draft.subscriptions[0] = { ...draft.subscriptions[0], ...patch };
  }

  function addSubscription() {
    draft.subscriptions = [...draft.subscriptions, { topic: "", qos: 0 }];
  }

  function updateExtra(index: number, patch: Partial<{ topic: string; qos: number }>) {
    const next = draft.subscriptions.map((subscription, i) =>
      i === index + 1 ? { ...subscription, ...patch } : subscription,
    );
    draft.subscriptions = next;
  }

  function removeExtra(index: number) {
    draft.subscriptions = draft.subscriptions.filter((_, i) => i !== index + 1);
  }

  function toggleLastWill(enabled: boolean) {
    draft.lastWill = enabled ? (draft.lastWill ?? emptyLastWill()) : null;
  }

  function updateWill(patch: Partial<typeof will>) {
    draft.lastWill = { ...will, ...patch };
  }

  async function pick(kind: FileKind) {
    pickError = null;
    try {
      const { path } = await pickFile(kind);
      if (kind === "ca") draft.caCertPath = path;
      if (kind === "cert") draft.clientCertPath = path;
      if (kind === "key") draft.clientKeyPath = path;
    } catch (err) {
      const message = errorMessage(err);
      if (message !== "cancelled") {
        pickError = message;
      }
    }
  }

  $effect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        oncancel();
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const items = focusables();
      if (items.length === 0) {
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      if (event.shiftKey && (active === first || !modalEl?.contains(active))) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (active === last || !modalEl?.contains(active))) {
        event.preventDefault();
        first.focus();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  onMount(() => {
    nameInput?.focus();
  });
</script>

<div class="backdrop">
  <div
    class="modal"
    bind:this={modalEl}
    role="dialog"
    aria-modal="true"
    aria-labelledby="profile-editor-title"
  >
    <header>
      <h2 id="profile-editor-title">{draft.id ? "Edit connection" : "New connection"}</h2>
    </header>

    <form
      class="form"
      bind:this={formEl}
      onsubmit={(event) => {
        event.preventDefault();
        onsaveAndConnect();
      }}
    >
      <div class="body">
        <label>
          Name
          <input bind:this={nameInput} bind:value={draft.name} placeholder="Home Assistant" />
        </label>

        <fieldset class="protocols">
          <legend>Protocol</legend>
          <div class="segments" role="radiogroup" aria-label="Protocol">
            {#each protocols as protocol}
              <label class:active={draft.protocol === protocol}>
                <input
                  type="radio"
                  name="protocol"
                  value={protocol}
                  checked={draft.protocol === protocol}
                  onchange={() => setProtocol(protocol)}
                />
                {protocol}
              </label>
            {/each}
          </div>
        </fieldset>

        <div class="row">
          <label class="grow">
            Host
            <input bind:value={draft.host} required placeholder="localhost" />
          </label>
          <label class="port">
            Port
            <input type="number" min="1" max="65535" bind:value={draft.port} required />
          </label>
        </div>

        <label>
          Client ID
          <span class="file">
            <input bind:value={draft.clientId} spellcheck="false" />
            <button type="button" class="icon" aria-label="Regenerate client ID" onclick={regenerateClientId}>
              ↻
            </button>
          </span>
        </label>

        <label>
          Username
          <input bind:value={draft.username} autocomplete="off" />
        </label>

        <label>
          Password
          <input
            type="password"
            bind:value={draft.password}
            autocomplete="new-password"
            placeholder={draft.hasPassword ? "saved — leave blank to keep" : ""}
          />
          <span class="hint">stored in keychain</span>
        </label>

        <section class="block">
          <h3>TLS</h3>
          <label class="check">
            <input type="checkbox" bind:checked={draft.tlsValidate} />
            Validate certificate
          </label>
          {#if !draft.tlsValidate}
            <p class="warn" role="status">
              Certificate validation is off. The broker identity will not be checked. Only use this
              for a trusted homelab broker.
            </p>
          {/if}
          <label>
            CA
            <span class="file">
              <input bind:value={draft.caCertPath} placeholder="optional" spellcheck="false" />
              <button type="button" onclick={() => pick("ca")}>Browse</button>
            </span>
          </label>
          <label>
            Client cert
            <span class="file">
              <input bind:value={draft.clientCertPath} placeholder="optional" spellcheck="false" />
              <button type="button" onclick={() => pick("cert")}>Browse</button>
            </span>
          </label>
          <label>
            Client key
            <span class="file">
              <input bind:value={draft.clientKeyPath} placeholder="optional" spellcheck="false" />
              <button type="button" onclick={() => pick("key")}>Browse</button>
            </span>
          </label>
        </section>

        <section class="block">
          <h3>Session</h3>
          <div class="row wrap">
            <label class="check">
              <input type="checkbox" bind:checked={draft.session.clean} />
              Clean session
            </label>
            <label class="keep">
              Keepalive
              <span class="inline">
                <input type="number" min="1" bind:value={draft.session.keepAliveSecs} />
                <span class="unit">s</span>
              </span>
            </label>
          </div>
          <div class="row">
            <label class="grow">
              Subscriptions
              <input
                value={primary.topic}
                oninput={(event) => updatePrimary({ topic: event.currentTarget.value })}
                placeholder="#"
                spellcheck="false"
              />
            </label>
            <label class="qos">
              QoS
              <select
                value={primary.qos}
                onchange={(event) => updatePrimary({ qos: Number(event.currentTarget.value) })}
              >
                {#each qosValues as qos}
                  <option value={qos}>{qos}</option>
                {/each}
              </select>
            </label>
          </div>
        </section>

        <details class="advanced" bind:open={advancedOpen}>
          <summary>Advanced</summary>
          <div class="advanced-body">
            {#if extraSubs.length > 0}
              <div class="extra">
                {#each extraSubs as sub, index (index)}
                  <div class="row">
                    <label class="grow">
                      Extra subscription
                      <input
                        value={sub.topic}
                        oninput={(event) => updateExtra(index, { topic: event.currentTarget.value })}
                        spellcheck="false"
                      />
                    </label>
                    <label class="qos">
                      QoS
                      <select
                        value={sub.qos}
                        onchange={(event) =>
                          updateExtra(index, { qos: Number(event.currentTarget.value) })}
                      >
                        {#each qosValues as qos}
                          <option value={qos}>{qos}</option>
                        {/each}
                      </select>
                    </label>
                    <button type="button" class="icon remove" onclick={() => removeExtra(index)}>
                      Remove
                    </button>
                  </div>
                {/each}
              </div>
            {/if}
            <button type="button" class="link" onclick={addSubscription}>+ Add subscription</button>

            <label>
              Max packet size
              <input type="number" min="1" bind:value={draft.session.maxPacketSize} />
            </label>

            <label class="check">
              <input
                type="checkbox"
                checked={draft.lastWill !== null}
                onchange={(event) => toggleLastWill(event.currentTarget.checked)}
              />
              Last will
            </label>
            {#if draft.lastWill}
              <label>
                Topic
                <input
                  value={will.topic}
                  oninput={(event) => updateWill({ topic: event.currentTarget.value })}
                  spellcheck="false"
                />
              </label>
              <label>
                Payload
                <textarea
                  rows="3"
                  value={will.payload}
                  oninput={(event) => updateWill({ payload: event.currentTarget.value })}
                ></textarea>
              </label>
              <div class="row wrap">
                <label class="qos">
                  QoS
                  <select
                    value={will.qos}
                    onchange={(event) => updateWill({ qos: Number(event.currentTarget.value) })}
                  >
                    {#each qosValues as qos}
                      <option value={qos}>{qos}</option>
                    {/each}
                  </select>
                </label>
                <label class="check">
                  <input
                    type="checkbox"
                    checked={will.retain}
                    onchange={(event) => updateWill({ retain: event.currentTarget.checked })}
                  />
                  Retain
                </label>
              </div>
            {/if}
          </div>
        </details>

      </div>

      {#if error || pickError}
        <p class="error" role="alert">{error ?? pickError}</p>
      {/if}

      <footer>
        <button type="button" disabled={busy} onclick={oncancel}>Cancel</button>
        <button type="button" disabled={busy} onclick={saveIfValid}>Save</button>
        <button type="submit" class="primary" disabled={busy}>Save & Connect</button>
      </footer>
    </form>
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
    width: min(36rem, 100%);
    max-height: calc(100vh - 3rem);
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

  .form {
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .body {
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5);
  }

  label,
  .block,
  .advanced {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--fg-muted);
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.02em;
  }

  .block,
  .advanced {
    gap: var(--space-3);
    margin-top: var(--space-1);
    padding-top: var(--space-3);
    border-top: 1px solid var(--border);
  }

  h3,
  legend,
  summary {
    margin: 0;
    color: var(--fg-faint);
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .protocols {
    border: 0;
    padding: 0;
    margin: 0;
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
    letter-spacing: 0;
    text-transform: none;
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

  input,
  select,
  textarea {
    width: 100%;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0.45rem 0.6rem;
    color: var(--fg);
    font-weight: 500;
    letter-spacing: 0;
  }

  textarea {
    resize: vertical;
    min-height: 4.5rem;
    font-family: var(--mono);
  }

  input:focus,
  select:focus,
  textarea:focus {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .row {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
  }

  .row.wrap {
    flex-wrap: wrap;
    align-items: center;
  }

  .grow {
    flex: 1;
  }

  .port,
  .qos {
    width: 5.5rem;
  }

  .keep {
    width: 8.5rem;
  }

  .file,
  .inline {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .unit {
    color: var(--fg-muted);
    font-weight: 600;
  }

  .check {
    flex-direction: row;
    align-items: center;
    gap: var(--space-2);
    color: var(--fg);
    font-size: 13px;
    font-weight: 500;
    letter-spacing: 0;
    text-transform: none;
  }

  .check input {
    width: auto;
    accent-color: var(--accent);
  }

  .hint {
    color: var(--fg-faint);
    font-weight: 500;
    letter-spacing: 0;
  }

  .warn {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    color: #f0c14b;
    background: color-mix(in srgb, #f0c14b 14%, transparent);
    font-weight: 500;
    letter-spacing: 0;
  }

  .advanced-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .extra {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .link {
    align-self: flex-start;
    border: 0;
    background: transparent;
    color: var(--accent);
    padding: 0;
    font-weight: 600;
  }

  .icon {
    flex: 0 0 auto;
    border: 1px solid var(--border-strong);
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
    padding: 0.45rem 0.65rem;
  }

  .remove {
    margin-bottom: 1px;
    color: var(--fg-muted);
    font-size: 12px;
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

  footer button,
  .file button {
    border: 1px solid var(--border-strong);
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
    padding: 0.45rem 0.75rem;
  }

  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-fg);
    font-weight: 600;
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
