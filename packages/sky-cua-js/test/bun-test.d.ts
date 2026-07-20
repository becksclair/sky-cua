declare module "bun:test" {
  type Matchers = {
    toBe(expected: unknown): void;
    toEqual(expected: unknown): void;
  };

  export function describe(name: string, callback: () => void): void;
  export function test(name: string, callback: () => void | Promise<void>): void;
  export function afterEach(callback: () => void | Promise<void>): void;
  export function expect(value: unknown): Matchers;
}

declare const Bun: {
  spawnSync(
    command: readonly string[],
    options?: {
      cwd?: string;
      env?: Record<string, string>;
      stdout?: "pipe" | "inherit";
      stderr?: "pipe" | "inherit";
    }
  ): { exitCode: number; stdout: Uint8Array; stderr: Uint8Array };
};
