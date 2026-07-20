import { invalidArgument, SkyCuaError } from "../errors";
import type { SkyConfig } from "../config";
import { PhoneScreenshot } from "./screenshot";
import { PhoneTransport } from "./transport";
import type {
  PhoneAccessibilityTreeRequest, PhoneAccessibilityTreeResponse, PhoneActionResponse,
  PhoneAppCurrentRequest, PhoneAppForceStopRequest, PhoneAppInstallRequest, PhoneAppLaunchRequest,
  PhoneAppListRequest, PhoneAppOpenIntentRequest, PhoneAppResponse, PhoneCapabilitiesResponse,
  PhoneCompanionStatusRequest, PhoneCompanionStatusResponse, PhoneConnectRequest,
  PhoneConnectedResponse, PhoneDisconnectRequest, PhoneDisconnectResponse,
  PhoneInstallCompanionRequest, PhoneListDevicesRequest, PhoneListDevicesResponse,
  PhoneNotificationActionRequest, PhoneNotificationDismissRequest, PhoneNotificationOpenRequest,
  PhoneNotificationReplyRequest, PhoneNotificationsRequest, PhoneNotificationsResponse,
  PhoneObserveRequest, PhoneObserveResponse, PhoneOpenSettingsRequest, PhonePairWirelessRequest,
  PhonePairWirelessResponse, PhonePressKeyRequest, PhoneRefreshCapabilitiesRequest, PhoneRequest,
  PhoneResponse, PhoneScreenshotRequest, PhoneSession, PhoneSessionSelector, PhoneStatusReport,
  PhoneStatusRequest, PhoneSwipeRequest, PhoneTapRequest, PhoneTypeTextRequest
} from "./protocol";

type Input<T extends { type: string }> = Omit<T, "type">;
type BoundInput<T extends { type: string } & PhoneSessionSelector> = Omit<T, "type" | "session_id" | "serial">;
export type PhoneClientOptions = { serviceSocketPath?: string };

const RESPONSE_TYPES = new Set([
  "observe", "status", "devices", "capabilities", "paired_wireless", "connected", "disconnected",
  "screenshot", "action", "companion_status", "accessibility_tree", "notifications", "app"
]);
const REQUEST_TYPES = new Set<PhoneRequest["type"]>([
  "observe", "status", "list_devices", "refresh_capabilities", "pair_wireless", "connect",
  "disconnect", "screenshot", "tap", "swipe", "type_text", "press_key", "install_companion",
  "companion_status", "accessibility_tree", "notifications", "notification_open",
  "notification_dismiss", "notification_action", "notification_reply", "app_current", "app_list",
  "app_launch", "app_open_intent", "app_force_stop", "app_install", "open_settings"
]);

function record(value: unknown, name: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw invalidArgument(`${name} input must be a plain object.`);
  }
  return value as Record<string, unknown>;
}

function exact(value: unknown, allowed: readonly string[], name: string): Record<string, unknown> {
  const result = record(value, name);
  const keys = new Set(allowed);
  for (const key of Object.keys(result)) {
    if (!keys.has(key)) throw invalidArgument(`${name} input contains unsupported field ${key}.`);
  }
  return result;
}

function string(value: unknown, name: string): string {
  if (typeof value !== "string" || value.length === 0) throw invalidArgument(`${name} must be a non-empty string.`);
  return value;
}

function finite(value: unknown, name: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) throw invalidArgument(`${name} must be a finite number.`);
  return value;
}

function coordinate(value: unknown, name: string): number {
  const result = finite(value, name);
  if (result < 0) throw invalidArgument(`${name} must be non-negative.`);
  return result;
}

function integer(value: unknown, name: string, min = 1): number {
  if (!Number.isInteger(value) || (value as number) < min) throw invalidArgument(`${name} must be an integer of at least ${min}.`);
  return value as number;
}

function optionalBoolean(value: unknown, name: string): boolean | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== "boolean") throw invalidArgument(`${name} must be a boolean.`);
  return value;
}

function optionalEnum<T extends string>(value: unknown, name: string, values: readonly T[]): T | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== "string" || !values.includes(value as T)) {
    throw invalidArgument(`${name} must be one of ${values.join(", ")}.`);
  }
  return value as T;
}

function optionalInput(value: unknown): Record<string, unknown> {
  return value === undefined ? {} : record(value, "Phone operation");
}

function selector(value: Record<string, unknown>): PhoneSessionSelector {
  const sessionId = value.session_id;
  const serial = value.serial;
  if (sessionId !== undefined && serial !== undefined) {
    throw invalidArgument("Phone operation accepts session_id or serial, not both.");
  }
  return {
    ...(sessionId === undefined ? {} : { session_id: string(sessionId, "session_id") }),
    ...(serial === undefined ? {} : { serial: string(serial, "serial") })
  };
}

function responseType<K extends PhoneResponse["type"]>(response: PhoneResponse, expected: readonly K[], request: string): Extract<PhoneResponse, { type: K }> {
  if (!RESPONSE_TYPES.has(response.type) || !expected.includes(response.type as K)) {
    throw invalidArgument(`Sky-cua service returned ${response.type} for Phone ${request}; expected ${expected.join(" or ")}.`);
  }
  return response as Extract<PhoneResponse, { type: K }>;
}

export class PhoneClient {
  private readonly transport: PhoneTransport;
  private closed = false;

  constructor(options: PhoneClientOptions = {}) {
    const config = options.serviceSocketPath === undefined
      ? undefined
      : ({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0, service_socket_path: options.serviceSocketPath } satisfies SkyConfig);
    this.transport = new PhoneTransport(config);
  }

  async request(request: PhoneRequest): Promise<PhoneResponse> {
    if (this.closed) throw new PhoneDisconnectedError();
    if (typeof request !== "object" || request === null || !REQUEST_TYPES.has(request.type)) {
      throw invalidArgument("Phone request must have a recognized type.");
    }
    return await this.transport.request(request);
  }

  async status(input: Input<PhoneStatusRequest> = {}): Promise<PhoneStatusReport> {
    const value = exact(input, ["refresh_devices"], "status");
    const refreshDevices = optionalBoolean(value.refresh_devices, "refresh_devices");
    const response = await this.request({ type: "status", ...(refreshDevices === undefined ? {} : { refresh_devices: refreshDevices }) });
    return responseType(response, ["status"], "status");
  }

  async list_devices(input: Input<PhoneListDevicesRequest> = {}): Promise<PhoneListDevicesResponse> {
    const value = exact(input, ["include_mdns"], "list_devices");
    const includeMdns = optionalBoolean(value.include_mdns, "include_mdns");
    const response = await this.request({ type: "list_devices", ...(includeMdns === undefined ? {} : { include_mdns: includeMdns }) });
    return responseType(response, ["devices"], "list_devices");
  }

  async pair_wireless(input: Input<PhonePairWirelessRequest>): Promise<PhonePairWirelessResponse> {
    const value = exact(input, ["host_port", "pairing_code"], "pair_wireless");
    const response = await this.request({ type: "pair_wireless", host_port: string(value.host_port, "host_port"), pairing_code: string(value.pairing_code, "pairing_code") });
    return responseType(response, ["paired_wireless"], "pair_wireless");
  }

  async connect(input: Input<PhoneConnectRequest> = {}): Promise<PhoneDeviceSession> {
    const value = exact(input, ["serial", "backend", "install_companion", "start_scrcpy"], "connect");
    const request = { type: "connect", ...value } as PhoneConnectRequest;
    if (request.serial !== undefined) string(request.serial, "serial");
    optionalEnum(request.backend, "backend", ["auto", "adb", "companion", "scrcpy", "none"]);
    optionalBoolean(request.install_companion, "install_companion");
    optionalBoolean(request.start_scrcpy, "start_scrcpy");
    const response: PhoneConnectedResponse = responseType(await this.request(request), ["connected"], "connect");
    const { type: _type, ...session } = response;
    return new PhoneDeviceSession(this, session);
  }

  bind(session: PhoneSession | PhoneSessionSelector): PhoneDeviceSession {
    if ("capability_profile" in session) return new PhoneDeviceSession(this, session);
    const selected = selector(record(session, "bind"));
    if (selected.session_id === undefined && selected.serial === undefined) throw invalidArgument("bind requires session_id or serial.");
    return new PhoneDeviceSession(this, selected);
  }

  async observe(input: Input<PhoneObserveRequest> = {}): Promise<PhoneObserveResponse> {
    const request = this.sessionRequest("observe", input, ["backend", "include_image_data", "include_accessibility", "include_notifications"]);
    optionalEnum(request.backend, "backend", ["auto", "adb", "companion", "scrcpy", "none"]);
    optionalBoolean(request.include_image_data, "include_image_data"); optionalBoolean(request.include_accessibility, "include_accessibility"); optionalBoolean(request.include_notifications, "include_notifications");
    return responseType(await this.request(request), ["observe"], "observe");
  }

  async refresh_capabilities(input: Input<PhoneRefreshCapabilitiesRequest> = {}): Promise<PhoneCapabilitiesResponse> {
    return responseType(await this.request(this.sessionRequest("refresh_capabilities", input)), ["capabilities"], "refresh_capabilities");
  }

  async disconnect(input: Input<PhoneDisconnectRequest> = {}): Promise<PhoneDisconnectResponse> {
    const request = this.sessionRequest("disconnect", input, ["keep_wireless"]); optionalBoolean(request.keep_wireless, "keep_wireless");
    return responseType(await this.request(request), ["disconnected"], "disconnect");
  }

  async screenshot(input: Input<PhoneScreenshotRequest> = {}): Promise<PhoneScreenshot> {
    const request = this.sessionRequest("screenshot", input, ["backend", "include_image_data"]);
    optionalEnum(request.backend, "backend", ["auto", "adb", "companion", "scrcpy", "none"]); optionalBoolean(request.include_image_data, "include_image_data");
    const response = responseType(await this.request(request), ["screenshot"], "screenshot");
    return new PhoneScreenshot(response);
  }

  async tap(input: Input<PhoneTapRequest>): Promise<PhoneActionResponse> {
    const request = this.sessionRequest("tap", input, ["phone_snapshot_id", "x", "y", "use_device_coordinates"]);
    coordinate(request.x, "x"); coordinate(request.y, "y");
    optionalBoolean(request.use_device_coordinates, "use_device_coordinates");
    return responseType(await this.request(request), ["action"], "tap");
  }

  async swipe(input: Input<PhoneSwipeRequest>): Promise<PhoneActionResponse> {
    const request = this.sessionRequest("swipe", input, ["phone_snapshot_id", "start_x", "start_y", "end_x", "end_y", "duration_ms", "use_device_coordinates"]);
    coordinate(request.start_x, "start_x"); coordinate(request.start_y, "start_y"); coordinate(request.end_x, "end_x"); coordinate(request.end_y, "end_y");
    if (request.duration_ms !== undefined) integer(request.duration_ms, "duration_ms", 0);
    optionalBoolean(request.use_device_coordinates, "use_device_coordinates");
    return responseType(await this.request(request), ["action"], "swipe");
  }

  async type_text(input: Input<PhoneTypeTextRequest>): Promise<PhoneActionResponse> {
    const request = this.sessionRequest("type_text", input, ["text"]); string(request.text, "text");
    return responseType(await this.request(request), ["action"], "type_text");
  }

  async press_key(input: Input<PhonePressKeyRequest>): Promise<PhoneActionResponse> {
    const request = this.sessionRequest("press_key", input, ["key"]); string(request.key, "key");
    return responseType(await this.request(request), ["action"], "press_key");
  }

  async install_companion(input: Input<PhoneInstallCompanionRequest> = {}): Promise<PhoneActionResponse> {
    const request = this.sessionRequest("install_companion", input, ["force_reinstall", "allow_downgrade"]); optionalBoolean(request.force_reinstall, "force_reinstall"); optionalBoolean(request.allow_downgrade, "allow_downgrade");
    return responseType(await this.request(request), ["action"], "install_companion");
  }

  async companion_status(input: Input<PhoneCompanionStatusRequest> = {}): Promise<PhoneCompanionStatusResponse> {
    return responseType(await this.request(this.sessionRequest("companion_status", input)), ["companion_status"], "companion_status");
  }

  async accessibility_tree(input: Input<PhoneAccessibilityTreeRequest> = {}): Promise<PhoneAccessibilityTreeResponse> {
    const request = this.sessionRequest("accessibility_tree", input, ["node_limit"]);
    if (request.node_limit !== undefined) integer(request.node_limit, "node_limit");
    return responseType(await this.request(request), ["accessibility_tree"], "accessibility_tree");
  }

  async notifications(input: Input<PhoneNotificationsRequest> = {}): Promise<PhoneNotificationsResponse> {
    const request = this.sessionRequest("notifications", input, ["limit"]);
    if (request.limit !== undefined) integer(request.limit, "limit");
    return responseType(await this.request(request), ["notifications"], "notifications");
  }

  async notification_open(input: Input<PhoneNotificationOpenRequest>): Promise<PhoneNotificationsResponse> {
    return await this.notification("notification_open", input, ["event_id"]);
  }
  async notification_dismiss(input: Input<PhoneNotificationDismissRequest>): Promise<PhoneNotificationsResponse> {
    return await this.notification("notification_dismiss", input, ["event_id"]);
  }
  async notification_action(input: Input<PhoneNotificationActionRequest>): Promise<PhoneNotificationsResponse> {
    return await this.notification("notification_action", input, ["event_id", "action_id"]);
  }
  async notification_reply(input: Input<PhoneNotificationReplyRequest>): Promise<PhoneNotificationsResponse> {
    return await this.notification("notification_reply", input, ["event_id", "action_id", "text"]);
  }

  async app_current(input: Input<PhoneAppCurrentRequest> = {}): Promise<PhoneAppResponse> { return await this.app("app_current", input); }
  async app_list(input: Input<PhoneAppListRequest> = {}): Promise<PhoneAppResponse> {
    const request = this.sessionRequest("app_list", input, ["include_system", "limit"]);
    optionalBoolean(request.include_system, "include_system");
    if (request.limit !== undefined) integer(request.limit, "limit");
    return responseType(await this.request(request), ["app"], "app_list");
  }
  async app_launch(input: Input<PhoneAppLaunchRequest>): Promise<PhoneAppResponse> { return await this.app("app_launch", input, ["package_name"]); }
  async app_open_intent(input: Input<PhoneAppOpenIntentRequest>): Promise<PhoneAppResponse> { return await this.app("app_open_intent", input, ["intent_uri", "package_name"]); }
  async app_force_stop(input: Input<PhoneAppForceStopRequest>): Promise<PhoneAppResponse> { return await this.app("app_force_stop", input, ["package_name"]); }
  async app_install(input: Input<PhoneAppInstallRequest>): Promise<PhoneAppResponse> {
    const request = this.sessionRequest("app_install", input, ["apk_paths", "mode", "reinstall", "allow_downgrade", "allow_test_apk", "grant_runtime_permissions"]);
    if (!Array.isArray(request.apk_paths) || request.apk_paths.length === 0) throw invalidArgument("apk_paths must be a non-empty array.");
    for (const path of request.apk_paths) string(path, "apk_paths entry");
    optionalEnum(request.mode, "mode", ["single", "multiple", "multi_package"]);
    optionalBoolean(request.reinstall, "reinstall"); optionalBoolean(request.allow_downgrade, "allow_downgrade"); optionalBoolean(request.allow_test_apk, "allow_test_apk"); optionalBoolean(request.grant_runtime_permissions, "grant_runtime_permissions");
    return responseType(await this.request(request), ["app"], "app_install");
  }
  async open_settings(input: Input<PhoneOpenSettingsRequest>): Promise<PhoneAppResponse> {
    const value = exact(input, ["session_id", "serial", "screen", "package_name"], "open_settings");
    const screen = optionalEnum(value.screen, "screen", ["accessibility", "notification_access", "overlay_permission", "app_details", "wireless_debugging", "battery_optimization"]);
    if (screen === undefined) throw invalidArgument("screen is required.");
    const request: PhoneOpenSettingsRequest = { type: "open_settings", ...selector(value), screen, ...(value.package_name === undefined ? {} : { package_name: string(value.package_name, "package_name") }) };
    return responseType(await this.request(request), ["app"], "open_settings");
  }

  get disconnected(): boolean { return this.closed; }
  close(): void { this.closed = true; this.transport.close(); }

  private sessionRequest<T extends PhoneRequest["type"]>(type: T, input: unknown, fields: readonly string[] = []): Extract<PhoneRequest, { type: T }> {
    const value = exact(optionalInput(input), ["session_id", "serial", ...fields], type);
    return { type, ...selector(value), ...Object.fromEntries(fields.filter((key) => value[key] !== undefined).map((key) => [key, value[key]])) } as Extract<PhoneRequest, { type: T }>;
  }

  private async notification<T extends "notification_open" | "notification_dismiss" | "notification_action" | "notification_reply">(type: T, input: unknown, fields: readonly string[]): Promise<PhoneNotificationsResponse> {
    const request = this.sessionRequest(type, input, fields);
    for (const field of fields) string((request as unknown as Record<string, unknown>)[field], field);
    return responseType(await this.request(request), ["notifications"], type);
  }

  private async app<T extends "app_current" | "app_launch" | "app_open_intent" | "app_force_stop" | "open_settings">(type: T, input: unknown, fields: readonly string[] = []): Promise<PhoneAppResponse> {
    const request = this.sessionRequest(type, input, fields);
    for (const field of fields) if ((request as unknown as Record<string, unknown>)[field] !== undefined) string((request as unknown as Record<string, unknown>)[field], field);
    return responseType(await this.request(request), ["app"], type);
  }
}

export class PhoneDeviceSession {
  readonly info?: Readonly<PhoneSession>;
  readonly selector: Readonly<PhoneSessionSelector>;
  private readonly client: PhoneClient;
  private active = true;

  constructor(client: PhoneClient, session: PhoneSession | PhoneSessionSelector) {
    this.client = client;
    this.info = "capability_profile" in session ? Object.freeze({ ...session }) : undefined;
    this.selector = Object.freeze(this.info === undefined ? { ...session } : { session_id: this.info.session_id });
  }

  get session_id(): string | undefined { return this.info?.session_id ?? this.selector.session_id; }
  get serial(): string | undefined { return this.info?.serial ?? this.selector.serial; }
  get disconnected(): boolean { return !this.active || this.client.disconnected; }
  observe(input: BoundInput<PhoneObserveRequest> = {}) { return this.call(() => this.client.observe({ ...this.selector, ...input })); }
  refresh_capabilities() { return this.call(() => this.client.refresh_capabilities(this.selector)); }
  disconnect(input: BoundInput<PhoneDisconnectRequest> = {}) { return this.call(() => this.client.disconnect({ ...this.selector, ...input })); }
  screenshot(input: BoundInput<PhoneScreenshotRequest> = {}) { return this.call(() => this.client.screenshot({ ...this.selector, ...input })); }
  tap(input: BoundInput<PhoneTapRequest>) { return this.call(() => this.client.tap({ ...this.selector, ...input })); }
  swipe(input: BoundInput<PhoneSwipeRequest>) { return this.call(() => this.client.swipe({ ...this.selector, ...input })); }
  type_text(input: BoundInput<PhoneTypeTextRequest>) { return this.call(() => this.client.type_text({ ...this.selector, ...input })); }
  press_key(input: BoundInput<PhonePressKeyRequest>) { return this.call(() => this.client.press_key({ ...this.selector, ...input })); }
  install_companion(input: BoundInput<PhoneInstallCompanionRequest> = {}) { return this.call(() => this.client.install_companion({ ...this.selector, ...input })); }
  companion_status() { return this.call(() => this.client.companion_status(this.selector)); }
  accessibility_tree(input: BoundInput<PhoneAccessibilityTreeRequest> = {}) { return this.call(() => this.client.accessibility_tree({ ...this.selector, ...input })); }
  notifications(input: BoundInput<PhoneNotificationsRequest> = {}) { return this.call(() => this.client.notifications({ ...this.selector, ...input })); }
  notification_open(input: BoundInput<PhoneNotificationOpenRequest>) { return this.call(() => this.client.notification_open({ ...this.selector, ...input })); }
  notification_dismiss(input: BoundInput<PhoneNotificationDismissRequest>) { return this.call(() => this.client.notification_dismiss({ ...this.selector, ...input })); }
  notification_action(input: BoundInput<PhoneNotificationActionRequest>) { return this.call(() => this.client.notification_action({ ...this.selector, ...input })); }
  notification_reply(input: BoundInput<PhoneNotificationReplyRequest>) { return this.call(() => this.client.notification_reply({ ...this.selector, ...input })); }
  app_current() { return this.call(() => this.client.app_current(this.selector)); }
  app_list(input: BoundInput<PhoneAppListRequest> = {}) { return this.call(() => this.client.app_list({ ...this.selector, ...input })); }
  app_launch(input: BoundInput<PhoneAppLaunchRequest>) { return this.call(() => this.client.app_launch({ ...this.selector, ...input })); }
  app_open_intent(input: BoundInput<PhoneAppOpenIntentRequest>) { return this.call(() => this.client.app_open_intent({ ...this.selector, ...input })); }
  app_force_stop(input: BoundInput<PhoneAppForceStopRequest>) { return this.call(() => this.client.app_force_stop({ ...this.selector, ...input })); }
  app_install(input: BoundInput<PhoneAppInstallRequest>) { return this.call(() => this.client.app_install({ ...this.selector, ...input })); }
  open_settings(input: BoundInput<PhoneOpenSettingsRequest>) { return this.call(() => this.client.open_settings({ ...this.selector, ...input })); }

  private assertActive(): void {
    if (this.disconnected) throw new PhoneDisconnectedError(this.session_id, this.serial);
  }

  private async call<T>(operation: () => Promise<T>): Promise<T> {
    this.assertActive();
    try {
      const result = await operation();
      if (truthfullyDisconnected(result)) this.active = false;
      return result;
    } catch (error) {
      if (serviceReportsDisconnected(error)) this.active = false;
      throw error;
    }
  }
}

export class PhoneDisconnectedError extends SkyCuaError {
  readonly phone_session_id?: string;
  readonly serial?: string;
  constructor(sessionId?: string, serial?: string) {
    super("SKY_CUA_SERVICE_DISCONNECTED", "The bound Phone session is disconnected and cannot be reused.", { retry: "never", session_id: sessionId });
    this.name = "PhoneDisconnectedError";
    this.phone_session_id = sessionId;
    this.serial = serial;
  }
}

function truthfullyDisconnected(value: unknown): boolean {
  const response = value instanceof PhoneScreenshot ? value.response : value;
  if (typeof response !== "object" || response === null) return false;
  const record = response as Record<string, unknown>;
  if (record.type === "disconnected" && record.disconnected === true) return true;
  return Array.isArray(record.diagnostics) && record.diagnostics.some((entry) =>
    typeof entry === "object" && entry !== null &&
    ((entry as { code?: unknown }).code === "PhoneNoSession" || (entry as { code?: unknown }).code === "PhoneDisconnected")
  );
}

function serviceReportsDisconnected(error: unknown): boolean {
  if (!(error instanceof SkyCuaError)) return false;
  const code = error.code as string;
  return code === "SKY_CUA_PHONE_DISCONNECTED" || code === "SKY_CUA_PHONE_SESSION_NOT_FOUND" ||
    /phone.*(?:disconnected|session.*not found|no session)/i.test(error.message);
}

export function createPhoneClient(options: PhoneClientOptions = {}): PhoneClient {
  return new PhoneClient(options);
}
