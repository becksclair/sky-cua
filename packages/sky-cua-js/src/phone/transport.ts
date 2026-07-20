import { phoneRequestContext, setPhoneUseResponseMeta, withSuspendedTimeout } from "../context";
import { SkyCuaError, errorFromService } from "../errors";
import { resolveServiceSocketPath, type SkyConfig } from "../config";
import { NdjsonConnection, NdjsonDisconnectError } from "../transport/ndjson-client";
import type { ServiceError } from "../protocol/generated";
import type { PhoneRequest, PhoneResponse, PhoneServiceRequest, PhoneServiceResponse } from "./protocol";

const NON_IDEMPOTENT = new Set<PhoneRequest["type"]>([
  "pair_wireless", "tap", "swipe", "type_text", "press_key", "install_companion",
  "notification_open", "notification_dismiss", "notification_action", "notification_reply",
  "app_launch", "app_open_intent", "app_force_stop", "app_install", "open_settings"
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isServiceError(value: unknown): value is ServiceError {
  return isRecord(value) && value.type === "error" && value.ok === false &&
    typeof value.code === "string" && typeof value.message === "string";
}

export class PhoneTransport {
  private readonly endpoint: string;
  private closed = false;

  constructor(config?: SkyConfig) {
    this.endpoint = resolveServiceSocketPath(config);
  }

  async request(request: PhoneRequest): Promise<PhoneResponse> {
    if (this.closed) {
      throw new SkyCuaError("SKY_CUA_SERVICE_DISCONNECTED", "The Phone client is closed.");
    }
    const context = phoneRequestContext();
    const envelope: PhoneServiceRequest = {
      type: "phone",
      request,
      ...(context === undefined ? {} : { context })
    };
    return await withSuspendedTimeout(async () => {
      let connection: NdjsonConnection | undefined;
      try {
        connection = await NdjsonConnection.connect(this.endpoint);
        const response = await connection.request<PhoneServiceResponse | ServiceError>(envelope);
        if (isServiceError(response)) {
          throw errorFromService(response);
        }
        if (!isRecord(response) || response.type !== "phone" || !isRecord(response.response) ||
          typeof response.response.type !== "string") {
          throw new SkyCuaError(
            "SKY_CUA_INVALID_REQUEST",
            "Sky-cua service returned an invalid Phone response envelope."
          );
        }
        setPhoneUseResponseMeta();
        return response.response as PhoneResponse;
      } catch (error) {
        if (!(error instanceof NdjsonDisconnectError)) {
          throw error;
        }
        if (!error.connected) {
          throw new SkyCuaError(
            "SKY_CUA_SERVICE_RESTART_REQUIRED",
            "The sky-cua service socket is unavailable; the host-owned service must be restarted.",
            { retry: "caller_must_restart_service", cause: error }
          );
        }
        if (error.wrote && NON_IDEMPOTENT.has(request.type)) {
          throw new SkyCuaError(
            "SKY_CUA_ACTION_OUTCOME_UNKNOWN",
            `The Phone ${request.type} operation may have completed before the service disconnected.`,
            { session_id: context?.session_id, turn_id: context?.turn_id, cause: error }
          );
        }
        throw new SkyCuaError(
          "SKY_CUA_SERVICE_DISCONNECTED",
          "The sky-cua service connection closed before a complete Phone response.",
          {
            retry: "never",
            session_id: context?.session_id,
            turn_id: context?.turn_id,
            cause: error
          }
        );
      } finally {
        connection?.close();
      }
    });
  }

  close(): void {
    this.closed = true;
  }
}
