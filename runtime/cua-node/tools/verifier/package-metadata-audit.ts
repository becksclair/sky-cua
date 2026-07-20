import { record } from "./json";
import type { JsonRecord } from "./types";

export type WrongPlatformOptionalDependency = {
  dependency: string;
  platform: "darwin" | "windows" | "arm" | "musl";
};

function dependencyNameTokens(dependency: string): string[] {
  return dependency
    .toLowerCase()
    .split(/[@/._-]+/u)
    .filter((token) => token.length > 0);
}

function wrongPlatform(
  dependency: string,
): WrongPlatformOptionalDependency["platform"] | null {
  const tokens = dependencyNameTokens(dependency);
  if (tokens.includes("darwin")) return "darwin";
  if (tokens.includes("win32") || tokens.includes("windows")) return "windows";
  if (tokens.some((token) => /^arm(?:64|v\d+l?)?$/u.test(token))) return "arm";
  if (
    tokens.some(
      (token) =>
        token === "musl" || token === "linuxmusl" || token.endsWith("musl"),
    )
  )
    return "musl";
  return null;
}

export function findWrongPlatformOptionalDependencies(
  packageJson: JsonRecord,
  label: string,
): WrongPlatformOptionalDependency[] {
  if (packageJson.optionalDependencies === undefined) return [];
  const optionalDependencies = record(
    packageJson.optionalDependencies,
    `${label}.optionalDependencies`,
  );
  return Object.keys(optionalDependencies)
    .map((dependency): WrongPlatformOptionalDependency | null => {
      const platform = wrongPlatform(dependency);
      return platform === null ? null : { dependency, platform };
    })
    .filter((issue): issue is WrongPlatformOptionalDependency => issue !== null)
    .sort((left, right) => left.dependency.localeCompare(right.dependency));
}
