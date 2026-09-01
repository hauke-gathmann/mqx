<script lang="ts">
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import {
    getHistoryMeta,
    getMessage,
    listHistory,
    selectTopic,
    treeChildren,
    treeSearch,
  } from "./api";
  import { writeClipboard } from "./clipboard";
  import Inspector from "./Inspector.svelte";
  import MessageList, { listPageSize } from "./MessageList.svelte";
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
    type HistoryItem,
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
  let history = $state<HistoryItem[]>([]);
  let historyIndex = $state<number | null>(null);
  let query = $state("");
  let mode = $state<SearchMode>("keep");
  let appliedQuery = $state("");
  let appliedMode = $state<SearchMode>("keep");
  let hits = $state<SearchHit[] | null>(null);
  let searchExpanded = $state(new Set<string>());
  let now = $state(Date.now());
  let pane = $state<"tree" | "messages" | "inspector">("tree");
  let searchEl = $state<HTMLInputElement | null>(null);
  let messageListEl = $state<HTMLDivElement | null>(null);
  let loading = $state(new Set<string>([ROOT]));
  let queued = $state<TreeBatch[]>([]);
  let error = $state<string | null>(null);
  let seq = 0;
  let liveStamp = 0;
  let searchSeq = 0;
  const inflight = new Map<string, Promise<void>>();

  const SPLIT_KEY = "mqx:split";
  const DEFAULT_TREE_PCT = 26;
  const DEFAULT_MESSAGES_PCT = 24;
  const MIN_TREE_PX = 180;
  const MIN_MESSAGES_PX = 200;
  const MIN_DETAIL_PX = 240;

  function readSplit(): { tree: number; messages: number } {
    try {
      const raw = localStorage.getItem(SPLIT_KEY);
      if (!raw) {
        return { tree: DEFAULT_TREE_PCT, messages: DEFAULT_MESSAGES_PCT };
      }
      const parsed = JSON.parse(raw) as { tree?: unknown; messages?: unknown };
      const tree = typeof parsed.tree === "number" ? parsed.tree : DEFAULT_TREE_PCT;
      const messages = typeof parsed.messages === "number" ? parsed.messages : DEFAULT_MESSAGES_PCT;
      if (tree < 8 || messages < 8 || tree + messages > 88) {
        return { tree: DEFAULT_TREE_PCT, messages: DEFAULT_MESSAGES_PCT };
      }
      return { tree, messages };
    } catch {
      return { tree: DEFAULT_TREE_PCT, messages: DEFAULT_MESSAGES_PCT };
    }
  }

  const initialSplit = readSplit();
  let splitEl = $state<HTMLDivElement | undefined>();
  let treePct = $state(initialSplit.tree);
  let messagesPct = $state(initialSplit.messages);
  let dragging = $state<"tree" | "messages" | null>(null);
  const detailPct = $derived(Math.max(8, 100 - treePct - messagesPct));
  const splitStyle = $derived(
    `minmax(${MIN_TREE_PX}px, ${treePct}fr) 7px minmax(${MIN_MESSAGES_PX}px, ${messagesPct}fr) 7px minmax(${MIN_DETAIL_PX}px, ${detailPct}fr)`,
  );

  function persistSplit() {
    try {
      localStorage.setItem(SPLIT_KEY, JSON.stringify({ tree: treePct, messages: messagesPct }));
    } catch {
      // Ignore quota / private-mode failures.
    }
  }

  function clampSplit(nextTree: number, nextMessages: number, width: number) {
    const minTree = (MIN_TREE_PX / width) * 100;
    const minMessages = (MIN_MESSAGES_PX / width) * 100;
    const minDetail = (MIN_DETAIL_PX / width) * 100;
    const maxPair = 100 - minDetail;
    let tree = Math.max(minTree, nextTree);
    let messages = Math.max(minMessages, nextMessages);
    if (tree + messages > maxPair) {
      messages = Math.max(minMessages, maxPair - tree);
      tree = Math.max(minTree, Math.min(tree, maxPair - messages));
    }
    treePct = tree;
    messagesPct = messages;
  }

  function startDrag(which: "tree" | "messages", event: PointerEvent) {
    const el = splitEl;
    if (!el || event.button !== 0) {
      return;
    }
    event.preventDefault();
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    dragging = which;
    const width = el.getBoundingClientRect().width;
    const originX = event.clientX;
    const originTree = treePct;
    const originMessages = messagesPct;

    function onMove(move: PointerEvent) {
      const dPct = ((move.clientX - originX) / width) * 100;
      if (which === "tree") {
        clampSplit(originTree + dPct, originMessages - dPct, width);
      } else {
        clampSplit(originTree, originMessages + dPct, width);
      }
    }

    function onUp(up: PointerEvent) {
      handle.releasePointerCapture(up.pointerId);
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
      handle.removeEventListener("pointercancel", onUp);
      dragging = null;
      persistSplit();
    }

    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
    handle.addEventListener("pointercancel", onUp);
  }

  function nudgeSplit(which: "tree" | "messages", deltaPct: number) {
    const width = splitEl?.getBoundingClientRect().width ?? 1000;
    if (which === "tree") {
      clampSplit(treePct + deltaPct, messagesPct - deltaPct, width);
    } else {
      clampSplit(treePct, messagesPct + deltaPct, width);
    }
    persistSplit();
  }

  function onGutterKey(which: "tree" | "messages", event: KeyboardEvent) {
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      event.stopPropagation();
      nudgeSplit(which, -2);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      event.stopPropagation();
      nudgeSplit(which, 2);
    } else if (event.key === "Home") {
      event.preventDefault();
      event.stopPropagation();
      treePct = DEFAULT_TREE_PCT;
      messagesPct = DEFAULT_MESSAGES_PCT;
      persistSplit();
    }
  }

  function resetSplit() {
    treePct = DEFAULT_TREE_PCT;
    messagesPct = DEFAULT_MESSAGES_PCT;
    persistSplit();
  }

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
        : status === "detached"
          ? "The tree is frozen. Go live to apply traffic received in the meantime."
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

  function isTopicNotFound(err: unknown): boolean {
    return /^topic .+ not found$/i.test(errorMessage(err));
  }

  function isMessageNotFound(err: unknown): boolean {
    return /^message .+ not found on /i.test(errorMessage(err));
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
    const selectedUpserted = current
      ? batch.upserts.some((node) => node.path === current && node.hasPayload)
      : false;
    mutate((model) => applyBatch(model, batch.upserts, batch.deletes));
    if ((gone || cleared) && selected) {
      void clearSelection();
    } else if (selectedUpserted && historyIndex != null) {
      void refreshSelectedHistory();
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

  async function resyncFromStore() {
    const paths = [...tree.loaded].sort(
      (a, b) => pathSegments(a).length - pathSegments(b).length,
    );
    for (const path of paths) {
      inflight.delete(path);
      try {
        await loadChildren(path);
      } catch (err) {
        error = errorMessage(err);
        return;
      }
    }
    const topic = selected;
    if (!topic) {
      return;
    }
    const token = seq;
    const follow = historyIndex == null || meta == null || historyIndex === meta.latestIndex;
    try {
      const [nextMessage, nextMeta, nextHistory] = await Promise.all([
        getMessage(topic, null),
        getHistoryMeta(topic),
        listHistory(topic),
      ]);
      if (token !== seq || selected !== topic) {
        return;
      }
      meta = nextMeta;
      history = nextHistory;
      if (follow) {
        message = nextMessage;
        historyIndex = nextMeta.latestIndex;
      } else {
        const clamped = clampHistoryIndex(nextHistory, nextMeta.latestIndex, historyIndex);
        if (clamped !== historyIndex) {
          historyIndex = clamped;
          void openHistory(clamped, true);
        }
      }
    } catch (err) {
      if (token !== seq || selected !== topic) {
        return;
      }
      if (isTopicNotFound(err)) {
        void clearSelection();
        return;
      }
      error = errorMessage(err);
    }
  }

  let prevStatus: SessionStatusKind | null = null;
  $effect(() => {
    const next = status;
    const prev = prevStatus;
    prevStatus = next;
    if (prev === "detached" && next === "connected") {
      void resyncFromStore();
    }
  });

  function onBatch(batch: TreeBatch) {
    if (status === "detached" || !sameSession(batch.epoch, batch.profileId)) {
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

  function focusMessageList() {
    if (isTypingTarget(document.activeElement)) {
      return;
    }
    messageListEl?.focus({ preventScroll: true });
  }

  async function select(path: string) {
    const node = tree.nodes.get(path);
    if (!node?.hasPayload) {
      return;
    }
    const token = ++seq;
    liveStamp = 0;
    selected = path;
    pane = "messages";
    clearInspector();
    error = null;
    void tick().then(() => {
      if (token === seq) {
        focusMessageList();
      }
    });
    try {
      await selectTopic(path);
      if (token !== seq) {
        return;
      }
      const [nextMessage, nextMeta, nextHistory] = await Promise.all([
        getMessage(path, null),
        getHistoryMeta(path),
        listHistory(path),
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
      history = nextHistory;
      historyIndex = nextMeta.latestIndex;
    } catch (err) {
      if (token !== seq) {
        return;
      }
      if (isTopicNotFound(err)) {
        void clearSelection();
        return;
      }
      error = errorMessage(err);
    }
  }

  function clearInspector() {
    message = null;
    meta = null;
    history = [];
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

  async function openHistory(index: number, recovered = false) {
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
      if (isMessageNotFound(err)) {
        if (!recovered) {
          void refreshSelectedHistory();
        }
        return;
      }
      if (isTopicNotFound(err)) {
        void clearSelection();
        return;
      }
      error = errorMessage(err);
    }
  }

  async function refreshSelectedHistory() {
    if (!selected || historyIndex == null) {
      return;
    }
    const topic = selected;
    const token = seq;
    try {
      const [nextMeta, nextHistory] = await Promise.all([
        getHistoryMeta(topic),
        listHistory(topic),
      ]);
      if (token !== seq || selected !== topic) {
        return;
      }
      meta = nextMeta;
      history = nextHistory;
      const clamped = clampHistoryIndex(nextHistory, nextMeta.latestIndex, historyIndex);
      if (clamped !== historyIndex) {
        historyIndex = clamped;
        void openHistory(clamped, true);
      }
    } catch (err) {
      if (token !== seq || selected !== topic) {
        return;
      }
      if (isTopicNotFound(err)) {
        void clearSelection();
        return;
      }
      error = errorMessage(err);
    }
  }

  function clampHistoryIndex(
    items: HistoryItem[],
    latestIndex: number,
    current: number | null,
  ): number {
    if (current != null && items.some((item) => item.index === current)) {
      return current;
    }
    return items[0]?.index ?? latestIndex;
  }

  function stepHistory(delta: number) {
    if (historyIndex == null || history.length === 0) {
      return;
    }
    const pos = history.findIndex((item) => item.index === historyIndex);
    if (pos < 0) {
      return;
    }
    const next = Math.min(history.length - 1, Math.max(0, pos + delta));
    const nextIndex = history[next]?.index;
    if (nextIndex != null && nextIndex !== historyIndex) {
      void openHistory(nextIndex);
    }
  }

  function jumpHistory(index: number) {
    if (meta == null) {
      return;
    }
    const next = Math.min(meta.count - 1, Math.max(0, index));
    if (next !== historyIndex) {
      void openHistory(next);
    }
  }

  async function onTopicMessage(payload: MessageDto) {
    if (status === "detached" || !sameSession(payload.epoch)) {
      return;
    }
    if (payload.topic !== selected) {
      return;
    }
    const token = seq;
    const stamp = ++liveStamp;
    const follow = historyIndex == null || meta == null || historyIndex === meta.latestIndex;
    try {
      const [nextMeta, nextHistory] = await Promise.all([
        getHistoryMeta(payload.topic),
        listHistory(payload.topic),
      ]);
      if (token !== seq || stamp !== liveStamp || payload.topic !== selected) {
        return;
      }
      meta = nextMeta;
      history = nextHistory;
      if (follow) {
        message = payload;
        historyIndex = nextMeta.latestIndex;
      } else {
        const clamped = clampHistoryIndex(nextHistory, nextMeta.latestIndex, historyIndex);
        if (clamped !== historyIndex) {
          historyIndex = clamped;
          void openHistory(clamped);
        }
      }
    } catch (err) {
      if (token !== seq || stamp !== liveStamp) {
        return;
      }
      if (isTopicNotFound(err)) {
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
    } else if (pane === "tree") {
      return;
    } else if (event.key === "ArrowDown" || event.key === "j") {
      event.preventDefault();
      stepHistory(-1);
    } else if (event.key === "ArrowUp" || event.key === "k") {
      event.preventDefault();
      stepHistory(1);
    } else if (pane === "messages" && event.key === "Home") {
      event.preventDefault();
      if (meta != null) {
        jumpHistory(meta.latestIndex);
      }
    } else if (pane === "messages" && event.key === "End") {
      event.preventDefault();
      jumpHistory(0);
    } else if (pane === "messages" && event.key === "PageUp") {
      event.preventDefault();
      stepHistory(listPageSize(messageListEl));
    } else if (pane === "messages" && event.key === "PageDown") {
      event.preventDefault();
      stepHistory(-listPageSize(messageListEl));
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

<div class="explorer" data-status={status}>
  {#if error}
    <p class="banner" role="alert">{error}</p>
  {/if}
  <div class="split" class:dragging bind:this={splitEl} style:grid-template-columns={splitStyle}>
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
    <button
      type="button"
      class="gutter"
      aria-label="Resize topic tree"
      title="Drag to resize. Double-click to reset."
      onpointerdown={(event) => startDrag("tree", event)}
      ondblclick={resetSplit}
      onkeydown={(event) => onGutterKey("tree", event)}
    ></button>
    <section class="messages">
      <MessageList
        topic={selected}
        items={history}
        selectedIndex={historyIndex}
        bind:listEl={messageListEl}
        onselectindex={(index) => void openHistory(index)}
        onfocuspane={() => (pane = "messages")}
      />
    </section>
    <button
      type="button"
      class="gutter"
      aria-label="Resize message list"
      title="Drag to resize. Double-click to reset."
      onpointerdown={(event) => startDrag("messages", event)}
      ondblclick={resetSplit}
      onkeydown={(event) => onGutterKey("messages", event)}
    ></button>
    <section class="detail">
      {#key selected}
        <Inspector
          topic={selected}
          {message}
          {historyIndex}
          node={selectedNode}
          {now}
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
  }

  .split.dragging {
    cursor: col-resize;
    user-select: none;
  }

  .tree,
  .messages,
  .detail {
    min-width: 0;
    min-height: 0;
  }

  .detail {
    display: flex;
    flex-direction: column;
  }

  .gutter {
    position: relative;
    z-index: 1;
    width: 100%;
    height: 100%;
    margin: 0;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: var(--border);
    cursor: col-resize;
    touch-action: none;
  }

  .gutter::after {
    content: "";
    position: absolute;
    inset: 0 -3px;
  }

  .gutter:hover,
  .gutter:focus-visible,
  .split.dragging .gutter {
    background: var(--accent);
  }

  .gutter:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
</style>
