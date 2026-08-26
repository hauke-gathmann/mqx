<script lang="ts">
  import { tick } from "svelte";
  import type { HistoryItem } from "./types";

  let {
    topic,
    items,
    selectedIndex,
    onselectindex,
    onfocuspane,
  }: {
    topic: string | null;
    items: HistoryItem[];
    selectedIndex: number | null;
    onselectindex: (index: number) => void;
    onfocuspane: () => void;
  } = $props();

  let listEl = $state<HTMLDivElement | null>(null);
  let prevTopic: string | null = null;
  let prevIndex: number | null = null;
  let prevCount = 0;

  const rows = $derived([...items].reverse());
  const latestIndex = $derived(items.length > 0 ? items[items.length - 1].index : null);
  const followingLatest = $derived(
    selectedIndex != null && latestIndex != null && selectedIndex === latestIndex,
  );

  $effect(() => {
    const currentTopic = topic;
    const index = selectedIndex;
    const count = items.length;
    const following =
      index != null && items.length > 0 && index === items[items.length - 1]?.index;
    const el = listEl;

    if (currentTopic !== prevTopic) {
      prevTopic = currentTopic;
      prevIndex = null;
      prevCount = 0;
    }

    const grew = count > prevCount;
    const added = count - prevCount;
    const indexChanged = index !== prevIndex;
    prevIndex = index;
    prevCount = count;

    if (!el) {
      return;
    }

    if (following) {
      void tick().then(() => {
        if (listEl) {
          listEl.scrollTop = 0;
        }
      });
      return;
    }

    if (grew && added > 0) {
      void tick().then(() => {
        const node = listEl;
        if (!node) {
          return;
        }
        const nextRows = node.querySelectorAll(".row");
        let delta = 0;
        for (let i = 0; i < added && i < nextRows.length; i += 1) {
          delta += (nextRows[i] as HTMLElement).offsetHeight;
        }
        node.scrollTop += delta;
      });
      return;
    }

    if (indexChanged) {
      void tick().then(() => {
        listEl
          ?.querySelector<HTMLElement>(".row.current")
          ?.scrollIntoView({ block: "nearest" });
      });
    }
  });

  function jumpToLatest() {
    if (latestIndex == null) {
      return;
    }
    if (selectedIndex === latestIndex) {
      if (listEl) {
        listEl.scrollTop = 0;
      }
      return;
    }
    onselectindex(latestIndex);
  }

  function formatArrival(ms: number): string {
    const date = new Date(ms);
    const now = new Date();
    const time = date.toLocaleTimeString(undefined, {
      hour12: false,
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
    const millis = String(date.getMilliseconds()).padStart(3, "0");
    const clock = `${time}.${millis}`;
    if (date.toDateString() === now.toDateString()) {
      return clock;
    }
    return `${date.toLocaleDateString(undefined, { month: "short", day: "numeric" })} ${clock}`;
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) {
      return `${bytes} B`;
    }
    if (bytes < 1024 * 1024) {
      const kb = bytes / 1024;
      return `${kb >= 10 ? kb.toFixed(0) : kb.toFixed(1)} KB`;
    }
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<div class="pane" onfocusin={onfocuspane}>
  <header>
    {#if topic}
      <h2 title={topic}>{topic}</h2>
      <span class="count">{items.length} {items.length === 1 ? "message" : "messages"}</span>
      {#if items.length > 0}
        <button
          type="button"
          class="jump"
          class:live={followingLatest}
          onclick={jumpToLatest}
        >
          {followingLatest ? "Following latest" : "Jump to latest"}
        </button>
      {/if}
    {:else}
      <h2 class="muted">Messages</h2>
    {/if}
  </header>

  {#if !topic}
    <p class="empty">Select a topic to see its messages.</p>
  {:else if items.length === 0}
    <p class="empty">No messages on this topic yet.</p>
  {:else}
    <div class="scroll" bind:this={listEl} role="listbox" aria-label="Message history">
      {#each rows as item (item.index)}
        {@const latest = item.index === items[items.length - 1]?.index}
        <button
          type="button"
          class="row"
          class:current={item.index === selectedIndex}
          role="option"
          aria-selected={item.index === selectedIndex}
          onclick={() => onselectindex(item.index)}
        >
          <span class="time">{formatArrival(item.timestamp)}</span>
          <span class="meta">
            <span class="badge">{item.format}</span>
            <span class="size">{formatSize(item.size)}</span>
            {#if item.retain}
              <span class="badge retain">retain</span>
            {/if}
            {#if latest && followingLatest}
              <span class="badge latest">latest</span>
            {:else if latest}
              <span class="badge">latest</span>
            {/if}
          </span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
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

  .count {
    color: var(--fg-muted);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .jump {
    flex-shrink: 0;
    border: 1px solid var(--border-strong);
    background: var(--bg-hover);
    border-radius: var(--radius-sm);
    padding: 0.2rem 0.5rem;
    font-size: 11px;
    color: var(--fg-muted);
    white-space: nowrap;
  }

  .jump.live {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }

  .empty {
    margin: 0;
    padding: var(--space-5);
    color: var(--fg-muted);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    overflow-anchor: none;
  }

  .row {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.15rem;
    width: 100%;
    margin: 0;
    padding: 0.45rem var(--space-3);
    border: 0;
    border-bottom: 1px solid var(--border);
    background: transparent;
    text-align: left;
    border-radius: 0;
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .row.current {
    background: var(--accent-muted);
  }

  .time {
    font-family: var(--mono);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
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

  .badge.retain {
    color: var(--retain);
    border-color: color-mix(in srgb, var(--retain) 45%, var(--border));
  }

  .badge.latest {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }

  .size {
    font-size: 11px;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
</style>
