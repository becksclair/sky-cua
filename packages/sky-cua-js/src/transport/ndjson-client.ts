import { Buffer } from "node:buffer";
import { createConnection, type Socket } from "node:net";

import {
  CANCEL_TURN_STATUSES,
  MAX_FRAME_BYTES,
  type ActionType,
  type CuaJsCapability,
  type RequestContext,
  type ServiceRequest,
  type ServiceResponse
} from "../protocol/generated";
import { SkyCuaError, errorFromService } from "../errors";
import { serviceEndpoint } from "./endpoint";
import { isServiceError, requireCapabilities, validateHealth } from "./health";
import type { SkyConfig } from "../config";

export class NdjsonDisconnectError extends Error {
  readonly connected: boolean;
  readonly wrote: boolean;

  constructor(message: string, options: { connected: boolean; wrote: boolean; cause?: unknown }) {
    super(message, { cause: options.cause });
    this.name = "NdjsonDisconnectError";
    this.connected = options.connected;
    this.wrote = options.wrote;
  }
}

export class NdjsonConnection {
  private readonly socket: Socket;
  private readonly connected: boolean;
  private inFlight = false;
  private closed = false;
  private broken = false;

  private constructor(socket: Socket, connected: boolean) {
    this.socket = socket;
    this.connected = connected;
    this.socket.on("error", () => {
      this.broken = true;
    });
    this.socket.once("close", () => {
      this.closed = true;
    });
  }

  static async connect(endpoint: string): Promise<NdjsonConnection> {
    let settled = false;
    let socket: Socket | undefined;
    return await new Promise<NdjsonConnection>((resolve, reject) => {
      const onConnect = (): void => {
        if (settled || socket === undefined) {
          return;
        }
        settled = true;
        socket.off("error", onError);
        socket.off("close", onClose);
        resolve(new NdjsonConnection(socket, true));
      };
      const onError = (cause: unknown): void => {
        if (settled) {
          return;
        }
        settled = true;
        reject(
          new NdjsonDisconnectError(`Unable to connect to sky-cua service at ${endpoint}.`, {
            connected: false,
            wrote: false,
            cause
          })
        );
      };
      const onClose = (): void => {
        if (settled) {
          return;
        }
        settled = true;
        reject(
          new NdjsonDisconnectError(`Sky-cua service closed before connecting to ${endpoint}.`, {
            connected: false,
            wrote: false
          })
        );
      };
      socket = createConnection({ path: endpoint });
      socket.once("connect", onConnect);
      socket.once("error", onError);
      socket.once("close", onClose);
    });
  }

  async request(request: ServiceRequest): Promise<ServiceResponse> {
    if (this.closed || this.broken) {
      throw new NdjsonDisconnectError("Sky-cua service socket is already closed.", {
        connected: this.connected,
        wrote: false
      });
    }
    if (this.inFlight) {
      throw new Error("NDJSON service connections allow only one in-flight request.");
    }
    this.inFlight = true;
    const line = `${JSON.stringify(request)}\n`;
    const lineBytes = Buffer.byteLength(line);
    if (lineBytes > MAX_FRAME_BYTES) {
      this.inFlight = false;
      throw new SkyCuaError(
        "SKY_CUA_FRAME_TOO_LARGE",
        `Sky-cua request frame is ${lineBytes} bytes; the limit is ${MAX_FRAME_BYTES}.`
      );
    }

    return await new Promise<ServiceResponse>((resolve, reject) => {
      const segments: Buffer[] = [];
      let bufferedBytes = 0;
      let wrote = false;
      let settled = false;

      const cleanup = (): void => {
        this.socket.off("data", onData);
        this.socket.off("error", onError);
        this.socket.off("close", onClose);
        this.inFlight = false;
      };
      const fail = (error: unknown): void => {
        if (settled) {
          return;
        }
        settled = true;
        cleanup();
        reject(error);
      };
      const onError = (cause: unknown): void => {
        fail(
          new NdjsonDisconnectError("Sky-cua service socket reported an error.", {
            connected: this.connected,
            wrote,
            cause
          })
        );
      };
      const onClose = (): void => {
        fail(
          new NdjsonDisconnectError("Sky-cua service socket closed before a complete response.", {
            connected: this.connected,
            wrote
          })
        );
      };
      const onData = (chunk: Uint8Array): void => {
        if (settled) {
          return;
        }
        const incoming = Buffer.from(chunk);
        const newline = incoming.indexOf(0x0a);
        if (newline < 0) {
          bufferedBytes += incoming.length;
          if (bufferedBytes >= MAX_FRAME_BYTES) {
            this.socket.destroy();
            fail(
              new SkyCuaError(
                "SKY_CUA_FRAME_TOO_LARGE",
                `Sky-cua response frame exceeds ${MAX_FRAME_BYTES} bytes.`
              )
            );
            return;
          }
          segments.push(incoming);
          return;
        }

        const frameBytes = bufferedBytes + newline + 1;
        if (frameBytes > MAX_FRAME_BYTES) {
          this.socket.destroy();
          fail(
            new SkyCuaError(
              "SKY_CUA_FRAME_TOO_LARGE",
              `Sky-cua response frame is ${frameBytes} bytes; the limit is ${MAX_FRAME_BYTES}.`
            )
          );
          return;
        }

        const finalSegment = incoming.subarray(0, newline);
        let frameBuffer: Buffer;
        if (segments.length === 0) {
          frameBuffer = finalSegment;
        } else if (segments.length === 1 && finalSegment.length === 0) {
          frameBuffer = segments[0]!;
        } else {
          const frameSegments = finalSegment.length === 0
            ? segments
            : [...segments, finalSegment];
          frameBuffer = Buffer.concat(frameSegments);
        }

        let parsed: unknown;
        try {
          parsed = JSON.parse(frameBuffer.toString("utf8")) as unknown;
        } catch (cause) {
          this.socket.destroy();
          fail(
            new SkyCuaError("SKY_CUA_INVALID_REQUEST", "Sky-cua service returned malformed JSON.", {
              cause
            })
          );
          return;
        }
        if (
          typeof parsed !== "object" ||
          parsed === null ||
          typeof (parsed as { type?: unknown }).type !== "string"
        ) {
          this.socket.destroy();
          fail(
            new SkyCuaError(
              "SKY_CUA_INVALID_REQUEST",
              "Sky-cua service returned a response without a type."
            )
          );
          return;
        }
        settled = true;
        cleanup();
        resolve(parsed as ServiceResponse);
      };

      this.socket.on("data", onData);
      this.socket.once("error", onError);
      this.socket.once("close", onClose);
      try {
        wrote = true;
        this.socket.write(line);
      } catch (cause) {
        wrote = false;
        onError(cause);
      }
    });
  }

  close(): void {
    this.closed = true;
    this.socket.destroy();
    this.inFlight = false;
  }
}

type RequestState = {
  cancelled: boolean;
};

type RequestOptions = {
  context?: RequestContext;
  requiredCapabilities?: readonly CuaJsCapability[];
  signal?: AbortSignal;
};

const DEFAULT_CANCEL_WAIT_MS = 1_000;
const ACTION_TYPES = new Set<ActionType>([
  "click",
  "drag",
  "move",
  "press_key",
  "scroll",
  "type_text"
]);
const CANCEL_TURN_STATUS_SET = new Set(CANCEL_TURN_STATUSES);

export class SkyCuaTransport {
  private readonly endpoint: string;
  private actionConnection: NdjsonConnection | undefined;
  private health: ReturnType<typeof validateHealth> | undefined;
  private actionTail: Promise<void> = Promise.resolve();

  constructor(config?: SkyConfig) {
    this.endpoint = serviceEndpoint(config);
  }

  async request(request: ServiceRequest, options: RequestOptions = {}): Promise<ServiceResponse> {
    const state: RequestState = { cancelled: false };
    const run = (): Promise<ServiceResponse> => {
      if (options.context === undefined) {
        return this.performAction(request, options, state);
      }
      return this.withDeadline(
        () => this.performAction(request, options, state),
        options.context,
        state,
        options.signal
      );
    };
    const queued = this.actionTail.then(
      run,
      run
    );
    this.actionTail = queued.then(
      () => undefined,
      () => undefined
    );
    return queued;
  }

  async cancelTurn(
    context: RequestContext,
    reason: string
  ): Promise<Extract<ServiceResponse, { type: "cancel_turn" }>> {
    if (
      context.session_id.length === 0 ||
      context.turn_id.length === 0
    ) {
      throw new SkyCuaError(
        "SKY_CUA_CANCEL_TURN_INVALID_CONTEXT",
        "CancelTurn requires non-empty session_id and turn_id."
      );
    }
    if (reason.length === 0 || reason.length > 256) {
      throw new SkyCuaError(
        "SKY_CUA_CANCEL_TURN_INVALID_REASON",
        "CancelTurn reason must contain between 1 and 256 characters."
      );
    }
    const request: ServiceRequest = {
      type: "cancel_turn",
      session_id: context.session_id,
      turn_id: context.turn_id,
      reason
    };
    let lastFailure: NdjsonDisconnectError | undefined;
    for (let attempt = 0; attempt < 2; attempt += 1) {
      let connection: NdjsonConnection | undefined;
      try {
        connection = await NdjsonConnection.connect(this.endpoint);
        const response = await connection.request(request);
        if (isServiceError(response)) {
          if (response.retry === "safe_after_reconnect" && attempt === 0) {
            continue;
          }
          throw errorFromService(response);
        }
        if (
          response.type !== "cancel_turn" ||
          response.ok !== true ||
          response.session_id !== context.session_id ||
          response.turn_id !== context.turn_id ||
          !CANCEL_TURN_STATUS_SET.has(response.status)
        ) {
          throw new SkyCuaError(
            "SKY_CUA_INVALID_REQUEST",
            "Sky-cua service returned an invalid CancelTurn response."
          );
        }
        return response;
      } catch (error) {
        if (!(error instanceof NdjsonDisconnectError)) {
          throw error;
        }
        lastFailure = error;
        if (error.wrote || attempt === 1) {
          break;
        }
      } finally {
        connection?.close();
      }
    }
    throw this.mapDisconnect(lastFailure ?? new NdjsonDisconnectError("CancelTurn disconnected.", {
      connected: false,
      wrote: false
    }), "cancel_turn");
  }

  close(): void {
    this.dropActionConnection();
  }

  private async performAction(
    request: ServiceRequest,
    options: RequestOptions,
    state: RequestState
  ): Promise<ServiceResponse> {
    if (state.cancelled) {
      throw new SkyCuaError(
        "SKY_CUA_TURN_CANCELLED",
        "The sky-cua turn was cancelled before the action started.",
        {
          session_id: options.context?.session_id,
          turn_id: options.context?.turn_id
        }
      );
    }
    let lastFailure: NdjsonDisconnectError | undefined;
    for (let attempt = 0; attempt < 2; attempt += 1) {
      try {
        const health = await this.ensureHealth();
        if (options.requiredCapabilities !== undefined) {
          requireCapabilities(health, options.requiredCapabilities);
        }
        const connection = await this.getActionConnection();
        const response = await connection.request(request);
        if (isServiceError(response)) {
          if (
            response.retry === "safe_after_reconnect" &&
            attempt === 0 &&
            !state.cancelled
          ) {
            this.dropActionConnection();
            continue;
          }
          throw errorFromService(response);
        }
        this.validateActionResponse(request, response);
        return response;
      } catch (error) {
        if (!(error instanceof NdjsonDisconnectError)) {
          throw error;
        }
        lastFailure = error;
        this.dropActionConnection();
        if (state.cancelled || error.wrote || attempt === 1) {
          break;
        }
      }
    }
    throw this.mapDisconnect(
      lastFailure ?? new NdjsonDisconnectError("Sky-cua action disconnected.", {
        connected: false,
        wrote: false
      }),
      request.type,
      options.context
    );
  }

  private async ensureHealth(): Promise<ReturnType<typeof validateHealth>> {
    if (this.actionConnection !== undefined && this.health !== undefined) {
      return this.health;
    }
    let lastFailure: NdjsonDisconnectError | undefined;
    for (let attempt = 0; attempt < 2; attempt += 1) {
      try {
        const connection = await this.getActionConnection();
        const response = await connection.request({ type: "health" });
        this.health = validateHealth(response);
        return this.health;
      } catch (error) {
        if (!(error instanceof NdjsonDisconnectError)) {
          this.dropActionConnection();
          throw error;
        }
        lastFailure = error;
        this.dropActionConnection();
        if (!error.wrote && attempt === 0) {
          continue;
        }
        break;
      }
    }
    throw this.mapDisconnect(
      lastFailure ?? new NdjsonDisconnectError("Sky-cua health disconnected.", {
        connected: false,
        wrote: false
      }),
      "health"
    );
  }

  private async getActionConnection(): Promise<NdjsonConnection> {
    if (this.actionConnection !== undefined) {
      return this.actionConnection;
    }
    this.actionConnection = await NdjsonConnection.connect(this.endpoint);
    return this.actionConnection;
  }

  private dropActionConnection(): void {
    this.actionConnection?.close();
    this.actionConnection = undefined;
    this.health = undefined;
  }

  private mapDisconnect(
    failure: NdjsonDisconnectError,
    requestType: string,
    context?: RequestContext
  ): SkyCuaError {
    if (requestType === "health" || !failure.connected) {
      return new SkyCuaError(
        "SKY_CUA_SERVICE_RESTART_REQUIRED",
        "The sky-cua service socket is unavailable; the host-owned service must be restarted.",
        { retry: "caller_must_restart_service", cause: failure }
      );
    }
    const nonIdempotentMutation = new Set([
      "click",
      "drag",
      "press_key",
      "scroll",
      "type_text"
    ]);
    if (nonIdempotentMutation.has(requestType) && failure.wrote) {
      return new SkyCuaError(
        "SKY_CUA_ACTION_OUTCOME_UNKNOWN",
        "The action may have completed before the service disconnected.",
        {
          session_id: context?.session_id,
          turn_id: context?.turn_id,
          cause: failure
        }
      );
    }
    return new SkyCuaError(
      "SKY_CUA_SERVICE_DISCONNECTED",
      "The sky-cua service connection closed before a complete response.",
      {
        retry: "never",
        session_id: context?.session_id,
        turn_id: context?.turn_id,
        cause: failure
      }
    );
  }

  private async withDeadline(
    operation: () => Promise<ServiceResponse>,
    context: RequestContext,
    state: RequestState,
    signal?: AbortSignal
  ): Promise<ServiceResponse> {
    const deadlineMs = context.deadline_ms ?? 30_000;
    return await new Promise<ServiceResponse>((resolve, reject) => {
      let settled = false;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const cleanup = (): void => {
        if (timer !== undefined) {
          clearTimeout(timer);
        }
        signal?.removeEventListener("abort", onAbort);
      };
      const finish = (callback: () => void): void => {
        if (settled) {
          return;
        }
        settled = true;
        cleanup();
        callback();
      };
      const cancellationError = (reason: "deadline" | "cancelled"): SkyCuaError =>
        new SkyCuaError(
          reason === "deadline" ? "SKY_CUA_DEADLINE_EXCEEDED" : "SKY_CUA_TURN_CANCELLED",
          reason === "deadline"
            ? `The sky-cua action exceeded its ${deadlineMs}ms deadline.`
            : "The sky-cua turn was cancelled.",
          { session_id: context.session_id, turn_id: context.turn_id }
        );
      const cancelAndReject = (reason: "deadline" | "cancelled"): void => {
        if (settled) {
          return;
        }
        state.cancelled = true;
        this.dropActionConnection();
        void this.waitForCancel(context, reason).then(
          () => finish(() => reject(cancellationError(reason))),
          (error: unknown) => finish(() => reject(error))
        );
      };
      const onAbort = (): void => cancelAndReject("cancelled");
      if (signal?.aborted === true) {
        onAbort();
        return;
      }
      signal?.addEventListener("abort", onAbort, { once: true });
      timer = setTimeout(() => cancelAndReject("deadline"), deadlineMs);
      operation().then(
        (response) => {
          if (state.cancelled) {
            return;
          }
          finish(() => resolve(response));
        },
        (error: unknown) => {
          if (state.cancelled) {
            return;
          }
          finish(() => reject(error));
        }
      );
    });
  }

  private async waitForCancel(context: RequestContext, reason: "deadline" | "cancelled"): Promise<void> {
    const cancellation = this.cancelTurn(context, reason === "deadline" ? "deadline" : "user_cancelled");
    await Promise.race([
      cancellation.then(() => undefined),
      new Promise<void>((resolve) => setTimeout(resolve, DEFAULT_CANCEL_WAIT_MS))
    ]);
  }

  private validateActionResponse(request: ServiceRequest, response: ServiceResponse): void {
    if (!ACTION_TYPES.has(request.type as ActionType)) {
      return;
    }
    const actionRequest = request as Extract<ServiceRequest, { type: ActionType }>;
    if (
      response.type !== actionRequest.type ||
      response.ok !== true ||
      !("session_id" in response) ||
      response.session_id !== actionRequest.context.session_id ||
      !("turn_id" in response) ||
      response.turn_id !== actionRequest.context.turn_id
    ) {
      throw new SkyCuaError(
        "SKY_CUA_INVALID_REQUEST",
        "The sky-cua service returned an invalid or mismatched action response."
      );
    }
  }
}
