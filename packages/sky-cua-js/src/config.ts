import { readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { invalidArgument, targetUnavailable, SkyCuaError } from "./errors";
import {
  DEFAULT_MOUSE_SIZE_PX,
  DEFAULT_POST_ACTION_SLEEP_MS
} from "./protocol/generated";

export type SkyTarget = "linux" | "mac";

export type SkyConfig = {
  target: SkyTarget;
  post_action_sleep_ms: number;
  mouse_size_px: number;
  service_socket_path?: string;
};

type ConfigFile = {
  target?: unknown;
  platform?: unknown;
  post_action_sleep_ms?: unknown;
  mouse_size_px?: unknown;
  service_socket_path?: unknown;
  serviceSocketPath?: unknown;
  socket_path?: unknown;
};

const CONFIG_ENV_NAMES = ["OAI_SKY_CONFIG_PATH", "SKY_CUA_JS_CONFIG_PATH"] as const;
const CONFIG_KEYS = new Set([
  "target",
  "platform",
  "post_action_sleep_ms",
  "mouse_size_px",
  "service_socket_path",
  "serviceSocketPath",
  "socket_path"
]);

function firstNonEmptyEnvironment(names: readonly string[]): string | undefined {
  for (const name of names) {
    const value = process.env[name];
    if (typeof value === "string" && value.length > 0) {
      return value;
    }
  }
  return undefined;
}

function parseConfigFile(path: string): ConfigFile {
  let contents: string;
  try {
    contents = readFileSync(path, "utf8");
  } catch (cause) {
    throw new SkyCuaError(
      "SKY_CUA_INVALID_ARGUMENT",
      `Unable to read sky configuration at ${path}.`,
      { cause }
    );
  }

  let value: unknown;
  try {
    value = JSON.parse(contents);
  } catch (cause) {
    throw new SkyCuaError(
      "SKY_CUA_INVALID_ARGUMENT",
      `Sky configuration at ${path} is not valid JSON.`,
      { cause }
    );
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw invalidArgument("Sky configuration must be a JSON object.");
  }
  for (const key of Object.keys(value)) {
    if (!CONFIG_KEYS.has(key)) {
      throw invalidArgument(`Sky configuration contains unsupported field ${key}.`);
    }
  }
  return value as ConfigFile;
}

function configuredTarget(value: unknown, platform: string): SkyTarget {
  const target = value === undefined ? platform : value;
  if (target === "linux") {
    return "linux";
  }
  if (target === "darwin" || target === "mac") {
    return "mac";
  }
  throw targetUnavailable(typeof target === "string" ? target : platform);
}

function boundedInteger(
  value: unknown,
  name: string,
  minimum: number,
  maximum: number,
  fallback: number
): number {
  if (value === undefined) {
    return fallback;
  }
  if (
    typeof value !== "number" ||
    !Number.isInteger(value) ||
    value < minimum ||
    value > maximum
  ) {
    throw invalidArgument(`${name} must be an integer between ${minimum} and ${maximum}.`);
  }
  return value;
}

export function resolveSkyConfig(): SkyConfig {
  const configPath = firstNonEmptyEnvironment(CONFIG_ENV_NAMES);
  const file = configPath === undefined ? {} : parseConfigFile(configPath);
  const target = configuredTarget(file.target ?? file.platform, process.platform);
  const serviceSocketPath = file.service_socket_path ?? file.serviceSocketPath ?? file.socket_path;
  if (
    serviceSocketPath !== undefined &&
    (typeof serviceSocketPath !== "string" || serviceSocketPath.length === 0)
  ) {
    throw invalidArgument("service_socket_path must be a non-empty string.");
  }
  return {
    target,
    post_action_sleep_ms: boundedInteger(
      file.post_action_sleep_ms,
      "post_action_sleep_ms",
      0,
      30_000,
      DEFAULT_POST_ACTION_SLEEP_MS
    ),
    mouse_size_px: boundedInteger(
      file.mouse_size_px,
      "mouse_size_px",
      0,
      128,
      DEFAULT_MOUSE_SIZE_PX
    ),
    ...(typeof serviceSocketPath === "string"
      ? { service_socket_path: serviceSocketPath }
      : {})
  };
}

export function resolveServiceSocketPath(config?: SkyConfig): string {
  const explicit = process.env.SKY_CUA_SERVICE_SOCKET_PATH;
  if (typeof explicit === "string" && explicit.length > 0) {
    return explicit;
  }
  if (config?.service_socket_path !== undefined) {
    return config.service_socket_path;
  }
  const runtimeDir = process.env.XDG_RUNTIME_DIR;
  if (typeof runtimeDir === "string" && runtimeDir.length > 0) {
    return join(runtimeDir, "sky-cua", "service.sock");
  }
  const cacheDir = process.env.XDG_CACHE_HOME;
  if (typeof cacheDir === "string" && cacheDir.length > 0) {
    return join(cacheDir, "sky-cua", "service.sock");
  }
  const home = process.env.HOME;
  if (typeof home === "string" && home.length > 0) {
    return join(home, ".cache", "sky-cua", "service.sock");
  }
  const uid = typeof process.getuid === "function" ? process.getuid() : "unknown";
  return join(tmpdir(), `sky-cua-uid-${uid}`, "service.sock");
}
