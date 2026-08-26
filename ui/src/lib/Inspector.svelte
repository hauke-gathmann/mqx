<script lang="ts">
  import { onMount } from "svelte";
  import { applyJq, jqHistory } from "./api";
  import { writeClipboard } from "./clipboard";
  import PayloadView from "./PayloadView.svelte";
  import { isTypingTarget, liveFreshness } from "./treeState";
  import {
    errorMessage,
    type HistoryMeta,
    type JqApplyResult,
    type JqError,
    type MessageDto,
    type TreeNodeDto,
  } from "./types";

  let {
    topic,
    message,
    meta,
    historyIndex,
    node = null,
    now,
    onselectindex,
    onfocuspane,
  }: {
    topic: string | null;
    message: MessageDto | null;
    meta: HistoryMeta | null;
    historyIndex: number | null;
    node?: TreeNodeDto | null;
    now: number;
    onselectindex: (index: number) => void;
    onfocuspane: () => void;
  } = $props();

  let draft = $state("");
  let appliedFilter = $state<string | null>(null);
  let jqResult = $state<JqApplyResult | null>(null);
  let jqErrors = $state<JqError[]>([]);
  let suggestions = $state<string[]>([]);
  let suggestionIndex = $state(-1);
  let jqInput = $state<HTMLInputElement | null>(null);
  let copied = $state<"topic" | "payload" | null>(null);
  let copyTimer: ReturnType<typeof setTimeout> | undefined;

  const shownText = $derived(
    jqResult && jqErrors.length === 0 ? jqResult.text : (message?.payloadText ?? ""),
  );
  const shownJson = $derived(
    jqResult && jqErrors.length === 0 ? jqResult.json : message?.payloadJson,
  );
  const jsonMode = $derived(shownJson !== undefined);
  const freshness = $derived(node ? liveFreshness(node, now) : null);
  const followingLatest = $derived(
    meta != null && historyIndex != null && historyIndex === meta.latestIndex,
  );

  onMount(() => {
    if (!topic) {
      return;
    }
    let cancelled = false;
    void jqHistory(topic).then((items) => {
      if (!cancelled) {
        suggestions = items;
      }
    });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    const filter = appliedFilter;
    const currentTopic = topic;
    const index = historyIndex;
    const stamp = message?.timestamp;
    if (!filter || !currentTopic || message == null) {
      return;
    }
    void stamp;
    let cancelled = false;
    void applyJq(currentTopic, index, filter)
      .then((result) => {
        if (cancelled) {
          return;
        }
        jqErrors = result.errors;
        jqResult = result.errors.length === 0 ? result : null;
        if (result.errors.length === 0) {
          return jqHistory(currentTopic);
        }
        return null;
      })
      .then((items) => {
        if (!cancelled && items) {
          suggestions = items;
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          jqResult = null;
          jqErrors = [{ message: errorMessage(err), start: 0, end: 0 }];
        }
      });
    return () => {
      cancelled = true;
    };
  });

  function applyDraft() {
    const filter = draft.trim();
    appliedFilter = filter || null;
    if (!appliedFilter) {
      jqResult = null;
      jqErrors = [];
    }
  }

  function clearJq() {
    draft = "";
    appliedFilter = null;
    jqResult = null;
    jqErrors = [];
    suggestionIndex = -1;
  }

  function onJqKey(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      applyDraft();
    } else if (event.key === "Escape") {
      event.preventDefault();
      clearJq();
    } else if (event.key === "ArrowUp" && suggestions.length > 0) {
      event.preventDefault();
      suggestionIndex =
        suggestionIndex < 0
          ? 0
          : Math.min(suggestions.length - 1, suggestionIndex + 1);
      draft = suggestions[suggestionIndex] ?? draft;
    } else if (event.key === "ArrowDown" && suggestions.length > 0) {
      event.preventDefault();
      if (suggestionIndex <= 0) {
        suggestionIndex = -1;
        draft = appliedFilter ?? "";
      } else {
        suggestionIndex -= 1;
        draft = suggestions[suggestionIndex] ?? draft;
      }
    }
  }

  function flashCopied(kind: "topic" | "payload") {
    copied = kind;
    clearTimeout(copyTimer);
    copyTimer = setTimeout(() => {
      copied = null;
    }, 1200);
  }

  function copyTopic(event: PointerEvent) {
    event.preventDefault();
    event.stopPropagation();
    const text = topic;
    if (!text) {
      return;
    }
    if (writeClipboard(text)) {
      flashCopied("topic");
    }
  }

  function copyPayload(event: PointerEvent) {
    event.preventDefault();
    event.stopPropagation();
    const text = shownText;
    if (!text) {
      return;
    }
    if (writeClipboard(text)) {
      flashCopied("payload");
    }
  }

  function onCopy(event: ClipboardEvent) {
    if (isTypingTarget(event.target)) {
      return;
    }
    const origin = event.target;
    if (!(origin instanceof Element) || !origin.closest(".payload")) {
      return;
    }
    const selectedText = window.getSelection()?.toString();
    if (selectedText) {
      return;
    }
    if (shownText) {
      event.preventDefault();
      writeClipboard(shownText);
    }
  }

  const MAX_DOTS = 48;
  const dots = $derived.by(() => {
    if (!meta) {
      return [] as number[];
    }
    if (meta.count <= MAX_DOTS) {
      return Array.from({ length: meta.count }, (_, i) => i);
    }
    const current = historyIndex ?? meta.latestIndex;
    const half = Math.floor(MAX_DOTS / 2);
    let start = Math.max(0, current - half);
    const end = Math.min(meta.count, start + MAX_DOTS);
    start = Math.max(0, end - MAX_DOTS);
    return Array.from({ length: end - start }, (_, i) => start + i);
  });

</script>

<div class="inspector" onfocusin={onfocuspane}>
  <header>
    {#if topic}
      <h2 title={topic}>{topic}</h2>
      <div class="meta">
        {#if message?.format === "json" || jsonMode}
          <span class="badge">JSON</span>
        {:else if message}
          <span class="badge">{message.format}</span>
        {/if}
        {#if freshness}
          <span class="dot {freshness}" title={freshness}></span>
        {/if}
        <button
          type="button"
          class="copy"
          class:done={copied === "topic"}
          onpointerdown={copyTopic}
        >
          <span class="idle">Copy topic</span>
          <span class="done-label">Copied!</span>
        </button>
        <button
          type="button"
          class="copy"
          class:done={copied === "payload"}
          disabled={!shownText}
          onpointerdown={copyPayload}
        >
          <span class="idle">Copy payload</span>
          <span class="done-label">Copied!</span>
        </button>
      </div>
    {:else}
      <h2 class="muted">Select a topic</h2>
    {/if}
  </header>

  {#if message?.error}
    <p class="banner" role="alert">{message.error}</p>
  {/if}

  <div class="payload" oncopy={onCopy}>
    {#if message}
      <PayloadView doc={shownText} {jsonMode} />
    {:else}
      <p class="empty">Select a topic with a payload to inspect it.</p>
    {/if}
  </div>

  <div class="dock">
    <div class="jq">
      <span class="prompt-label">jq</span>
      <span class="prompt" aria-hidden="true">❯</span>
      <input
        bind:this={jqInput}
        bind:value={draft}
        type="text"
        spellcheck="false"
        autocomplete="off"
        placeholder=".bri"
        aria-label="jq filter"
        disabled={!message}
        onkeydown={onJqKey}
      />
    </div>
    {#if jqErrors.length > 0}
      <ul class="jq-errors" role="alert">
        {#each jqErrors as err, i (i)}
          <li>{err.message}</li>
        {/each}
      </ul>
    {/if}
    {#if meta && meta.count > 0 && historyIndex != null && message?.topic === topic}
      <div class="history">
        <div class="dots" role="listbox" aria-label="Message history">
          {#each dots as index (index)}
            <button
              type="button"
              class="crumb"
              class:current={index === historyIndex}
              role="option"
              aria-selected={index === historyIndex}
              aria-label="Message {index + 1} of {meta.count}"
              onclick={() => onselectindex(index)}
            ></button>
          {/each}
        </div>
        <span class="pos">
          {followingLatest ? "latest" : historyIndex + 1} · {historyIndex + 1}/{meta.count}
        </span>
      </div>
    {/if}
  </div>
</div>

<style>
  .inspector {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    flex: 1;
    height: 100%;
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--border);
    min-height: 2.4rem;
    flex-shrink: 0;
  }

  h2 {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    font-weight: 600;
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  h2.muted {
    color: var(--fg-muted);
    font-family: var(--font);
    font-weight: 500;
  }

  .meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .copy {
    display: inline-grid;
    place-items: center;
    border: 1px solid var(--border-strong);
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
    padding: 0.2rem 0.5rem;
    font-size: 11px;
    color: var(--fg-muted);
    white-space: nowrap;
    text-align: center;
  }

  .copy .idle,
  .copy .done-label {
    grid-area: 1 / 1;
  }

  .copy .done-label {
    visibility: hidden;
  }

  .copy.done .idle {
    visibility: hidden;
  }

  .copy.done .done-label {
    visibility: visible;
  }

  .copy:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .badge {
    font-size: 10px;
    letter-spacing: 0.04em;
    color: var(--fg-muted);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 0.35rem;
    text-transform: uppercase;
  }

  .dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 50%;
    background: var(--fg-faint);
  }

  .dot.retain {
    background: var(--retain);
  }

  .dot.fresh {
    background: var(--fg);
  }

  .dot.intime {
    background: var(--fg-muted);
  }

  .dot.stale {
    background: var(--fg-faint);
  }

  .banner {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    font-size: 13px;
    flex-shrink: 0;
  }

  .payload {
    flex: 1;
    min-height: 0;
    position: relative;
  }

  .empty {
    margin: 0;
    padding: var(--space-5);
    color: var(--fg-muted);
  }

  .dock {
    flex-shrink: 0;
    border-top: 1px solid var(--border);
    background: var(--bg-elevated);
  }

  .jq {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
  }

  .prompt-label {
    font-size: 11px;
    font-weight: 650;
    color: var(--fg-muted);
    text-transform: lowercase;
  }

  .prompt {
    color: var(--accent);
    font-family: var(--mono);
  }

  .jq input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    outline: none;
    font-family: var(--mono);
    font-size: 13px;
  }

  .jq-errors {
    list-style: none;
    margin: 0;
    padding: 0 var(--space-3) var(--space-2);
    color: var(--danger);
    font-size: 12px;
    font-family: var(--mono);
  }

  .history {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-3) var(--space-2);
  }

  .dots {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    flex: 1;
    min-width: 0;
  }

  .crumb {
    width: 0.5rem;
    height: 0.5rem;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: var(--fg-faint);
    opacity: 0.55;
  }

  .crumb.current {
    background: var(--accent);
    opacity: 1;
    transform: scale(1.25);
  }

  .pos {
    color: var(--fg-muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
</style>
