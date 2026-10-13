<script lang="ts">
  import { json } from "@codemirror/lang-json";
  import {
    HighlightStyle,
    bracketMatching,
    codeFolding,
    ensureSyntaxTree,
    foldEffect,
    foldGutter,
    foldInside,
    foldState,
    foldable,
    syntaxTree,
    syntaxHighlighting,
    unfoldAll,
    unfoldEffect,
  } from "@codemirror/language";
  import { Compartment, EditorState, StateEffect, Transaction } from "@codemirror/state";
  import { EditorView, lineNumbers } from "@codemirror/view";
  import { tags } from "@lezer/highlight";
  import { onMount } from "svelte";
  import { writeClipboard } from "./clipboard";
  import { resolvedDark } from "./theme";

  let {
    doc = "",
    jsonMode = false,
  }: {
    doc?: string;
    jsonMode?: boolean;
  } = $props();

  let parent = $state<HTMLDivElement | undefined>();
  let wrap = $state<HTMLDivElement | undefined>();
  let view = $state<EditorView | undefined>();
  let dark = $state(resolvedDark());
  let menu = $state<{ x: number; y: number } | null>(null);
  const lang = new Compartment();
  const appearance = new Compartment();
  type FoldRange = { from: number; to: number };
  let plannedFolds: FoldRange[] = [];
  const FOLD_EVENT = "mqx-fold";

  function containedIn(inner: FoldRange, outer: FoldRange): boolean {
    return (
      inner.from >= outer.from &&
      inner.to <= outer.to &&
      (inner.from !== outer.from || inner.to !== outer.to)
    );
  }

  function outermost(ranges: FoldRange[]): FoldRange[] {
    return ranges.filter((range) => !ranges.some((other) => containedIn(range, other)));
  }

  function collectFolds(editor: EditorView): FoldRange[] {
    ensureSyntaxTree(editor.state, editor.state.doc.length, 5000);
    const seen = new Set<string>();
    const ranges: FoldRange[] = [];
    const add = (range: FoldRange | null | undefined) => {
      if (!range || range.from >= range.to) {
        return;
      }
      const key = `${range.from}:${range.to}`;
      if (seen.has(key)) {
        return;
      }
      seen.add(key);
      ranges.push(range);
    };
    syntaxTree(editor.state).iterate({
      enter(node) {
        if (node.name === "Object" || node.name === "Array") {
          add(foldInside(node.node));
        }
      },
    });
    for (let pos = 0; pos < editor.state.doc.length; ) {
      const line = editor.lineBlockAt(pos);
      add(foldable(editor.state, line.from, line.to));
      pos = line.to + 1;
    }
    return ranges;
  }

  function dispatchFolds(editor: EditorView, ranges: FoldRange[]) {
    const effects: StateEffect<unknown>[] = ranges.map((range) => foldEffect.of(range));
    if (!editor.state.field(foldState, false)) {
      effects.push(StateEffect.appendConfig.of(codeFolding()));
    }
    if (!effects.length) {
      return;
    }
    editor.dispatch({
      effects,
      annotations: Transaction.userEvent.of(FOLD_EVENT),
    });
  }

  const foldAfterUnfold = EditorView.updateListener.of((update) => {
    if (!plannedFolds.length) {
      return;
    }
    const opened: FoldRange[] = [];
    for (const tr of update.transactions) {
      if (tr.annotation(Transaction.userEvent) === FOLD_EVENT) {
        continue;
      }
      for (const effect of tr.effects) {
        if (effect.is(unfoldEffect)) {
          opened.push(effect.value);
        }
      }
    }
    if (!opened.length) {
      return;
    }
    const next: FoldRange[] = [];
    for (const parent of opened) {
      next.push(
        ...outermost(plannedFolds.filter((range) => containedIn(range, parent))),
      );
    }
    if (!next.length) {
      return;
    }
    queueMicrotask(() => {
      if (update.view === view) {
        dispatchFolds(update.view, next);
      }
    });
  });

  function highlightStyle(isDark: boolean) {
    return HighlightStyle.define(
      isDark
        ? [
            { tag: tags.propertyName, color: "#9cdcfe" },
            { tag: tags.string, color: "#ce9178" },
            { tag: tags.number, color: "#b5cea8" },
            { tag: tags.bool, color: "#569cd6" },
            { tag: tags.null, color: "#569cd6" },
            { tag: tags.keyword, color: "#c586c0" },
            { tag: tags.punctuation, color: "#d4d4d4" },
          ]
        : [
            { tag: tags.propertyName, color: "#0451a5" },
            { tag: tags.string, color: "#a31515" },
            { tag: tags.number, color: "#098658" },
            { tag: tags.bool, color: "#0451a5" },
            { tag: tags.null, color: "#0451a5" },
            { tag: tags.keyword, color: "#af00db" },
            { tag: tags.punctuation, color: "#393a34" },
          ],
    );
  }

  function languageExt(mode: boolean, isDark: boolean) {
    return mode
      ? [json(), foldGutter(), bracketMatching(), syntaxHighlighting(highlightStyle(isDark))]
      : [];
  }

  function appearanceExt(dark: boolean) {
    return EditorView.theme(
      {
        "&": {
          height: "100%",
          backgroundColor: "transparent",
          color: "var(--fg)",
          fontSize: "12.5px",
        },
        "&.cm-focused": { outline: "none" },
        ".cm-scroller": {
          overflow: "auto",
          fontFamily: "var(--mono)",
        },
        ".cm-content": { caretColor: "transparent" },
        ".cm-gutters": {
          backgroundColor: "transparent",
          borderRight: "1px solid var(--border)",
          color: "var(--fg-faint)",
        },
        ".cm-activeLine": { backgroundColor: "transparent" },
        ".cm-activeLineGutter": { backgroundColor: "transparent" },
        ".cm-foldGutter span": { color: "var(--fg-faint)" },
      },
      { dark },
    );
  }

  function closeMenu() {
    menu = null;
  }

  function onContextMenu(event: MouseEvent) {
    if (!jsonMode) {
      return;
    }
    event.preventDefault();
    const box = wrap?.getBoundingClientRect();
    if (!box) {
      return;
    }
    const width = 168;
    const height = 108;
    menu = {
      x: Math.min(Math.max(4, event.clientX - box.left), Math.max(4, box.width - width)),
      y: Math.min(Math.max(4, event.clientY - box.top), Math.max(4, box.height - height)),
    };
  }

  function copyDoc() {
    writeClipboard(view?.state.doc.toString() ?? doc);
    closeMenu();
  }

  function collapseAll() {
    if (view) {
      plannedFolds = collectFolds(view);
      dispatchFolds(view, outermost(plannedFolds));
    }
    closeMenu();
  }

  function expandAll() {
    plannedFolds = [];
    if (view) {
      unfoldAll(view);
    }
    closeMenu();
  }

  onMount(() => {
    if (!parent) {
      return;
    }
    view = new EditorView({
      parent,
      state: EditorState.create({
        doc,
        extensions: [
          EditorState.readOnly.of(true),
          EditorView.editable.of(false),
          lineNumbers(),
          codeFolding(),
          foldAfterUnfold,
          lang.of(languageExt(jsonMode, resolvedDark())),
          appearance.of(appearanceExt(resolvedDark())),
        ],
      }),
    });
    const onTheme = () => {
      dark = resolvedDark();
    };
    window.addEventListener("mqx:theme", onTheme);
    const media = window.matchMedia("(prefers-color-scheme: light)");
    media.addEventListener("change", onTheme);
    const onPointer = (event: PointerEvent) => {
      if (event.target instanceof Element && event.target.closest("[data-menu]")) {
        return;
      }
      closeMenu();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        closeMenu();
      }
    };
    window.addEventListener("pointerdown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mqx:theme", onTheme);
      media.removeEventListener("change", onTheme);
      window.removeEventListener("pointerdown", onPointer);
      window.removeEventListener("keydown", onKey);
      view?.destroy();
      view = undefined;
    };
  });

  $effect(() => {
    const editor = view;
    const next = doc;
    const mode = jsonMode;
    const isDark = dark;
    if (!editor) {
      return;
    }
    if (editor.state.doc.toString() !== next) {
      plannedFolds = [];
      editor.dispatch({
        changes: { from: 0, to: editor.state.doc.length, insert: next },
      });
    }
  });

  $effect(() => {
    const editor = view;
    const mode = jsonMode;
    const isDark = dark;
    if (!editor) {
      return;
    }
    editor.dispatch({
      effects: [
        lang.reconfigure(languageExt(mode, isDark)),
        appearance.reconfigure(appearanceExt(isDark)),
      ],
    });
  });
</script>

<div
  bind:this={wrap}
  class="wrap"
  role="region"
  aria-label="Message payload"
  oncontextmenu={onContextMenu}
>
  <div bind:this={parent} class="cm"></div>
  {#if menu}
    <div class="menu" data-menu style="left: {menu.x}px; top: {menu.y}px" role="menu">
      <button type="button" role="menuitem" onpointerdown={copyDoc}>Copy payload</button>
      <button type="button" role="menuitem" onpointerdown={collapseAll}>Collapse all</button>
      <button type="button" role="menuitem" onpointerdown={expandAll}>Expand all</button>
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    height: 100%;
    min-height: 0;
  }

  .cm {
    height: 100%;
    min-height: 0;
  }

  .cm :global(.cm-editor) {
    height: 100%;
  }

  .menu {
    position: absolute;
    z-index: 5;
    min-width: 10.5rem;
    padding: 0.2rem;
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    box-shadow: var(--shadow);
  }

  .menu button {
    display: block;
    width: 100%;
    text-align: left;
    padding: 0.35rem 0.55rem;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--fg);
    font-size: 12px;
  }

  .menu button:hover {
    background: var(--bg-hover);
  }
</style>
