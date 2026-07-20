import { createServer, type Socket, type Server } from "node:net";
import { Buffer } from "node:buffer";
import { unlinkSync } from "node:fs";

import {
  HEALTH_CAPABILITIES,
  type HealthResponse
} from "../../src/protocol/generated";
import type { TransportRequest, TransportResponse } from "../../src/window-action";

export type FakeDaemonRequest = {
  request: TransportRequest;
  socket: Socket;
};

export type FakeDaemonOptions = {
  onRequest?: (request: FakeDaemonRequest) => Promise<TransportResponse | undefined> | TransportResponse | undefined;
  fragmentResponses?: boolean;
  defaultHealth?: Partial<HealthResponse>;
};

export class FakeDaemon {
  readonly path: string;
  readonly requests: TransportRequest[] = [];
  readonly connectionIds: Socket[] = [];
  private readonly options: FakeDaemonOptions;
  private server: Server | undefined;

  constructor(path: string, options: FakeDaemonOptions = {}) {
    this.path = path;
    this.options = options;
  }

  async start(): Promise<void> {
    try {
      unlinkSync(this.path);
    } catch {
      // The test socket normally does not exist yet.
    }
    this.server = createServer((socket) => this.handleConnection(socket));
    await new Promise<void>((resolve, reject) => {
      this.server?.once("error", reject);
      this.server?.listen(this.path, resolve);
    });
  }

  async close(): Promise<void> {
    for (const socket of this.connectionIds) {
      socket.destroy();
    }
    await new Promise<void>((resolve) => {
      if (this.server === undefined) {
        resolve();
        return;
      }
      this.server.close(() => resolve());
    });
    this.server = undefined;
    try {
      unlinkSync(this.path);
    } catch {
      // The server may already have removed the socket.
    }
  }

  private handleConnection(socket: Socket): void {
    this.connectionIds.push(socket);
    let buffer = "";
    socket.on("data", (chunk: Uint8Array) => {
      buffer += Buffer.from(chunk).toString("utf8");
      while (true) {
        const newline = buffer.indexOf("\n");
        if (newline < 0) {
          return;
        }
        const line = buffer.slice(0, newline);
        buffer = buffer.slice(newline + 1);
        const request = JSON.parse(line) as TransportRequest;
        this.requests.push(request);
        void this.respond(socket, request);
      }
    });
  }

  private async respond(socket: Socket, request: TransportRequest): Promise<void> {
    const custom = this.options.onRequest;
    let response = custom === undefined ? undefined : await custom({ request, socket });
    if (response === undefined && request.type === "health") {
      response = {
        type: "health",
        ok: true,
        protocol_version: 1,
        service_version: "0.1.0",
        capabilities: [...HEALTH_CAPABILITIES],
        service_socket: this.path,
        ...this.options.defaultHealth
      };
    }
    if (response === undefined) {
      response = defaultResponse(request);
    }
    const encoded = `${JSON.stringify(response)}\n`;
    if (this.options.fragmentResponses === true) {
      for (const byte of Buffer.from(encoded)) {
        socket.write(Buffer.from([byte]));
      }
    } else {
      socket.write(encoded);
    }
  }
}

function defaultResponse(request: TransportRequest): TransportResponse {
  switch (request.type) {
    case "get_screenshot":
      return {
        type: "get_screenshot",
        ok: true,
        screenshots: []
      };
    case "cancel_turn":
      return {
        type: "cancel_turn",
        ok: true,
        session_id: request.session_id,
        turn_id: request.turn_id,
        status: "cancel_requested"
      };
    case "health":
      return {
        type: "health",
        ok: true,
        protocol_version: 1,
        service_version: "0.1.0",
        capabilities: [...HEALTH_CAPABILITIES],
        service_socket: "fake"
      };
    case "activate_window":
      return {
        type: "activate_window",
        outcome: {
          success: true,
          message: "window activated",
          code: "Activated",
          diagnostics: []
        }
      };
    default:
      return {
        type: request.type,
        ok: true,
        session_id: request.context.session_id,
        turn_id: request.context.turn_id
      };
  }
}
