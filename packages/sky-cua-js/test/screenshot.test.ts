import { describe, expect, test } from "bun:test";
import { Buffer } from "node:buffer";

import type { GetScreenshotResponse, WireScreenshot } from "../src/protocol/generated";
import { decodeScreenshots } from "../src/screenshot";

const canonicalBase64 = "UklGRldFQlA=";
const screenshot: WireScreenshot = {
  filepath: "/tmp/capture.webp",
  bytes_base64: canonicalBase64,
  mime_type: "image/webp",
  width: 20,
  height: 10
};

function response(wire: WireScreenshot): GetScreenshotResponse {
  return { type: "get_screenshot", ok: true, screenshots: [wire] };
}

describe("screenshot wire decoding", () => {
  test("derives facade bytes and data_url from the one canonical base64 field", () => {
    const [result] = decodeScreenshots(response(screenshot));

    expect(result?.bytes).toEqual(Uint8Array.from(Buffer.from(canonicalBase64, "base64")));
    expect(result?.data_url).toBe(`data:image/webp;base64,${canonicalBase64}`);
    expect(result?.filepath).toBe("/tmp/capture.webp");
  });

  test("accepts a matching legacy duplicated data_url during transition", () => {
    const legacy = {
      ...screenshot,
      data_url: `data:image/webp;base64,${canonicalBase64}` as const
    };

    expect(decodeScreenshots(response(legacy))[0]?.data_url).toBe(legacy.data_url);
  });

  test("rejects a conflicting legacy duplicated data_url", () => {
    const conflicting = {
      ...screenshot,
      data_url: "data:image/webp;base64,AAAA" as const
    };

    let thrown: unknown;
    try {
      decodeScreenshots(response(conflicting));
    } catch (error) {
      thrown = error;
    }
    expect(thrown instanceof Error).toBe(true);
    expect((thrown as Error).message).toBe(
      "get_screenshot data_url does not match bytes_base64."
    );
  });
});
