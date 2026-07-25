import { invalidArgument } from "../errors";
import {
  type AppShotCaptureRequest,
  type AppShotCaptureResult,
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
import type {
  ActivateWindowRequest,
  WindowActionOutcome,
  WindowTarget
} from "../window-action";

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

export type ActivateWindowInput = WindowTarget;

export type AppShotCaptureInput = Omit<AppShotCaptureRequest, "type">;

export type LinuxClient = {
  activate_window(input: ActivateWindowInput): Promise<WindowActionOutcome>;
  appshot_capture(input: AppShotCaptureInput): Promise<AppShotCaptureResult>;
  click(input: ClickInput): Promise<void>;
  drag(input: DragInput): Promise<void>;
  get_screenshot(): Promise<ReturnType<typeof decodeScreenshots>>;
  move(input: MoveInput): Promise<void>;
  press_key(input: PressKeyInput): Promise<void>;
  scroll(input: ScrollInput): Promise<void>;
  type_text(input: TypeTextInput): Promise<void>;
};

const LINUX_KEYS = [
  "activate_window",
  "appshot_capture",
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

const WINDOW_TARGET_FIELDS = [
  "window_id",
  "pid",
  "tty",
  "terminal_pid",
  "terminal_command",
  "terminal_cwd",
  "app_id",
  "wm_class",
  "title"
] as const;

function normalizeWindowTarget(input: ActivateWindowInput): WindowTarget {
  const value = recordInput(input, "activate_window");
  assertKeys(value, WINDOW_TARGET_FIELDS, "activate_window");
  const target: WindowTarget = {};
  for (const field of WINDOW_TARGET_FIELDS) {
    const candidate = value[field];
    if (candidate === undefined) {
      continue;
    }
    if (field === "pid" || field === "terminal_pid") {
      if (!Number.isInteger(candidate) || (candidate as number) < 1 || (candidate as number) > 0xffff_ffff) {
        throw invalidArgument(`${field} must be an integer between 1 and 4294967295.`);
      }
      target[field] = candidate as number;
      continue;
    }
    if (typeof candidate !== "string" || candidate.trim().length === 0) {
      throw invalidArgument(`${field} must be a non-empty string when supplied.`);
    }
    target[field] = candidate.trim();
  }
  if (Object.keys(target).length === 0) {
    throw invalidArgument(
      "activate_window requires window_id, pid, app_id, wm_class, title, or a terminal selector."
    );
  }
  return target;
}

function windowActionOutcome(value: unknown): WindowActionOutcome {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw invalidArgument("Sky-cua service returned an invalid activate_window outcome.");
  }
  const outcome = value as Record<string, unknown>;
  if (
    typeof outcome.success !== "boolean" ||
    typeof outcome.message !== "string" ||
    typeof outcome.code !== "string" ||
    !Array.isArray(outcome.diagnostics) ||
    outcome.diagnostics.some((diagnostic) => {
      if (typeof diagnostic !== "object" || diagnostic === null || Array.isArray(diagnostic)) {
        return true;
      }
      const record = diagnostic as Record<string, unknown>;
      return typeof record.code !== "string" ||
        typeof record.message !== "string" ||
        (record.details !== undefined && typeof record.details !== "string");
    })
  ) {
    throw invalidArgument("Sky-cua service returned an invalid activate_window outcome.");
  }
  return value as WindowActionOutcome;
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
    async activate_window(input: ActivateWindowInput): Promise<WindowActionOutcome> {
      const context = requestContext();
      const request: ActivateWindowRequest = {
        type: "activate_window",
        target: normalizeWindowTarget(input),
        context
      };
      const response = await withSuspendedTimeout(async () => transport.request(request, {
        context,
        requiredCapabilities: ["linux.activate_window", "turn.cancel"]
      }));
      if (response.type !== "activate_window") {
        throw invalidArgument("Sky-cua service returned an invalid activate_window response.");
      }
      const outcome = windowActionOutcome(response.outcome);
      setComputerUseResponseMeta();
      return outcome;
    },

    async appshot_capture(input: AppShotCaptureInput): Promise<AppShotCaptureResult> {
      const value = recordInput(input, "appshot_capture");
      assertKeys(value, ["request_id", "target", "frontmost", "flags"], "appshot_capture");
      if (
        typeof value.request_id !== "string" ||
        !/^(?!\.)[A-Za-z0-9._-]{1,128}$/.test(value.request_id)
      ) {
        throw invalidArgument(
          "request_id must contain 1-128 ASCII letters, digits, '-', '_', or '.', and may not start with '.'."
        );
      }
      const frontmost = value.frontmost === true;
      if (value.frontmost !== undefined && typeof value.frontmost !== "boolean") {
        throw invalidArgument("frontmost must be a boolean when supplied.");
      }
      const target = value.target === undefined
        ? undefined
        : normalizeWindowTarget(value.target as WindowTarget);
      if ((target !== undefined) === frontmost) {
        throw invalidArgument(
          "appshot_capture requires exactly one target selector: target or frontmost=true."
        );
      }
      let flags: AppShotCaptureInput["flags"];
      if (value.flags !== undefined) {
        const flagValues = recordInput(value.flags, "appshot_capture flags");
        assertKeys(flagValues, ["include_ax_text"], "appshot_capture flags");
        if (
          flagValues.include_ax_text !== undefined &&
          typeof flagValues.include_ax_text !== "boolean"
        ) {
          throw invalidArgument("include_ax_text must be a boolean when supplied.");
        }
        flags = {
          ...(flagValues.include_ax_text === undefined
            ? {}
            : { include_ax_text: flagValues.include_ax_text })
        };
      }
      const response = await withSuspendedTimeout(async () => transport.request({
        type: "appshot_capture",
        request_id: value.request_id as string,
        ...(target === undefined ? {} : { target }),
        ...(frontmost ? { frontmost: true } : {}),
        ...(flags === undefined ? {} : { flags })
      }, {
        requiredCapabilities: ["appshot_capture.v1"]
      }));
      if (
        response.type !== "appshot_capture" ||
        response.result.request_id !== value.request_id ||
        response.result.capture_scope !== "window" ||
        typeof response.result.image.path !== "string" ||
        response.result.image.path.length === 0 ||
        response.result.image.size_bytes <= 0 ||
        response.result.image.dimensions.width <= 0 ||
        response.result.image.dimensions.height <= 0
      ) {
        throw invalidArgument("Sky-cua service returned an invalid appshot_capture response.");
      }
      return response.result;
    },

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
