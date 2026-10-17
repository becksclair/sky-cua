import productionPackage from "./package.json" with { type: "json" };

const PRODUCTION_PINS: Readonly<Record<string, string>> = productionPackage.dependencies;

export function productionPin(name: string): string {
  const version = PRODUCTION_PINS[name];
  if (version === undefined) throw new Error(`production/package.json does not pin ${name}`);
  return version;
}
