import { resolveSkyConfig } from "./config";
import { createLinuxClient, linuxOwnKeys, type LinuxClient } from "./targets/linux";
import { createMacPlaceholder, macOwnKeys, type MacPlaceholderClient } from "./targets/mac-placeholder";

export type { SkyConfig, SkyTarget } from "./config";
export type {
  ActivateWindowInput,
  ClickInput,
  DragInput,
  LinuxClient,
  LinuxOptions,
  MoveInput,
  PressKeyInput,
  ScrollInput,
  TypeTextInput
} from "./targets/linux";
export type { WindowActionDiagnostic, WindowActionOutcome, WindowTarget } from "./window-action";

type SkyClient = LinuxClient | MacPlaceholderClient;

type ResolvedSky = {
  readonly client: SkyClient;
  readonly keys: readonly string[];
};

function propertyValue(resolved: ResolvedSky, property: string): unknown {
  if (!resolved.keys.includes(property)) {
    return undefined;
  }
  return resolved.client[property as keyof SkyClient];
}

function lazySky(): SkyClient {
  const target = {};
  let resolved: ResolvedSky | undefined;
  const resolve = (): ResolvedSky => {
    if (resolved !== undefined) {
      return resolved;
    }
    const config = resolveSkyConfig();
    resolved = config.target === "linux"
      ? { client: createLinuxClient(config), keys: linuxOwnKeys() }
      : { client: createMacPlaceholder(), keys: macOwnKeys() };
    return resolved;
  };
  return new Proxy(target, {
    get(_target, property: string | symbol): unknown {
      const state = resolve();
      if (property === "then" || typeof property === "symbol") {
        return undefined;
      }
      return propertyValue(state, property);
    },
    has(_target, property: string | symbol): boolean {
      const state = resolve();
      if (typeof property !== "string") {
        return false;
      }
      return state.keys.includes(property);
    },
    ownKeys(_target): string[] {
      return [...resolve().keys];
    },
    getOwnPropertyDescriptor(_target, property: string | symbol): PropertyDescriptor | undefined {
      const state = resolve();
      if (typeof property !== "string") {
        return undefined;
      }
      if (!state.keys.includes(property)) {
        return undefined;
      }
      return {
        configurable: true,
        enumerable: true,
        writable: false,
        value: propertyValue(state, property)
      };
    },
    getPrototypeOf(): object {
      resolve();
      return Object.prototype;
    }
  }) as SkyClient;
}

export const sky = lazySky();
