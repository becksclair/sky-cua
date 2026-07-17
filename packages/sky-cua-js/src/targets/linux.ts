import { invalidArgument } from "../errors";
import {
  type ClickRequest,
  type CuaJsCapability,
  type DragRequest,
  type GetScreenshotResponse,
  type MoveRequest,
  type PressKeyRequest,
  type ScrollRequest,
  type TypeTextRequest
} from "../protocol/generated";
import { requestContext, setComputerUseResponseMeta, withSuspendedTimeout } from "../context";
import type { SkyConfig } from "../config";
import { decodeScreenshots } from "../screenshot";
import { SkyCuaTransport } from "../transport/ndjson-client";

export type LinuxOptions = Pick<SkyConfig, "post_action_sleep_ms" | "mouse_size_px">;

export type ClickInput = {
  x: number;
  y: number;
  mouse_button?: ClickRequest["mouse_button"];
  click_count?: number;
  key?: string;
};

export type DragInput = {
  from_x: number;
  from_y: number;
  to_x: number;
  to_y: number;
  key?: string;
};

export type MoveInput = {
  x: number;
  y: number;
  key?: string;
};

export type PressKeyInput = {
  key: string;
};

export type ScrollInput = {
  direction: ScrollRequest["direction"];
  pixels?: number;
  x?: number;
  y?: number;
  key?: string;
};

export type TypeTextInput = {
  text: string;
};

export type LinuxClient = {
  click(input: ClickInput): Promise<void>;
  drag(input: DragInput): Promise<void>;
  get_screenshot(): Promise<ReturnType<typeof decodeScreenshots>>;
  move(input: MoveInput): Promise<void>;
  press_key(input: PressKeyInput): Promise<void>;
  scroll(input: ScrollInput): Promise<void>;
  type_text(input: TypeTextInput): Promise<void>;
};

const LINUX_KEYS = [
  "click",
  "drag",
  "get_screenshot",
  "move",
  "press_key",
  "scroll",
  "type_text"
] as const;

function recordInput(value: unknown, name: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw invalidArgument(`${name} input must be a plain object.`);
  }
  return value as Record<string, unknown>;
}

function assertKeys(value: Record<string, unknown>, allowed: readonly string[], name: string): void {
  const permitted = new Set(allowed);
  for (const key of Object.keys(value)) {
    if (!permitted.has(key)) {
      throw invalidArgument(`${name} input contains unsupported field ${key}.`);
    }
  }
}

function finiteNumber(value: unknown, name: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw invalidArgument(`${name} must be a finite number.`);
  }
  return value;
}

function optionalHeldKey(value: unknown): string | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (typeof value !== "string" || value.length === 0) {
    throw invalidArgument("key must be a non-empty string when supplied.");
  }
  return value;
}

function optionalInteger(
  value: unknown,
  name: string,
  minimum: number,
  maximum: number
): number | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (!Number.isInteger(value) || (value as number) < minimum || (value as number) > maximum) {
    throw invalidArgument(`${name} must be an integer between ${minimum} and ${maximum}.`);
  }
  return value as number;
}

function requiredCapabilities(
  base: "linux.click" | "linux.drag" | "linux.move" | "linux.press_key" | "linux.scroll" | "linux.type_text",
  key?: string,
  extras: readonly ("linux.click.button" | "linux.click.count" | "linux.scroll.direction" | "linux.scroll.origin" | "linux.scroll.pixels")[] = []
): readonly CuaJsCapability[] {
  const result = new Set<CuaJsCapability>([
    base,
    "action.post_action_sleep_ms",
    "turn.cancel"
  ]);
  if (key !== undefined) {
    result.add("action.held_key");
  }
  for (const extra of extras) {
    result.add(extra);
  }
  return [...result];
}

function actionRequest(
  transport: SkyCuaTransport,
  request: ClickRequest | DragRequest | MoveRequest | PressKeyRequest | ScrollRequest | TypeTextRequest,
  capabilities: readonly CuaJsCapability[]
): Promise<void> {
  return withSuspendedTimeout(async () => {
    const context = request.context;
    await transport.request(request, {
      context,
      requiredCapabilities: capabilities
    });
    setComputerUseResponseMeta();
  });
}

export function createLinuxClient(config: SkyConfig): LinuxClient {
  const transport = new SkyCuaTransport(config);

  const client: LinuxClient = {
    async click(input: ClickInput): Promise<void> {
      const value = recordInput(input, "click");
      assertKeys(value, ["x", "y", "mouse_button", "click_count", "key"], "click");
      const x = finiteNumber(value.x, "x");
      const y = finiteNumber(value.y, "y");
      const mouseButton = value.mouse_button;
      if (
        mouseButton !== undefined &&
        mouseButton !== "left" &&
        mouseButton !== "right" &&
        mouseButton !== "middle" &&
        mouseButton !== "l" &&
        mouseButton !== "r" &&
        mouseButton !== "m"
      ) {
        throw invalidArgument("mouse_button must be left, right, middle, l, r, or m.");
      }
      const clickCount = optionalInteger(value.click_count, "click_count", 1, 100);
      const key = optionalHeldKey(value.key);
      const request: ClickRequest = {
        type: "click",
        context: requestContext(),
        x,
        y,
        ...(mouseButton === undefined ? {} : { mouse_button: mouseButton }),
        ...(clickCount === undefined ? {} : { click_count: clickCount }),
        ...(key === undefined ? {} : { key }),
        post_action_sleep_ms: config.post_action_sleep_ms
      };
      await actionRequest(
        transport,
        request,
        requiredCapabilities("linux.click", key, [
          ...(mouseButton === undefined ? [] : ["linux.click.button" as const]),
          ...(clickCount === undefined ? [] : ["linux.click.count" as const])
        ])
      );
    },

    async drag(input: DragInput): Promise<void> {
      const value = recordInput(input, "drag");
      assertKeys(value, ["from_x", "from_y", "to_x", "to_y", "key"], "drag");
      const key = optionalHeldKey(value.key);
      const request: DragRequest = {
        type: "drag",
        context: requestContext(),
        from_x: finiteNumber(value.from_x, "from_x"),
        from_y: finiteNumber(value.from_y, "from_y"),
        to_x: finiteNumber(value.to_x, "to_x"),
        to_y: finiteNumber(value.to_y, "to_y"),
        ...(key === undefined ? {} : { key }),
        post_action_sleep_ms: config.post_action_sleep_ms
      };
      await actionRequest(transport, request, requiredCapabilities("linux.drag", key));
    },

    async get_screenshot() {
      const context = optionalRequestContext();
      const request = {
        type: "get_screenshot" as const,
        ...(context === undefined ? {} : { context }),
        mouse_size_px: config.mouse_size_px
      };
      const response = await withSuspendedTimeout(async () =>
        transport.request(request, {
          context,
          requiredCapabilities: [
            "linux.get_screenshot",
            "screen.cursor_size",
            "screenshot.webp"
          ] as CuaJsCapability[]
        })
      );
      if (response.type === "error") {
        throw invalidArgument(response.message);
      }
      if (response.type !== "get_screenshot" || response.ok !== true) {
        throw invalidArgument("Sky-cua service returned an invalid get_screenshot response.");
      }
      return decodeScreenshots(response as GetScreenshotResponse);
    },

    async move(input: MoveInput): Promise<void> {
      const value = recordInput(input, "move");
      assertKeys(value, ["x", "y", "key"], "move");
      const key = optionalHeldKey(value.key);
      const request: MoveRequest = {
        type: "move",
        context: requestContext(),
        x: finiteNumber(value.x, "x"),
        y: finiteNumber(value.y, "y"),
        ...(key === undefined ? {} : { key }),
        post_action_sleep_ms: config.post_action_sleep_ms
      };
      await actionRequest(transport, request, requiredCapabilities("linux.move", key));
    },

    async press_key(input: PressKeyInput): Promise<void> {
      const value = recordInput(input, "press_key");
      assertKeys(value, ["key"], "press_key");
      if (typeof value.key !== "string" || value.key.length === 0) {
        throw invalidArgument("key must be a non-empty string.");
      }
      const request: PressKeyRequest = {
        type: "press_key",
        context: requestContext(),
        key: value.key,
        post_action_sleep_ms: config.post_action_sleep_ms
      };
      await actionRequest(
        transport,
        request,
        requiredCapabilities("linux.press_key")
      );
    },

    async scroll(input: ScrollInput): Promise<void> {
      const value = recordInput(input, "scroll");
      assertKeys(value, ["direction", "pixels", "x", "y", "key"], "scroll");
      const direction = value.direction;
      if (
        direction !== "up" &&
        direction !== "down" &&
        direction !== "left" &&
        direction !== "right" &&
        direction !== "u" &&
        direction !== "d" &&
        direction !== "l" &&
        direction !== "r"
      ) {
        throw invalidArgument("direction must be up, down, left, right, u, d, l, or r.");
      }
      const pixels = optionalInteger(value.pixels, "pixels", 1, 10_000);
      const hasX = value.x !== undefined;
      const hasY = value.y !== undefined;
      if (hasX !== hasY) {
        throw invalidArgument("scroll origin requires both x and y.");
      }
      const key = optionalHeldKey(value.key);
      const request: ScrollRequest = {
        type: "scroll",
        context: requestContext(),
        direction,
        ...(pixels === undefined ? {} : { pixels }),
        ...(hasX ? { x: finiteNumber(value.x, "x"), y: finiteNumber(value.y, "y") } : {}),
        ...(key === undefined ? {} : { key }),
        post_action_sleep_ms: config.post_action_sleep_ms
      };
      await actionRequest(
        transport,
        request,
        requiredCapabilities("linux.scroll", key, [
          "linux.scroll.direction",
          ...(hasX ? ["linux.scroll.origin" as const] : []),
          ...(pixels === undefined ? [] : ["linux.scroll.pixels" as const])
        ])
      );
    },

    async type_text(input: TypeTextInput): Promise<void> {
      const value = recordInput(input, "type_text");
      assertKeys(value, ["text"], "type_text");
      if (typeof value.text !== "string") {
        throw invalidArgument("text must be a string.");
      }
      const request: TypeTextRequest = {
        type: "type_text",
        context: requestContext(),
        text: value.text,
        post_action_sleep_ms: config.post_action_sleep_ms
      };
      await actionRequest(
        transport,
        request,
        requiredCapabilities("linux.type_text")
      );
    }
  };

  return client;
}

function optionalRequestContext(): ReturnType<typeof requestContext> | undefined {
  try {
    return requestContext();
  } catch (error) {
    if (
      typeof error === "object" &&
      error !== null &&
      "code" in error &&
      (error as { code?: unknown }).code === "SKY_CUA_INVALID_CONTEXT"
    ) {
      return undefined;
    }
    throw error;
  }
}

export function linuxOwnKeys(): readonly string[] {
  return LINUX_KEYS;
}
