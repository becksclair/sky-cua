declare const process: {
  readonly execPath: string;
  readonly platform: string;
  readonly env: Record<string, string | undefined>;
  getuid?: () => number;
  cwd: () => string;
  on(event: "unhandledRejection", listener: (error: unknown) => void): void;
  off(event: "unhandledRejection", listener: (error: unknown) => void): void;
};

declare module "node:crypto" {
  export interface Hash {
    update(data: Uint8Array | string): Hash;
    digest(encoding: "hex"): string;
  }

  export function createHash(algorithm: string): Hash;
}

declare module "node:buffer" {
  export class Buffer extends Uint8Array {
    static alloc(size: number): Buffer;
    static byteLength(value: string, encoding?: string): number;
    static concat(list: readonly Uint8Array[]): Buffer;
    static from(value: string, encoding?: string): Buffer;
    static from(value: Uint8Array): Buffer;
    static from(value: readonly number[]): Buffer;
    toString(encoding?: string): string;
    indexOf(value: number): number;
    subarray(begin?: number, end?: number): Buffer;
  }
}

declare module "node:fs" {
  export function chmodSync(path: string, mode: number): void;
  export function mkdtempSync(prefix: string): string;
  export function readFileSync(path: string): Uint8Array;
  export function readFileSync(path: string, encoding: "utf8"): string;
  export function rmSync(path: string, options: { recursive: boolean; force: boolean }): void;
  export function writeFileSync(path: string, data: string): void;
  export function unlinkSync(path: string): void;
}

declare module "node:os" {
  export function tmpdir(): string;
}

declare module "node:path" {
  export function join(...parts: string[]): string;
}

declare module "node:net" {
  export interface Socket {
    on(event: "data", listener: (chunk: Uint8Array) => void): this;
    on(event: "error", listener: (error: unknown) => void): this;
    on(event: "close", listener: () => void): this;
    once(event: "connect", listener: () => void): this;
    once(event: "error", listener: (error: unknown) => void): this;
    once(event: "close", listener: () => void): this;
    off(event: "data", listener: (chunk: Uint8Array) => void): this;
    off(event: "error", listener: (error: unknown) => void): this;
    off(event: "close", listener: () => void): this;
    write(data: string | Uint8Array, callback?: (error?: Error) => void): boolean;
    destroy(): void;
  }

  export interface Server {
    once(event: "error", listener: (error: unknown) => void): this;
    listen(path: string, callback: () => void): this;
    close(callback: () => void): this;
  }

  export function createServer(listener: (socket: Socket) => void): Server;
  export function createConnection(options: { path: string }): Socket;
}

declare module "node:timers/promises" {
  export function setTimeout(milliseconds: number): Promise<void>;
}

declare module "node:zlib" {
  import { Buffer } from "node:buffer";
  export function gunzipSync(data: Uint8Array): Buffer;
}
