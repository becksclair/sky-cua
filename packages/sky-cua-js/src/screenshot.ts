import { Buffer } from "node:buffer";

import { invalidArgument } from "./errors";
import type { GetScreenshotResponse, ScreenshotResult, WireScreenshot } from "./protocol/generated";

function isWireScreenshot(value: unknown): value is WireScreenshot {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const screenshot = value as Partial<WireScreenshot>;
  return (
    typeof screenshot.filepath === "string" &&
    screenshot.filepath.length > 0 &&
    typeof screenshot.bytes_base64 === "string" &&
    screenshot.bytes_base64.length > 0 &&
    (screenshot.data_url === undefined ||
      (typeof screenshot.data_url === "string" &&
        screenshot.data_url.startsWith("data:image/webp;base64,"))) &&
    screenshot.mime_type === "image/webp" &&
    typeof screenshot.width === "number" &&
    Number.isInteger(screenshot.width) &&
    screenshot.width > 0 &&
    typeof screenshot.height === "number" &&
    Number.isInteger(screenshot.height) &&
    screenshot.height > 0
  );
}

export function decodeScreenshots(response: GetScreenshotResponse): ScreenshotResult[] {
  if (!Array.isArray(response.screenshots)) {
    throw invalidArgument("get_screenshot response must contain an array of screenshots.");
  }
  return response.screenshots.map((wire) => {
    if (!isWireScreenshot(wire)) {
      throw invalidArgument("get_screenshot returned an invalid WebP screenshot.");
    }
    const bytes = Uint8Array.from(Buffer.from(wire.bytes_base64, "base64"));
    if (bytes.length === 0) {
      throw invalidArgument("get_screenshot returned empty WebP bytes.");
    }
    const dataUrl = `data:image/webp;base64,${wire.bytes_base64}` as const;
    if (wire.data_url !== undefined && wire.data_url !== dataUrl) {
      throw invalidArgument("get_screenshot data_url does not match bytes_base64.");
    }
    return {
      filepath: wire.filepath,
      bytes,
      data_url: dataUrl
    };
  });
}
