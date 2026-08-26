<script lang="ts">
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import {
    getHistoryMeta,
    getMessage,
    selectTopic,
    treeChildren,
    treeSearch,
  } from "./api";
  import { writeClipboard } from "./clipboard";
  import Inspector from "./Inspector.svelte";
  import TopicTree from "./TopicTree.svelte";
  import {
    ROOT,
    ancestorPaths,
    applyBatch,
    emptyTree,
    flatten,
    isTypingTarget,
    pathSegments,
    setChildren,
    visibleFromHits,
    type TreeModel,
  } from "./treeState";
  import {
    errorMessage,
    type HistoryMeta,
    type MessageDto,
    type SearchHit,
    type SearchMode,
    type SessionStatusKind,
    type TreeBatch,
  } from "./types";

  let {
    profileId,
    epoch,
    status,
  }: {
    profileId: string | null;
    epoch: number | null;
    status: SessionStatusKind;
  } = $props();

  let tree = $state<TreeModel>(emptyTree());
  let selected = $state<string | null>(null);
  let message = $state<MessageDto | null>(null);
  let meta = $state<HistoryMeta | null>(null);
  let historyIndex = $state<number | null>(null);
  let query = $state("");
  let mode = $state<SearchMode>("keep");
  let appliedQuery = $state("");
  let appliedMode = $state<SearchMode>("keep");
  let hits = $state<SearchHit[] | null>(null);
  let searchExpanded = $state(new Set<string>());
  let now = $state(Date.now());
  let pane = $state<"tree" | "inspector">("tree");
  let searchEl = $state<HTMLInputElement | null>(null);
  let loading = $state(new Set<string>([ROOT]));
  let queued = $state<TreeBatch[]>([]);
  let error = $state<string | null>(null);
  let seq = 0;
  let liveStamp = 0;
  let searchSeq = 0;
  const inflight = new Map<string, Promise<void>>();

  const SEARCH_HIT_CAP = 256;
  const SEARCH_LOAD_CAP = 64;

  const expanded = $derived.by(() => {
    const set = new Set(tree.expanded);
    if (hits) {
      for (const path of searchExpanded) {
        set.add(path);
      }
    }
    return set;
  });

  const visible = $derived(appliedMode === "keep" && hits ? visibleFromHits(hits) : null);
  const rows = $derived(
    flatten(
      tree,
      expanded,
      visible,
      appliedMode === "skip" && appliedQuery ? appliedQuery : null,
    ),
  );
  const selectedNode = $derived(selected ? (tree.nodes.get(selected) ?? null) : null);
  const emptyKind = $derived(
    appliedQuery
      ? "search"
      : status === "connecting" || status === "reconnecting"
        ? "connecting"
        : "idle",
  );
  const emptyTitle = $derived(
    emptyKind === "search"
      ? appliedMode === "skip"
        ? "Nothing left to show"
        : "No matching topics"
      : emptyKind === "connecting"
        ? "Connecting…"
        : "No topics yet",
  );
  const emptyDetail = $derived(
    emptyKind === "search"
      ? appliedMode === "skip"
        ? "Every visible topic contains that string."
        : "Only topics whose path contains the filter are shown."
      : emptyKind === "connecting"
        ? "The topic tree will appear here once the broker session is up."
        : "Messages show up as they arrive. If this stays empty, check the profile subscriptions.",
  );

  function sameSession(payloadEpoch?: number, payloadProfile?: string | null) {
    if (epoch != null && payloadEpoch != null && payloadEpoch !== epoch) {
      return false;
    }
    if (profileId && payloadProfile && payloadProfile !== profileId) {
      return false;
    }
    return true;
  }

  function isNotFound(err: unknown): boolean {
    return /not found/i.test(errorMessage(err));
  }

  function mutate(fn: (model: TreeModel) => void) {
    fn(tree);
    tree = {
      nodes: tree.nodes,
      children: tree.children,
      loaded: tree.loaded,
      expanded: tree.expanded,
    };
  }

  function applyIncomingBatch(batch: TreeBatch) {
    const current = selected;
    const gone = current
      ? batch.deletes.some((path) => current === path || current.startsWith(`${path}/`))
      : false;
    const cleared = current
      ? batch.upserts.some((node) => node.path === current && !node.hasPayload)
      : false;
    mutate((model) => applyBatch(model, batch.upserts, batch.deletes));
    if ((gone || cleared) && selected) {
      void clearSelection();
    }
  }

  function flushQueued() {
    const pending = queued;
    queued = [];
    for (const batch of pending) {
      applyIncomingBatch(batch);
    }
  }

  function loadChildren(path: string): Promise<void> {
    const existing = inflight.get(path);
    if (existing) {
      return existing;
    }
    const task = (async () => {
      loading.add(path);
      loading = new Set(loading);
      try {
        const kids = await treeChildren(path ? pathSegments(path) : []);
        mutate((model) => setChildren(model, path, kids));
      } finally {
        loading.delete(path);
        loading = new Set(loading);
        inflight.delete(path);
        if (loading.size === 0) {
          flushQueued();
        }
      }
    })();
    inflight.set(path, task);
    return task;
  }

  async function loadRoot() {
    try {
      await loadChildren(ROOT);
    } catch (err) {
      error = errorMessage(err);
    }
  }

  function onBatch(batch: TreeBatch) {
    if (!sameSession(batch.epoch, batch.profileId)) {
      return;
    }
    if (loading.size > 0) {
      queued = [...queued, batch];
      return;
    }
    applyIncomingBatch(batch);
  }

  async function revealHits(nextHits: SearchHit[], token: number) {
    const needed = new Set<string>();
    for (const hit of nextHits) {
      for (const ancestor of ancestorPaths(hit.path)) {
        needed.add(ancestor);
      }
    }
    const ordered = [...needed].sort(
      (a, b) => pathSegments(a).length - pathSegments(b).length,
    );
    if (token !== searchSeq) {
      return;
    }
    searchExpanded = new Set(ordered);
    const toLoad = ordered.filter((path) => !tree.loaded.has(path)).slice(0, SEARCH_LOAD_CAP);
    for (const path of toLoad) {
      if (token !== searchSeq) {
        return;
      }
      try {
        await loadChildren(path);
      } catch (err) {
        if (token === searchSeq) {
          error = errorMessage(err);
        }
        return;
      }
    }
  }

  async function select(path: string) {
    const node = tree.nodes.get(path);
    if (!node?.hasPayload) {
      return;
    }
    const token = ++seq;
    liveStamp = 0;
    selected = path;
    clearInspector();
    error = null;
    try {
      await selectTopic(path);
      if (token !== seq) {
        return;
      }
      const [nextMessage, nextMeta] = await Promise.all([
        getMessage(path, null),
        getHistoryMeta(path),
      ]);
      if (token !== seq) {
        return;
      }
      if (
        message !== null &&
        message.topic === path &&
        (message.timestamp > nextMessage.timestamp ||
          (meta !== null && meta.latestIndex > nextMeta.latestIndex))
      ) {
        return;
      }
      message = nextMessage;
      meta = nextMeta;
      historyIndex = nextMeta.latestIndex;
    } catch (err) {
      if (token !== seq) {
        return;
      }
      if (isNotFound(err)) {
        void clearSelection();
        return;
      }
      error = errorMessage(err);
    }
  }

  function clearInspector() {
    message = null;
    meta = null;
    historyIndex = null;
  }

  async function clearSelection() {
    seq += 1;
    selected = null;
    clearInspector();
    try {
      await selectTopic(null);
    } catch (err) {
      error = errorMessage(err);
    }
  }

  async function toggle(path: string) {
    if (tree.expanded.has(path)) {
      tree.expanded.delete(path);
      tree = {
        nodes: tree.nodes,
        children: tree.children,
        loaded: tree.loaded,
        expanded: tree.expanded,
      };
      return;
    }
    if (!tree.loaded.has(path) || inflight.has(path)) {
      try {
        await loadChildren(path);
      } catch (err) {
        error = errorMessage(err);
        return;
      }
    }
    tree.expanded.add(path);
    tree = {
      nodes: tree.nodes,
      children: tree.children,
      loaded: tree.loaded,
      expanded: tree.expanded,
    };
  }

  async function openHistory(index: number) {
    if (!selected) {
      return;
    }
    const topic = selected;
    const token = seq;
    try {
      const nextMessage = await getMessage(topic, index);
      if (token !== seq || selected !== topic) {
        return;
      }
      message = nextMessage;
      historyIndex = index;
    } catch (err) {
      if (token !== seq || selected !== topic) {
        return;
      }
      if (isNotFound(err)) {
        void clearSelection();
        return;
      }
      error = errorMessage(err);
    }
  }

  function stepHistory(delta: number) {
    if (meta == null || historyIndex == null) {
      return;
    }
    const next = Math.min(meta.count - 1, Math.max(0, historyIndex + delta));
    if (next !== historyIndex) {
      void openHistory(next);
    }
  }

  async function onTopicMessage(payload: MessageDto) {
    if (!sameSession(payload.epoch)) {
      return;
    }
    if (payload.topic !== selected) {
      return;
    }
    const token = seq;
    const stamp = ++liveStamp;
    const follow = historyIndex == null || meta == null || historyIndex === meta.latestIndex;
    try {
      const nextMeta = await getHistoryMeta(payload.topic);
      if (token !== seq || stamp !== liveStamp || payload.topic !== selected) {
        return;
      }
      meta = nextMeta;
      if (follow) {
        message = payload;
        historyIndex = nextMeta.latestIndex;
      }
    } catch (err) {
      if (token !== seq || stamp !== liveStamp) {
        return;
      }
      if (isNotFound(err)) {
        void clearSelection();
        return;
      }
      error = errorMessage(err);
    }
  }

  function copyTopic() {
    if (!selected) {
      return;
    }
    writeClipboard(selected);
  }

  function onKey(event: KeyboardEvent) {
    const typing = isTypingTarget(event.target);
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      searchEl?.focus();
      searchEl?.select();
      return;
    }
    if (event.key === "/" && !typing) {
      event.preventDefault();
      mode = "keep";
      searchEl?.focus();
      return;
    }
    if (event.key === "?" && !typing) {
      event.preventDefault();
      mode = "skip";
      searchEl?.focus();
      return;
    }
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "c") {
      if (typing) {
        return;
      }
      if (pane === "tree") {
        event.preventDefault();
        copyTopic();
      }
      return;
    }
    if (typing || event.metaKey || event.ctrlKey || event.altKey) {
      return;
    }
    if (event.key === "h") {
      event.preventDefault();
      stepHistory(-1);
    } else if (event.key === "l") {
      event.preventDefault();
      stepHistory(1);
    }
  }

  async function applyFilter() {
    const q = query.trim();
    appliedQuery = q;
    appliedMode = mode;
    const token = ++searchSeq;
    if (!q || mode === "skip") {
      hits = null;
      searchExpanded = new Set();
      return;
    }
    try {
      const nextHits = await treeSearch(q, "keep");
      if (token !== searchSeq) {
        return;
      }
      const limited = nextHits.slice(0, SEARCH_HIT_CAP);
      hits = limited;
      await revealHits(limited, token);
    } catch (err) {
      if (token === searchSeq) {
        error = errorMessage(err);
      }
    }
  }

  $effect(() => {
    const id = setInterval(() => {
      now = Date.now();
    }, 250);
    return () => clearInterval(id);
  });

  onMount(() => {
    const unlistens: UnlistenFn[] = [];
    let cancelled = false;

    void (async () => {
      try {
        const treeUnlisten = await listen<TreeBatch>("tree/batch", (event) => {
          onBatch(event.payload);
        });
        const messageUnlisten = await listen<MessageDto>("topic/message", (event) => {
          void onTopicMessage(event.payload);
        });
        const searchUnlisten = await listen("menu/search", () => {
          searchEl?.focus();
          searchEl?.select();
        });
        if (cancelled) {
          treeUnlisten();
          messageUnlisten();
          searchUnlisten();
          return;
        }
        unlistens.push(treeUnlisten, messageUnlisten, searchUnlisten);
      } catch (err) {
        error = errorMessage(err);
      }
      if (!cancelled) {
        await loadRoot();
      }
    })();

    window.addEventListener("keydown", onKey);
    return () => {
      cancelled = true;
      window.removeEventListener("keydown", onKey);
      for (const unlisten of unlistens) {
        unlisten();
      }
      void selectTopic(null);
    };
  });

</script>

<div class="explorer">
  {#if error}
    <p class="banner" role="alert">{error}</p>
  {/if}
  <div class="split">
    <aside class="tree">
      <TopicTree
        {rows}
        {selected}
        {expanded}
        bind:query
        bind:mode
        {hits}
        {now}
        {emptyKind}
        {emptyTitle}
        {emptyDetail}
        bind:searchEl
        onapply={() => void applyFilter()}
        onselect={(path) => void select(path)}
        ontoggle={(path) => void toggle(path)}
        onfocuspane={() => (pane = "tree")}
      />
    </aside>
    <section class="detail">
      {#key selected}
        <Inspector
          topic={selected}
          {message}
          {meta}
          {historyIndex}
          node={selectedNode}
          {now}
          onselectindex={(index) => void openHistory(index)}
          onfocuspane={() => (pane = "inspector")}
        />
      {/key}
    </section>
  </div>
</div>

<style>
  .explorer {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .banner {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    flex-shrink: 0;
  }

  .split {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(16rem, 28%) 1fr;
  }

  .tree,
  .detail {
    min-width: 0;
    min-height: 0;
  }

  .detail {
    display: flex;
    flex-direction: column;
  }

  .tree {
    border-right: 1px solid var(--border);
  }
</style>
