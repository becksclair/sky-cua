import { describe, expect, test } from "bun:test";

import {
  isSkyCuaObserveResult,
  modelSupportsImages,
  omitUnsupportedImages,
} from "./sky-cua-image-capability";

const text = { type: "text", text: "rich accessibility tree" };
const image = { type: "image", data: "base64", mimeType: "image/png" };

describe("Pi model image capability", () => {
  test("removes images but preserves semantic text for text-only models", () => {
    expect(modelSupportsImages({ input: ["text"] })).toBe(false);
    expect(omitUnsupportedImages([text, image], { input: ["text"] })).toEqual([
      text,
      {
        type: "text",
        text: "Image attachment omitted because the active Pi model does not support image input.",
      },
    ]);
  });

  test("leaves image-capable results unchanged", () => {
    expect(modelSupportsImages({ input: ["text", "image"] })).toBe(true);
    expect(omitUnsupportedImages([text, image], { input: ["text", "image"] })).toBeUndefined();
  });

  test("fails closed when model modalities are unavailable", () => {
    expect(omitUnsupportedImages([image], undefined)?.map((block) => block.type)).toEqual([
      "text",
    ]);
  });

  test("applies only to sky-cua observe results", () => {
    expect(isSkyCuaObserveResult({ toolName: "sky_cua_observe" })).toBe(true);
    expect(
      isSkyCuaObserveResult({
        toolName: "mcp",
        details: { server: "sky_cua", tool: "observe" },
      }),
    ).toBe(true);
    expect(isSkyCuaObserveResult({ toolName: "capture_screen" })).toBe(false);
    expect(
      isSkyCuaObserveResult({
        toolName: "mcp",
        details: { server: "another_server", tool: "observe" },
      }),
    ).toBe(false);
  });
});
