import { strict as assert } from "node:assert";
import { test } from "bun:test";
import { analyzeCell } from "../src/host/cell-analysis.ts";

test("cell analysis collects only module-top-level persistent declarations", () => {
  assert.deepEqual(
    analyzeCell(`
      const top = 1;
      let { nested: { value = 2 }, list: [first, ...remaining], ...objectRest } = source;
      var [head, , { tail: renamed }, ...arrayRest] = items;
      function topFunction() { const nestedFunction = 3; }
      class TopClass { method() { let nestedMethod = 4; } }
      if (condition) { const nestedBlock = 5; }
      if (condition) { var nestedVar = 5; }
      for (var loopVar of items) { var loopBodyVar = loopVar; }
      try { var tryVar = 6; throw failure; } catch (caught) { var catchVar = caught; const nestedCatch = caught; }
    `),
    [
      { kind: "const", name: "top" },
      { kind: "let", name: "value" },
      { kind: "let", name: "first" },
      { kind: "let", name: "remaining" },
      { kind: "let", name: "objectRest" },
      { kind: "var", name: "head" },
      { kind: "var", name: "renamed" },
      { kind: "var", name: "arrayRest" },
      { kind: "function", name: "topFunction" },
      { kind: "class", name: "TopClass" },
      { kind: "var", name: "nestedVar" },
      { kind: "var", name: "loopVar" },
      { kind: "var", name: "loopBodyVar" },
      { kind: "var", name: "tryVar" },
      { kind: "var", name: "catchVar" },
    ],
  );
});

test("cell analysis preserves static import and export rejection messages", () => {
  assert.throws(
    () => analyzeCell('import value from "fixture";'),
    /Top-level static import is not supported in node_repl/u,
  );
  assert.throws(
    () => analyzeCell("export const value = 1;"),
    /Top-level export is not supported in node_repl cells/u,
  );
});
