import {
  HEALTH_CAPABILITIES,
  PROTOCOL_VERSION,
  SUPPORTED_PROTOCOL_VERSION_MAX,
  SUPPORTED_PROTOCOL_VERSION_MIN,
  type CuaJsCapability,
  type HealthResponse,
  type ServiceResponse
} from "../protocol/generated";
import { SkyCuaError, errorFromService } from "../errors";

export function isServiceError(response: ServiceResponse): response is Extract<ServiceResponse, { type: "error" }> {
  return response.type === "error" && response.ok === false;
}

export function validateHealth(response: ServiceResponse): HealthResponse {
  if (isServiceError(response)) {
    throw errorFromService(response);
  }
  if (
    response.type !== "health" ||
    response.ok !== true ||
    typeof response.protocol_version !== "number" ||
    typeof response.service_version !== "string" ||
    !Array.isArray(response.capabilities) ||
    typeof response.service_socket !== "string" ||
    response.service_socket.length === 0
  ) {
    throw new SkyCuaError(
      "SKY_CUA_INVALID_REQUEST",
      "The sky-cua service returned an invalid health response."
    );
  }
  if (
    response.protocol_version < SUPPORTED_PROTOCOL_VERSION_MIN ||
    response.protocol_version > SUPPORTED_PROTOCOL_VERSION_MAX ||
    response.protocol_version !== PROTOCOL_VERSION
  ) {
    throw new SkyCuaError(
      "SKY_CUA_PROTOCOL_UNSUPPORTED",
      `Unsupported sky-cua service protocol version ${response.protocol_version}.`
    );
  }
  return response;
}

export function requireCapabilities(
  health: HealthResponse,
  required: readonly CuaJsCapability[]
): void {
  const capabilities = new Set(health.capabilities);
  const missing = required.filter((capability) => !capabilities.has(capability));
  if (missing.length > 0) {
    throw new SkyCuaError(
      "SKY_CUA_CAPABILITY_MISSING",
      `The sky-cua service is missing capabilities: ${missing.join(", ")}.`
    );
  }
}

export function allHealthCapabilities(): readonly CuaJsCapability[] {
  return HEALTH_CAPABILITIES;
}
