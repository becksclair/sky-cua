import type { ServiceErrorCode, ServiceErrorRetry } from "./protocol/generated";

export class SkyCuaError extends Error {
  readonly code: ServiceErrorCode;
  readonly retry: ServiceErrorRetry;
  readonly session_id?: string;
  readonly turn_id?: string;

  constructor(
    code: ServiceErrorCode,
    message: string,
    options: {
      retry?: ServiceErrorRetry;
      session_id?: string;
      turn_id?: string;
      cause?: unknown;
    } = {}
  ) {
    super(message, { cause: options.cause });
    this.name = "SkyCuaError";
    this.code = code;
    this.retry = options.retry ?? "never";
    this.session_id = options.session_id;
    this.turn_id = options.turn_id;
  }
}

export function isSkyCuaError(value: unknown): value is SkyCuaError {
  return value instanceof SkyCuaError;
}

export function errorFromService(
  response: {
    code: ServiceErrorCode;
    message: string;
    retry?: ServiceErrorRetry;
    session_id?: string;
    turn_id?: string;
  }
): SkyCuaError {
  return new SkyCuaError(response.code, response.message, response);
}

export function targetUnavailable(platform: string): SkyCuaError {
  return new SkyCuaError(
    "SKY_CUA_TARGET_UNAVAILABLE",
    `The sky-cua compatibility target is unavailable on ${platform}.`
  );
}

export function invalidArgument(message: string): SkyCuaError {
  return new SkyCuaError("SKY_CUA_INVALID_ARGUMENT", message);
}

export function invalidContext(message: string): SkyCuaError {
  return new SkyCuaError("SKY_CUA_INVALID_CONTEXT", message);
}
