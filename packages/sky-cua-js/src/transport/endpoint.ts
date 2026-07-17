import { resolveServiceSocketPath } from "../config";
import type { SkyConfig } from "../config";

export function serviceEndpoint(config?: SkyConfig): string {
  return resolveServiceSocketPath(config);
}
