<script lang="ts">
  import { createVirtualizer } from "@tanstack/svelte-virtual";
  import { get } from "svelte/store";
  import { ROW_HEIGHT, highlightsForPath, liveFreshness, type TreeRow } from "./treeState";
  import type { SearchHit, SearchMode } from "./types";

  let {
    rows,
    selected,
    expanded,
    query = $bindable(""),
    mode = $bindable("keep"),
    hits = null,
    now,
    emptyKind = "idle",
    emptyTitle = "No topics yet",
    emptyDetail = "Messages show up as they arrive.",
    searchEl = $bindable(null),
    onapply,
    onselect,
    ontoggle,
    onfocuspane,
  }: {
    rows: TreeRow[];
    selected: string | null;
    expanded: Set<string>;
    query?: string;
    mode?: SearchMode;
    hits?: SearchHit[] | null;
    now: number;
    emptyKind?: "connecting" | "search" | "idle";
    emptyTitle?: string;
    emptyDetail?: string;
    searchEl?: HTMLInputElement | null;
    onapply: () => void;
    onselect: (path: string) => void;
    ontoggle: (path: string) => void;
    onfocuspane: () => void;
  } = $props();

  let scrollEl = $state<HTMLDivElement | undefined>();

  const virtualizer = createVirtualizer<HTMLDivElement, HTMLDivElement>({
    count: 0,
    getScrollElement: () => scrollEl ?? null,
    estimateSize: () => ROW_HEIGHT,
    overscan: 16,
  });

  $effect(() => {
    const count = rows.length;
    const el = scrollEl;
    get(virtualizer).setOptions({
      count,
      getScrollElement: () => el ?? null,
    });
  });

  function chars(segment: string, marks: number[]) {
    const on = new Set(marks);
    return [...segment].map((ch, index) => ({ ch, on: on.has(index) }));
  }

  function onRowClick(row: TreeRow) {
    if (row.node.hasPayload) {
      onselect(row.path);
      return;
    }
    if (row.node.childCount > 0) {
      ontoggle(row.path);
    }
  }

  function onSearchKey(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      onapply();
    }
  }

  function onRowKey(event: KeyboardEvent, row: TreeRow) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onRowClick(row);
    } else if (event.key === "ArrowRight" && row.node.childCount > 0 && !expanded.has(row.path)) {
      event.preventDefault();
      ontoggle(row.path);
    } else if (event.key === "ArrowLeft" && expanded.has(row.path)) {
      event.preventDefault();
      ontoggle(row.path);
    }
  }
</script>

<div class="pane" onfocusin={onfocuspane}>
  <div class="search">
    <span class="icon" aria-hidden="true">🔍</span>
    <input
      bind:this={searchEl}
      bind:value={query}
      type="search"
      spellcheck="false"
      autocomplete="off"
      placeholder={mode === "skip"
        ? "Hide topics containing… (Enter)"
        : "Filter topics containing… (Enter)"}
      aria-label="Search topics"
      onkeydown={onSearchKey}
    />
    <div class="modes" role="group" aria-label="Search mode">
      <button type="button" class:active={mode === "keep"} onclick={() => (mode = "keep")}>
        keep
      </button>
      <button type="button" class:active={mode === "skip"} onclick={() => (mode = "skip")}>
        skip
      </button>
    </div>
  </div>

  <div class="scroll" bind:this={scrollEl} role="tree" aria-label="Topic tree">
    <div class="inner" style="height: {$virtualizer.getTotalSize()}px">
      {#each $virtualizer.getVirtualItems() as vRow (rows[vRow.index]?.path ?? vRow.index)}
        {@const row = rows[vRow.index]}
        {#if row}
          {@const freshness = liveFreshness(row.node, now)}
          {@const open = expanded.has(row.path)}
          {@const marks = hits ? highlightsForPath(row.path, hits) : []}
          <div
            class="row {freshness}"
            class:selected={selected === row.path}
            class:payload={row.node.hasPayload}
            role="treeitem"
            aria-selected={selected === row.path}
            aria-expanded={row.node.childCount > 0 ? open : undefined}
            tabindex="0"
            style="transform: translateY({vRow.start}px); height: {vRow.size}px; padding-left: {8 +
              row.depth * 14}px"
            onclick={() => onRowClick(row)}
            onkeydown={(event) => onRowKey(event, row)}
          >
            {#if row.node.childCount > 0}
              <button
                type="button"
                class="chevron"
                tabindex="-1"
                aria-label={open ? "Collapse" : "Expand"}
                onclick={(event) => {
                  event.stopPropagation();
                  ontoggle(row.path);
                }}
              >
                {open ? "▾" : "▸"}
              </button>
            {:else}
              <span class="chevron spacer"></span>
            {/if}
            <span class="label">
              {#each chars(row.node.segment, marks) as part, i (i)}
                {#if part.on}<mark>{part.ch}</mark>{:else}{part.ch}{/if}
              {/each}
            </span>
            {#if row.node.hasPayload && row.node.format === "json"}
              <span class="badge">JSON</span>
            {/if}
            {#if row.node.retain}
              <span class="badge retain">retain</span>
            {/if}
            {#if !open && row.node.childCount > 0}
              <span class="count">{row.node.childCount}</span>
            {/if}
          </div>
        {/if}
      {/each}
    </div>
    {#if rows.length === 0}
      <div class="empty" data-kind={emptyKind}>
        {#if emptyKind === "connecting"}
          <span class="pulse" aria-hidden="true"></span>
        {/if}
        <p class="empty-title">{emptyTitle}</p>
        <p class="empty-detail">{emptyDetail}</p>
      </div>
    {/if}
  </div>
</div>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    height: 100%;
  }

  .search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .icon {
    font-size: 12px;
    opacity: 0.8;
  }

  input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    outline: none;
    padding: 0.2rem 0;
  }

  .modes {
    display: flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow: hidden;
    flex-shrink: 0;
  }

  .modes button {
    border: 0;
    background: transparent;
    color: var(--fg-muted);
    padding: 0.15rem 0.45rem;
    font-size: 11px;
    text-transform: lowercase;
  }

  .modes button.active {
    background: var(--bg-active);
    color: var(--fg);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    position: relative;
  }

  .inner {
    position: relative;
    width: 100%;
  }

  .row {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding-right: var(--space-3);
    cursor: default;
    white-space: nowrap;
    user-select: none;
  }

  .row:hover,
  .row:focus-visible {
    background: var(--bg-hover);
    outline: none;
  }

  .row.selected {
    background: var(--bg-active);
  }

  .row.payload {
    cursor: pointer;
  }

  .chevron {
    width: 1.1rem;
    flex-shrink: 0;
    border: 0;
    background: transparent;
    color: var(--fg-faint);
    padding: 0;
    text-align: center;
    line-height: 1;
  }

  .chevron.spacer {
    display: inline-block;
  }

  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--mono);
    font-size: 12.5px;
  }

  .row.retain .label {
    color: var(--retain);
  }

  .row.fresh .label {
    color: var(--fg);
  }

  .row.intime .label {
    color: var(--fg-muted);
  }

  .row.stale .label {
    color: var(--fg-faint);
  }

  mark {
    background: color-mix(in srgb, var(--accent) 35%, transparent);
    color: inherit;
    border-radius: 2px;
    padding: 0;
  }

  .badge {
    font-size: 10px;
    letter-spacing: 0.04em;
    color: var(--fg-muted);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 0.3rem;
    line-height: 1.4;
  }

  .badge.retain {
    color: var(--retain);
    border-color: color-mix(in srgb, var(--retain) 40%, var(--border));
  }

  .count {
    margin-left: auto;
    color: var(--fg-faint);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }

  .empty {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-6);
    text-align: center;
  }

  .empty-title {
    margin: 0;
    color: var(--fg);
    font-weight: 600;
  }

  .empty-detail {
    margin: 0;
    max-width: 18rem;
    color: var(--fg-muted);
    font-size: 13px;
  }

  .pulse {
    width: 0.65rem;
    height: 0.65rem;
    border-radius: 50%;
    background: var(--warn);
    animation: pulse 1.2s ease-in-out infinite;
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 0.35;
      transform: scale(0.9);
    }
    50% {
      opacity: 1;
      transform: scale(1.15);
    }
  }
</style>
