import type { SearchHit, TreeNodeDto } from "./types";

export const ROOT = "";
export const FRESH_UNTIL_MS = 500;
export const STALE_AFTER_MS = 5000;
export const ROW_HEIGHT = 28;

export type TreeModel = {
  nodes: Map<string, TreeNodeDto>;
  children: Map<string, string[]>;
  loaded: Set<string>;
  expanded: Set<string>;
};

export type TreeRow = {
  path: string;
  depth: number;
  node: TreeNodeDto;
};

export function emptyTree(): TreeModel {
  return {
    nodes: new Map(),
    children: new Map([[ROOT, []]]),
    loaded: new Set([ROOT]),
    expanded: new Set(),
  };
}

export function parentPath(path: string): string {
  const index = path.lastIndexOf("/");
  return index === -1 ? ROOT : path.slice(0, index);
}

export function pathSegments(path: string): string[] {
  return path ? path.split("/") : [];
}

export function ancestorPaths(path: string): string[] {
  const parts = pathSegments(path);
  const out: string[] = [];
  for (let i = 1; i < parts.length; i += 1) {
    out.push(parts.slice(0, i).join("/"));
  }
  return out;
}

export function insertChild(
  list: string[],
  path: string,
  nodes: Map<string, TreeNodeDto>,
): string[] {
  if (list.includes(path)) {
    return list;
  }
  const next = [...list, path];
  next.sort((a, b) => {
    const left = nodes.get(a)?.segment ?? a.slice(a.lastIndexOf("/") + 1);
    const right = nodes.get(b)?.segment ?? b.slice(b.lastIndexOf("/") + 1);
    return left < right ? -1 : left > right ? 1 : 0;
  });
  return next;
}

export function upsertNode(tree: TreeModel, node: TreeNodeDto): void {
  tree.nodes.set(node.path, node);
  const parent = parentPath(node.path);
  if (tree.loaded.has(parent)) {
    const list = tree.children.get(parent) ?? [];
    tree.children.set(parent, insertChild(list, node.path, tree.nodes));
  }
}

export function removePath(tree: TreeModel, path: string): void {
  const parent = parentPath(path);
  const siblings = tree.children.get(parent);
  if (siblings) {
    tree.children.set(
      parent,
      siblings.filter((item) => item !== path),
    );
  }
  const prefix = `${path}/`;
  for (const key of [...tree.nodes.keys()]) {
    if (key === path || key.startsWith(prefix)) {
      tree.nodes.delete(key);
    }
  }
  for (const key of [...tree.children.keys()]) {
    if (key === path || key.startsWith(prefix)) {
      tree.children.delete(key);
    }
  }
  for (const key of [...tree.loaded]) {
    if (key === path || key.startsWith(prefix)) {
      tree.loaded.delete(key);
    }
  }
  for (const key of [...tree.expanded]) {
    if (key === path || key.startsWith(prefix)) {
      tree.expanded.delete(key);
    }
  }
}

export function setChildren(tree: TreeModel, parent: string, kids: TreeNodeDto[]): void {
  for (const kid of kids) {
    tree.nodes.set(kid.path, kid);
  }
  tree.children.set(
    parent,
    kids.map((kid) => kid.path),
  );
  tree.loaded.add(parent);
}

export function applyBatch(tree: TreeModel, upserts: TreeNodeDto[], deletes: string[]): void {
  for (const path of deletes) {
    removePath(tree, path);
  }
  for (const node of upserts) {
    upsertNode(tree, node);
  }
}

export function visibleFromHits(hits: SearchHit[]): Set<string> {
  const visible = new Set<string>();
  for (const hit of hits) {
    visible.add(hit.path);
    for (const ancestor of ancestorPaths(hit.path)) {
      visible.add(ancestor);
    }
  }
  return visible;
}

export function flatten(
  tree: TreeModel,
  expanded: Set<string>,
  visible: Set<string> | null,
  hideContaining: string | null = null,
): TreeRow[] {
  const rows: TreeRow[] = [];
  const walk = (parent: string, depth: number) => {
    for (const path of tree.children.get(parent) ?? []) {
      if (hideContaining && path.includes(hideContaining)) {
        continue;
      }
      if (visible && !visible.has(path)) {
        continue;
      }
      const node = tree.nodes.get(path);
      if (!node) {
        continue;
      }
      rows.push({ path, depth, node });
      if (expanded.has(path)) {
        walk(path, depth + 1);
      }
    }
  };
  walk(ROOT, 0);
  return rows;
}

export function liveFreshness(node: TreeNodeDto, now: number): TreeNodeDto["freshness"] {
  if (!node.hasPayload) {
    return "stale";
  }
  if (node.retain) {
    return "retain";
  }
  const age = Math.max(0, now - node.lastMs);
  if (age < FRESH_UNTIL_MS) {
    return "fresh";
  }
  if (age < STALE_AFTER_MS) {
    return "intime";
  }
  return "stale";
}

export function highlightsForPath(path: string, hits: SearchHit[]): number[] {
  const start = path.lastIndexOf("/") + 1;
  const end = path.length;
  const marks = new Set<number>();
  for (const hit of hits) {
    if (hit.path !== path && !hit.path.startsWith(`${path}/`)) {
      continue;
    }
    for (const index of hit.highlights) {
      if (index >= start && index < end) {
        marks.add(index - start);
      }
    }
  }
  return [...marks].sort((a, b) => a - b);
}

export function isTypingTarget(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
}
