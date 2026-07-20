import { Buffer } from "node:buffer";
import { readFile } from "node:fs/promises";

import { invalidArgument } from "../errors";
import type { PhoneImage, PhoneScreenshotResponse } from "./protocol";

function decodeInline(image: PhoneImage): Uint8Array {
  if (typeof image.mime_type !== "string" || !image.mime_type.startsWith("image/") ||
    typeof image.data_base64 !== "string" || image.data_base64.length === 0) {
    throw invalidArgument("Phone screenshot returned an invalid inline image.");
  }
  const bytes = Uint8Array.from(Buffer.from(image.data_base64, "base64"));
  if (bytes.length === 0) {
    throw invalidArgument("Phone screenshot returned empty inline image bytes.");
  }
  return bytes;
}

export class PhoneScreenshot {
  readonly response: Readonly<PhoneScreenshotResponse>;
  readonly path?: string;
  readonly mimeType: string;
  readonly inlineBytes?: Uint8Array;
  readonly inlineDataUrl?: `data:${string};base64,${string}`;

  constructor(response: PhoneScreenshotResponse) {
    this.response = Object.freeze({ ...response });
    this.path = response.screenshot_path;
    this.mimeType = response.inline_image?.mime_type ?? mimeTypeFromPath(response.screenshot_path);
    if (response.inline_image !== undefined) {
      this.inlineBytes = decodeInline(response.inline_image);
      this.inlineDataUrl = `data:${this.mimeType};base64,${response.inline_image.data_base64}`;
    }
    if (this.inlineBytes === undefined && this.path === undefined) {
      throw invalidArgument("Phone screenshot returned neither inline image data nor a local path.");
    }
  }

  async bytes(): Promise<Uint8Array> {
    if (this.inlineBytes !== undefined) {
      return Uint8Array.from(this.inlineBytes);
    }
    try {
      return Uint8Array.from(await readFile(this.path!));
    } catch (cause) {
      throw new SkyPhoneFileError(`Unable to read Phone screenshot at ${this.path}.`, this.path!, cause);
    }
  }

  async dataUrl(): Promise<`data:${string};base64,${string}`> {
    if (this.inlineDataUrl !== undefined) {
      return this.inlineDataUrl;
    }
    return `data:${this.mimeType};base64,${Buffer.from(await this.bytes()).toString("base64")}`;
  }

  async emitImage(): Promise<unknown> {
    const emit = globalThis.nodeRepl?.emitImage;
    if (typeof emit !== "function") {
      throw invalidArgument("nodeRepl.emitImage is unavailable in this runtime.");
    }
    return await emit(await this.dataUrl());
  }
}

function mimeTypeFromPath(path: string | undefined): string {
  const lower = path?.toLowerCase();
  if (lower?.endsWith(".webp")) return "image/webp";
  if (lower?.endsWith(".jpg") || lower?.endsWith(".jpeg")) return "image/jpeg";
  return "image/png";
}

export class SkyPhoneFileError extends Error {
  readonly code = "SKY_CUA_PHONE_FILE_READ_FAILED" as const;
  readonly path: string;

  constructor(message: string, path: string, cause?: unknown) {
    super(message, { cause });
    this.name = "SkyPhoneFileError";
    this.path = path;
  }
}
