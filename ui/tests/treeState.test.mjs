import assert from "node:assert/strict";
import test from "node:test";
import {
  ROOT, ancestorPaths, applyBatch, emptyTree, flatten, parentPath, pathSegments,
  setChildren, visibleFromHits,
} from "../src/lib/treeState.ts";

function node(path, childCount = 0) {
  return { path, segment: path.split("/").at(-1), childCount, hasPayload: childCount === 0 };
}

test("leading and repeated empty levels expand independently from the root", () => {
  const tree = emptyTree();
  assert.deepEqual(pathSegments(ROOT), []);
  assert.deepEqual(pathSegments(""), [""]);
  assert.deepEqual(pathSegments("/v1"), ["", "v1"]);
  assert.equal(parentPath(""), ROOT);
  assert.equal(parentPath("/v1"), "");
  assert.deepEqual(ancestorPaths("//v1/status"), ["", "/", "//v1"]);

  setChildren(tree, ROOT, [node("", 2), node("v1", 1)]);
  setChildren(tree, "", [node("/", 1), node("/v1", 1)]);
  setChildren(tree, "/v1", [node("/v1/asd")]);
  setChildren(tree, "/", [node("//other")]);
  const expanded = new Set(["", "/", "/v1"]);
  assert.deepEqual(flatten(tree, expanded, null).map((row) => row.path), ["", "/", "//other", "/v1", "/v1/asd", "v1"]);
  const visible = visibleFromHits([{ path: "/v1/asd", highlights: [] }]);
  assert.deepEqual(flatten(tree, expanded, visible).map((row) => row.path), ["", "/v1", "/v1/asd"]);

  applyBatch(tree, [], ["/v1/asd", "/v1"]);
  assert.deepEqual(flatten(tree, expanded, null).map((row) => row.path), ["", "/", "//other", "v1"]);
  applyBatch(tree, [], ["//other", "/", ""]);
  assert.deepEqual(flatten(tree, expanded, null).map((row) => row.path), ["v1"]);
});
