import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { expectedMcpImageContent, fixtureBytes, fixturePath } from "./fixtures.ts";

export type FakeCheck = { id: string; detail: string };

export type McpMessage = {
  id?: number;
  method?: string;
  params?: Record<string, unknown>;
};

export type McpResponse = {
  id: number | undefined;
  result?: Record<string, unknown>;
  error?: { code: number; message: string; data?: Record<string, unknown> };
};

export class FakeMcpHost {
  private counter = 0;
  private generation = 1;

  async handleLine(line: string, signal?: AbortSignal): Promise<McpResponse> {
    let message: McpMessage;
    try {
      const parsed = JSON.parse(line) as unknown;
      if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("message must be an object");
      }
      message = parsed as McpMessage;
    } catch (error) {
      const detail = error instanceof Error ? error.message : "invalid JSON";
      return {
        id: undefined,
        error: { code: -32700, message: `Malformed MCP framing: ${detail}` },
      };
    }
    if (message.method === "initialize") {
      return {
        id: message.id,
        result: {
          protocolVersion: "2025-06-18",
          serverInfo: { name: "cua_node-fixture", version: "fixture-1" },
        },
      };
    }
    if (message.method === "tools/list") {
      return {
        id: message.id,
        result: { tools: ["js", "js_reset", "js_add_node_module_dir"] },
      };
    }
    if (message.method === "tools/call") {
      const name = message.params?.name;
      const args = message.params?.arguments;
      if (name === "js_reset") {
        this.counter = 0;
        this.generation += 1;
        return {
          id: message.id,
          result: {
            generation: this.generation,
            content: [{ type: "text", text: "reset" }],
          },
        };
      }
      if (name === "js_add_node_module_dir") {
        return {
          id: message.id,
          result: {
            content: [
              {
                type: "text",
                text: `added:${String((args as Record<string, unknown> | undefined)?.path ?? "")}`,
              },
            ],
          },
        };
      }
      if (name === "js") {
        const code = String((args as Record<string, unknown> | undefined)?.code ?? "");
        const timeout = Number(
          (args as Record<string, unknown> | undefined)?.timeout_ms ?? 30_000,
        );
        if (code === "emit-image") {
          return {
            id: message.id,
            result: {
              content: [expectedMcpImageContent()],
              generation: this.generation,
            },
          };
        }
        try {
          const value = await this.execute(code, timeout, signal);
          return {
            id: message.id,
            result: {
              content: [{ type: "text", text: String(value) }],
              generation: this.generation,
            },
          };
        } catch (error) {
          const detail = error instanceof Error ? error.message : String(error);
          return {
            id: message.id,
            error: {
              code: -32000,
              message: detail,
              data: { generation: this.generation },
            },
          };
        }
      }
    }
    return {
      id: message.id,
      error: {
        code: -32601,
        message: `Unsupported fixture method: ${message.method ?? "<missing>"}`,
      },
    };
  }

  async processTranscript(transcript: string): Promise<McpResponse[]> {
    const responses: McpResponse[] = [];
    for (const line of transcript.split("\n")) {
      if (line.length > 0) {
        responses.push(await this.handleLine(line));
      }
    }
    return responses;
  }

  private async execute(
    code: string,
    timeout: number,
    signal?: AbortSignal,
  ): Promise<string | number> {
    if (code === "counter") {
      this.counter += 1;
      return this.counter;
    }
    if (code === "throw") {
      throw new Error("fixture JavaScript error");
    }
    if (code === "timeout") {
      await waitFor(10);
      if (timeout < 10) {
        throw new Error("EXECUTION_TIMEOUT");
      }
      return "timed-out-fixture-did-not-timeout";
    }
    if (code === "cancel-loop") {
      await waitForAbort(signal);
      throw new Error("EXECUTION_CANCELLED");
    }
    if (code === "crash") {
      this.counter = 0;
      this.generation += 1;
      throw new Error("KERNEL_CRASHED");
    }
    if (code === "hostile-console") {
      return "captured-without-stdout-corruption";
    }
    return code;
  }
}

function waitFor(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function waitForAbort(signal?: AbortSignal): Promise<void> {
  if (signal?.aborted === true) {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    signal?.addEventListener("abort", () => resolve(), { once: true });
    setTimeout(resolve, 100);
  });
}

export function encodeNativeFrame(payload: Uint8Array): Buffer {
  const frame = Buffer.allocUnsafe(4 + payload.byteLength);
  frame.writeUInt32BE(payload.byteLength, 0);
  Buffer.from(payload).copy(frame, 4);
  return frame;
}

export function decodeNativeFrames(
  chunks: readonly Uint8Array[],
  maxFrameBytes = 1024,
): Uint8Array[] {
  const stream = Buffer.concat(chunks.map((chunk) => Buffer.from(chunk)));
  const frames: Uint8Array[] = [];
  let offset = 0;
  while (offset < stream.byteLength) {
    if (stream.byteLength - offset < 4) {
      throw new Error("NATIVE_PIPE_INCOMPLETE_HEADER");
    }
    const length = stream.readUInt32BE(offset);
    if (length > maxFrameBytes) {
      throw new Error("NATIVE_PIPE_FRAME_TOO_LARGE");
    }
    if (stream.byteLength - offset - 4 < length) {
      throw new Error("NATIVE_PIPE_INCOMPLETE_FRAME");
    }
    frames.push(stream.subarray(offset + 4, offset + 4 + length));
    offset += 4 + length;
  }
  return frames;
}

export class FakeBrowserNativePeer {
  connectCount = 0;
  closeCount = 0;
  pendingRejected = false;
  connected = false;

  connect(): void {
    this.connectCount += 1;
    this.connected = true;
  }

  receive(chunks: readonly Uint8Array[], maxFrameBytes = 1024): unknown[] {
    return decodeNativeFrames(chunks, maxFrameBytes).map(
      (frame) => JSON.parse(Buffer.from(frame).toString("utf8")) as unknown,
    );
  }

  close(): void {
    this.connected = false;
    this.closeCount += 1;
    this.pendingRejected = true;
  }
}

export function trustedBrowserRequest(options: {
  clientBytes: Uint8Array;
  suppliedHash: string;
  peer: FakeBrowserNativePeer;
  payload: Record<string, unknown>;
}): unknown {
  const actualHash = createSha256(options.clientBytes);
  if (options.suppliedHash !== actualHash) {
    throw new Error("TRUSTED_BROWSER_HASH_REJECTED");
  }
  options.peer.connect();
  const frame = encodeNativeFrame(Buffer.from(JSON.stringify(options.payload), "utf8"));
  return options.peer.receive([
    frame.subarray(0, 2),
    frame.subarray(2, 7),
    frame.subarray(7),
  ]);
}

function createSha256(bytes: Uint8Array): string {
  return createHash("sha256").update(bytes).digest("hex");
}

export type PageState = {
  clicked: boolean;
  typed: string;
  scrollTop: number;
  scrollLeft: number;
};

export class FakeBrowserPage {
  readonly state: PageState = {
    clicked: false,
    typed: "",
    scrollTop: 0,
    scrollLeft: 0,
  };
  readonly html: string;

  constructor(path: string) {
    this.html = readFileSync(path, "utf8");
  }

  inspect(selector: string): { selector: string; found: boolean; text: string } {
    const found = this.html.includes(`id="${selector.replace(/^#/u, "")}"`);
    return { selector, found, text: found ? "fixture element" : "" };
  }

  click(selector: string): void {
    if (!this.inspect(selector).found) {
      throw new Error(`element not found: ${selector}`);
    }
    this.state.clicked = true;
  }

  type(selector: string, text: string): void {
    if (!this.inspect(selector).found) {
      throw new Error(`element not found: ${selector}`);
    }
    this.state.typed = text;
  }

  scroll(delta: { top?: number; left?: number }): void {
    this.state.scrollTop += delta.top ?? 0;
    this.state.scrollLeft += delta.left ?? 0;
  }
}

export class FakeSkyService {
  running = true;
  readonly actions: Array<{ op: string; payload: Record<string, unknown> }> = [];
  private readonly cancelled = new Set<string>();
  private ambiguousClick = true;

  async request(
    op: string,
    payload: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    if (!this.running) {
      throw new Error("SKY_CUA_SERVICE_RESTART_REQUIRED");
    }
    const context = payload.context as Record<string, unknown> | undefined;
    if (
      op !== "Health" &&
      op !== "Screenshot" &&
      (context?.session_id === undefined || context?.turn_id === undefined)
    ) {
      throw new Error("SKY_CUA_CONTEXT_REQUIRED");
    }
    this.actions.push({ op, payload });
    const turn = String(context?.turn_id ?? "");
    if (payload.cancel_probe === true) {
      await waitFor(12);
      if (this.cancelled.has(turn)) {
        throw new Error("SKY_CUA_CANCELLED");
      }
    }
    if (op === "Click" && payload.ambiguous === true && this.ambiguousClick) {
      this.ambiguousClick = false;
      throw new Error("SKY_CUA_ACTION_OUTCOME_UNKNOWN");
    }
    if (op === "Screenshot") {
      return {
        filepath: "fixture://screen.webp",
        mime: "image/webp",
        width: 2,
        height: 1,
        pixels: [
          [51, 102, 255, 255],
          [51, 102, 255, 255],
        ],
      };
    }
    return { ok: true, op };
  }

  cancel(turnId: string): void {
    this.cancelled.add(turnId);
  }
}

export class FakeSkyClient {
  constructor(private readonly service: FakeSkyService) {}

  health(): Promise<Record<string, unknown>> {
    return this.service.request("Health", {});
  }
  screenshot(): Promise<Record<string, unknown>> {
    return this.service.request("Screenshot", {});
  }
  move(
    payload: Record<string, unknown>,
    context: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    return this.mutate("Move", payload, context);
  }
  click(
    payload: Record<string, unknown>,
    context: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    return this.mutate("Click", payload, context);
  }
  drag(
    payload: Record<string, unknown>,
    context: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    return this.mutate("Drag", payload, context);
  }
  pressKey(
    payload: Record<string, unknown>,
    context: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    return this.mutate("PressKey", payload, context);
  }
  typeText(
    payload: Record<string, unknown>,
    context: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    return this.mutate("TypeText", payload, context);
  }
  scroll(
    payload: Record<string, unknown>,
    context: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    return this.mutate("Scroll", payload, context);
  }
  cancel(turnId: string): void {
    this.service.cancel(turnId);
  }

  private mutate(
    op: string,
    payload: Record<string, unknown>,
    context: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    return this.service.request(op, { ...payload, context });
  }
}

export function fakeOcr(imagePath: string): { text: string; confidence: number } {
  const bytes = readFileSync(imagePath);
  const expected = Buffer.from(fixtureBytes("media/ocr-known.ppm"));
  if (!bytes.equals(expected)) {
    throw new Error("OCR_FIXTURE_IMAGE_MISMATCH");
  }
  const text = readFileSync(fixturePath("media/known-text.txt"), "utf8").trim();
  return { text, confidence: 0.99 };
}

export function fakePdf(pdfPath: string): {
  text: string;
  hasText: boolean;
  hasVector: boolean;
  hasRaster: boolean;
  pixels: string;
} {
  const source = readFileSync(pdfPath, "utf8");
  const hasText = source.includes("Cua Node acceptance PDF");
  const hasVector = source.includes(" re S");
  const hasRaster = source.includes("/Subtype/Image");
  if (!hasText || !hasVector || !hasRaster) {
    throw new Error("PDF_FIXTURE_CONTENT_MISSING");
  }
  return {
    text: "Cua Node acceptance PDF",
    hasText,
    hasVector,
    hasRaster,
    pixels: "fixture-pdf-raster-v1",
  };
}

export function fakeImageTransform(path: string): {
  input: string;
  output_mime: string;
  pixel: [number, number, number, number];
  expectation: string;
} {
  const bytes = readFileSync(path);
  const input = path.endsWith(".png")
    ? "image/png"
    : path.endsWith(".jpg")
      ? "image/jpeg"
      : path.endsWith(".webp")
        ? "image/webp"
        : "image/svg+xml";
  const output =
    input === "image/png"
      ? "image/jpeg"
      : input === "image/jpeg"
        ? "image/webp"
        : "image/png";
  const pixel: [number, number, number, number] =
    input === "image/png"
      ? [255, 0, 0, 255]
      : input === "image/jpeg"
        ? [255, 255, 255, 255]
        : input === "image/webp"
          ? [0, 0, 255, 255]
          : [51, 102, 255, 255];
  if (bytes.byteLength < 12) {
    throw new Error("IMAGE_FIXTURE_TOO_SMALL");
  }
  return { input, output_mime: output, pixel, expectation: `1x1:${pixel.join(",")}` };
}

export function fakePlaywright(pagePath: string): {
  url: string;
  clicked: boolean;
  typed: string;
  scrolled: boolean;
} {
  const page = new FakeBrowserPage(pagePath);
  page.click("#action");
  page.type("#name", "playwright fixture");
  page.scroll({ top: 240 });
  return {
    url: `file://${pagePath}`,
    clicked: page.state.clicked,
    typed: page.state.typed,
    scrolled: page.state.scrollTop === 240,
  };
}
