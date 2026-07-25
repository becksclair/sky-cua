import { describe, expect, test } from "bun:test";
import fixture from "../../../crates/sky-cua-platform/tests/fixtures/service-protocol-cua-js.json" with {
  type: "json"
};
import {
  CANCEL_TURN_ERROR_CODES,
  CANCEL_TURN_STATUSES,
  DEFAULT_DEADLINE_MS,
  DEFAULT_MOUSE_SIZE_PX,
  DEFAULT_POST_ACTION_SLEEP_MS,
  HEALTH_CAPABILITIES,
  MAX_FRAME_BYTES,
  MAX_JSON_BYTES,
  PROTOCOL_VERSION,
  REQUEST_IDEMPOTENCY,
  SERVICE_ERROR_CODES,
  SERVICE_PROTOCOL,
  SERVICE_VERSION,
  type CancelTurnRequest,
  type CancelTurnResponse,
  type GetScreenshotResponse,
  type ProtocolFixture,
  type ServiceError,
  type ServiceErrorCode,
  type ServiceRequest,
  type ServiceResponse
} from "../src/protocol/generated";

const typedFixture: ProtocolFixture = fixture as unknown as ProtocolFixture;

describe("cua-js service protocol fixture", () => {
  test("generated contract is an exact JSON fixture copy", () => {
    expect(SERVICE_PROTOCOL).toEqual(typedFixture);
    expect(JSON.stringify(SERVICE_PROTOCOL, null, 2)).toBe(
      JSON.stringify(typedFixture, null, 2)
    );
  });

  test("freezes version, health capabilities, and transport limits", () => {
    expect(PROTOCOL_VERSION).toBe(typedFixture.protocol.version);
    expect(SERVICE_VERSION).toBe(typedFixture.health.response.properties.service_version.const);
    expect([...HEALTH_CAPABILITIES]).toEqual(
      typedFixture.health.response.properties.capabilities.items.enum
    );
    expect(MAX_FRAME_BYTES).toBe(typedFixture.wire.max_frame_bytes);
    expect(MAX_JSON_BYTES).toBe(typedFixture.wire.max_json_bytes);
    expect(typedFixture.wire.frame_includes_newline).toBe(true);
    expect(typedFixture.wire.request_ids).toBe(false);
    expect(typedFixture.wire.max_in_flight_requests_per_connection).toBe(1);
  });

  test("freezes every Linux request payload and context requirement", () => {
    const linuxMethods = typedFixture.compatibility.linux_methods;
    expect(Object.keys(typedFixture.requests)).toEqual([
      "health",
      "click",
      "drag",
      "get_screenshot",
      "move",
      "press_key",
      "scroll",
      "type_text",
      "activate_window",
      "appshot_capture",
      "cancel_turn"
    ]);
    expect(linuxMethods).toEqual([
      "activate_window",
      "appshot_capture",
      "click",
      "drag",
      "get_screenshot",
      "move",
      "press_key",
      "scroll",
      "type_text"
    ]);
    expect(typedFixture.context.rules.required_for).toEqual([
      "click",
      "drag",
      "move",
      "press_key",
      "scroll",
      "type_text"
    ]);
    expect(typedFixture.context.rules.optional_for).toEqual(["get_screenshot", "activate_window"]);
    expect(typedFixture.context.rules.forbidden_for).toEqual(["health", "appshot_capture"]);
    expect(typedFixture.context.request_context.deadline.default_ms).toBe(30_000);
    expect(typedFixture.definitions.post_action_sleep_ms.default).toBe(100);
    expect(typedFixture.definitions.mouse_size_px.default).toBe(12);
  });

  test("freezes screenshot, cancellation, and all error spellings", () => {
    expect(typedFixture.screenshot).toEqual({
      wire_mime_type: "image/webp",
      wire_extension: ".webp",
      bytes_field: "bytes_base64",
      canonical_base64_only: true,
      decoded_bytes_type: "Uint8Array",
      data_url_prefix: "data:image/webp;base64,",
      implicit_emit_image: false,
      array_result: true,
      cursor_size_default_px: 12,
      cursor_size_zero_disables: true
    });
    expect(typedFixture.cancel_turn.response_statuses).toEqual([
      "cancel_requested",
      "already_cancelled",
      "not_found"
    ]);
    expect([...CANCEL_TURN_STATUSES]).toEqual(typedFixture.cancel_turn.response_statuses);
    expect(typedFixture.cancel_turn.error_codes).toEqual([
      "SKY_CUA_CANCEL_TURN_INVALID_CONTEXT",
      "SKY_CUA_CANCEL_TURN_INVALID_REASON",
      "SKY_CUA_SERVICE_RESTART_REQUIRED",
      "SKY_CUA_SERVICE_DISCONNECTED"
    ]);
    expect([...CANCEL_TURN_ERROR_CODES]).toEqual(typedFixture.cancel_turn.error_codes);
    expect(typedFixture.responses.error.properties.code.enum).toEqual([
      "SKY_CUA_SERVICE_RESTART_REQUIRED",
      "SKY_CUA_SERVICE_DISCONNECTED",
      "SKY_CUA_PROTOCOL_UNSUPPORTED",
      "SKY_CUA_CAPABILITY_MISSING",
      "SKY_CUA_INVALID_REQUEST",
      "SKY_CUA_INVALID_CONTEXT",
      "SKY_CUA_INVALID_ARGUMENT",
      "SKY_CUA_FRAME_TOO_LARGE",
      "SKY_CUA_ACTION_OUTCOME_UNKNOWN",
      "SKY_CUA_DEADLINE_EXCEEDED",
      "SKY_CUA_TURN_CANCELLED",
      "SKY_CUA_CANCEL_TURN_INVALID_CONTEXT",
      "SKY_CUA_CANCEL_TURN_INVALID_REASON",
      "SKY_CUA_TARGET_UNAVAILABLE",
      "SKY_CUA_INTERNAL"
    ]);
    expect([...SERVICE_ERROR_CODES]).toEqual(typedFixture.errors.codes);
    expect(typedFixture.errors.codes).toEqual(typedFixture.responses.error.properties.code.enum);
    expect(Object.keys(typedFixture.errors.semantics)).toEqual(typedFixture.errors.codes);
    expect(typedFixture.errors.disconnect_mapping.after_write_mutation).toBe(
      "SKY_CUA_ACTION_OUTCOME_UNKNOWN"
    );
    expect(typedFixture.errors.deadline_mapping.deadline_ms).toBe(DEFAULT_DEADLINE_MS);
  });

  test("idempotency classification is exhaustive and explicit", () => {
    expect(REQUEST_IDEMPOTENCY).toEqual(typedFixture.idempotency.classification);
    expect(typedFixture.idempotency.exhaustive).toBe(true);
    expect(typedFixture.idempotency.classification).toEqual({
      health: "idempotent_read",
      click: "non_idempotent_mutation",
      drag: "non_idempotent_mutation",
      get_screenshot: "idempotent_read",
      activate_window: "idempotent_set",
      appshot_capture: "idempotent_read",
      move: "idempotent_set",
      press_key: "non_idempotent_mutation",
      scroll: "non_idempotent_mutation",
      type_text: "non_idempotent_mutation",
      cancel_turn: "idempotent_control",
      error: "not_applicable"
    });
  });
});

// Compile-time contract probes. These values intentionally exercise every
// public union; the test body keeps them live so Bun also type-checks the
// generated declarations when it loads this file.
const context = { session_id: "session-1", turn_id: "turn-1" } as const;
const requests: ServiceRequest[] = [
  { type: "health" },
  { type: "click", context, x: 10, y: 20, mouse_button: "middle", click_count: 2, key: "Shift" },
  { type: "drag", context, from_x: 10, from_y: 20, to_x: 30, to_y: 40, key: "Alt" },
  { type: "get_screenshot", mouse_size_px: 0 },
  { type: "move", context, x: 30, y: 40 },
  { type: "press_key", context, key: "Ctrl+L" },
  { type: "scroll", context, direction: "r", pixels: 250, x: 50, y: 60, key: "Shift" },
  { type: "type_text", context, text: "hello" },
  { type: "activate_window", target: { window_id: "window-1" }, context },
  { type: "appshot_capture", request_id: "appshot-1", frontmost: true },
  { type: "cancel_turn", session_id: "session-1", turn_id: "turn-1", reason: "deadline" }
];

const screenshotResponse: GetScreenshotResponse = {
  type: "get_screenshot",
  ok: true,
  screenshots: [
    {
      filepath: "/tmp/capture.webp",
      bytes_base64: "UklGRg==",
      mime_type: "image/webp",
      width: 800,
      height: 600
    }
  ]
};

const cancelRequest: CancelTurnRequest = {
  type: "cancel_turn",
  session_id: "session-1",
  turn_id: "turn-1",
  reason: "user_cancelled"
};

const cancelResponse: CancelTurnResponse = {
  type: "cancel_turn",
  ok: true,
  session_id: "session-1",
  turn_id: "turn-1",
  status: "cancel_requested"
};

const serviceError: ServiceError = {
  type: "error",
  ok: false,
  code: "SKY_CUA_ACTION_OUTCOME_UNKNOWN",
  message: "The action may have completed before the service disconnected.",
  session_id: "session-1",
  turn_id: "turn-1",
  retry: "never"
};

const errorCodes: ServiceErrorCode[] = [
  "SKY_CUA_SERVICE_RESTART_REQUIRED",
  "SKY_CUA_SERVICE_DISCONNECTED",
  "SKY_CUA_PROTOCOL_UNSUPPORTED",
  "SKY_CUA_CAPABILITY_MISSING",
  "SKY_CUA_INVALID_REQUEST",
  "SKY_CUA_INVALID_CONTEXT",
  "SKY_CUA_INVALID_ARGUMENT",
  "SKY_CUA_FRAME_TOO_LARGE",
  "SKY_CUA_ACTION_OUTCOME_UNKNOWN",
  "SKY_CUA_DEADLINE_EXCEEDED",
  "SKY_CUA_TURN_CANCELLED",
  "SKY_CUA_CANCEL_TURN_INVALID_CONTEXT",
  "SKY_CUA_CANCEL_TURN_INVALID_REASON",
  "SKY_CUA_TARGET_UNAVAILABLE",
  "SKY_CUA_INTERNAL"
];

const responses: ServiceResponse[] = [screenshotResponse, cancelResponse, serviceError];
void [
  DEFAULT_DEADLINE_MS,
  DEFAULT_MOUSE_SIZE_PX,
  DEFAULT_POST_ACTION_SLEEP_MS,
  requests,
  cancelRequest,
  errorCodes,
  responses
];
