import type { ServiceRequest, ServiceResponse } from "./protocol/generated";

export type {
  ActivateWindowRequest,
  ActivateWindowResponse,
  WindowActionDiagnostic,
  WindowActionOutcome,
  WindowTarget
} from "./protocol/generated";

export type TransportRequest = ServiceRequest;
export type TransportResponse = ServiceResponse;
