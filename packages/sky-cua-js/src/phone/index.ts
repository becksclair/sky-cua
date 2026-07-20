import { createPhoneClient, PhoneClient } from "./client";

export { createPhoneClient, PhoneClient, PhoneDeviceSession, PhoneDisconnectedError } from "./client";
export type { PhoneClientOptions } from "./client";
export { PhoneScreenshot, SkyPhoneFileError } from "./screenshot";
export { SkyCuaError, isSkyCuaError } from "../errors";
export type { CallerProvenance, McpClientInfo, PhoneRequestContext } from "../context";
export type * from "./protocol";

const PHONE_KEYS = [
  "request", "status", "list_devices", "pair_wireless", "connect", "bind", "observe",
  "refresh_capabilities", "disconnect", "screenshot", "tap", "swipe", "type_text",
  "press_key", "install_companion", "companion_status", "accessibility_tree", "notifications",
  "notification_open", "notification_dismiss", "notification_action", "notification_reply",
  "app_current", "app_list", "app_launch", "app_open_intent", "app_force_stop", "app_install",
  "open_settings", "disconnected", "close"
] as const;

function lazyPhone(): PhoneClient {
  let client: PhoneClient | undefined;
  const resolve = (): PhoneClient => client ??= createPhoneClient();
  return new Proxy({}, {
    get(_target, property: string | symbol): unknown {
      if (property === "then" || typeof property === "symbol") return undefined;
      if (!PHONE_KEYS.includes(property as typeof PHONE_KEYS[number])) return undefined;
      const value = resolve()[property as keyof PhoneClient];
      return typeof value === "function" ? value.bind(resolve()) : value;
    },
    has(_target, property: string | symbol): boolean {
      return typeof property === "string" && PHONE_KEYS.includes(property as typeof PHONE_KEYS[number]);
    },
    ownKeys(): string[] {
      return [...PHONE_KEYS];
    },
    getOwnPropertyDescriptor(_target, property: string | symbol): PropertyDescriptor | undefined {
      if (typeof property !== "string" || !PHONE_KEYS.includes(property as typeof PHONE_KEYS[number])) return undefined;
      const value = resolve()[property as keyof PhoneClient];
      return { configurable: true, enumerable: true, writable: false, value: typeof value === "function" ? value.bind(resolve()) : value };
    }
  }) as PhoneClient;
}

/** Lazily resolves the host-owned service socket on first use. */
export const phone = lazyPhone();
