import { Buffer } from "node:buffer";
import { createServer, type Server, type Socket } from "node:net";
import { unlinkSync, writeFileSync } from "node:fs";

import { afterEach, describe, expect, test } from "bun:test";

import { sky } from "../src/index";
import { createPhoneClient, phone, PhoneDisconnectedError, PhoneScreenshot, SkyCuaError, SkyPhoneFileError } from "../src/phone/index";
import type { PhoneRequest, PhoneServiceRequest } from "../src/phone/protocol";

const SOCKET = `/tmp/sky-cua-phone-js-${process.pid}.sock`;
const IMAGE_PATH = `/tmp/sky-cua-phone-js-${process.pid}.png`;

type Handler = (request: PhoneServiceRequest, socket: Socket) => unknown;

class PhoneDaemon {
  readonly requests: PhoneServiceRequest[] = [];
  connections = 0;
  private server?: Server;
  constructor(private readonly handler: Handler = defaultHandler) {}

  async start(): Promise<void> {
    try { unlinkSync(SOCKET); } catch {}
    this.server = createServer((socket) => {
      this.connections += 1;
      let buffer = "";
      socket.on("data", (chunk) => {
        buffer += Buffer.from(chunk).toString("utf8");
        const newline = buffer.indexOf("\n");
        if (newline < 0) return;
        const request = JSON.parse(buffer.slice(0, newline)) as PhoneServiceRequest;
        this.requests.push(request);
        const response = this.handler(request, socket);
        if (response !== undefined) socket.write(`${JSON.stringify(response)}\n`);
      });
    });
    await new Promise<void>((resolve) => this.server!.listen(SOCKET, resolve));
  }

  async close(): Promise<void> {
    await new Promise<void>((resolve) => this.server?.close(resolve));
    try { unlinkSync(SOCKET); } catch {}
  }
}

function phoneResponse(response: Record<string, unknown>) {
  return { type: "phone", response };
}

function defaultHandler(envelope: PhoneServiceRequest): unknown {
  const request = envelope.request;
  switch (request.type) {
    case "status": return phoneResponse({ type: "status", enabled: true, adb_available: true, scrcpy_available: false, companion_enabled: true, mdns_available: true, default_backend: "adb" });
    case "list_devices": return phoneResponse({ type: "devices", devices: [] });
    case "pair_wireless": return phoneResponse({ type: "paired_wireless", paired: true, host_port: request.host_port });
    case "connect": return phoneResponse({ type: "connected", session_id: "phone-1", serial: "serial-1", connection_kind: "usb", backend: "adb", capabilities: {}, capability_profile: { profile_id: "profile-1" }, managed_process: false, created_at_ms: 1 });
    case "disconnect": return phoneResponse({ type: "disconnected", session_id: request.session_id ?? "phone-1", serial: "serial-1", disconnected: true });
    case "observe": return phoneResponse({ type: "observe", session: { session_id: request.session_id ?? "phone-1", serial: "serial-1" }, backend: "adb", capability_profile_id: "profile-1", profile_refresh_state: "reused" });
    case "refresh_capabilities": return phoneResponse({ type: "capabilities", profile_id: "profile-1", session_id: request.session_id ?? "phone-1", serial: "serial-1" });
    case "screenshot": return phoneResponse({ type: "screenshot", session_id: request.session_id ?? "phone-1", serial: "serial-1", phone_snapshot_id: "snap-1", backend: "adb", capability_profile_id: "profile-1", profile_refresh_state: "reused", inline_image: { mime_type: "image/png", data_base64: Buffer.from("png-bytes").toString("base64") }, device_size: { width: 2, height: 2 }, coordinate_mapping: {}, cursor_capabilities: {}, capture_contains_native_overlay: false });
    case "companion_status": return phoneResponse({ type: "companion_status", session_id: request.session_id ?? "phone-1", serial: "serial-1", companion: {} });
    case "accessibility_tree": return phoneResponse({ type: "accessibility_tree", session_id: request.session_id ?? "phone-1", serial: "serial-1", backend: "companion", truncated: false, redacted: false });
    case "notifications":
    case "notification_open":
    case "notification_dismiss":
    case "notification_action":
    case "notification_reply": return phoneResponse({ type: "notifications", session_id: request.session_id ?? "phone-1", serial: "serial-1", backend: "companion", listener_enabled: true, truncated: false });
    case "app_current":
    case "app_list":
    case "app_launch":
    case "app_open_intent":
    case "app_force_stop":
    case "app_install":
    case "open_settings": return phoneResponse({ type: "app", session_id: request.session_id ?? "phone-1", serial: "serial-1", kind: request.type.slice(4), backend: "adb", success: true, truncated: false });
    default: return phoneResponse({ type: "action", session_id: "phone-1", serial: "serial-1", action: request.type, backend: "adb", capability_profile_id: "profile-1", profile_refresh_state: "reused" });
  }
}

function metadata(turn: string): void {
  globalThis.nodeRepl = {
    requestMeta: {
      session_id: "model-session",
      turn_id: turn,
      caller_provenance: "openclaw",
      identity_synthetic: true,
      client_info: { name: "OpenClaw", version: "2026.7.1", title: "OpenClaw host" }
    }
  };
}

afterEach(() => {
  globalThis.nodeRepl = undefined;
  try { unlinkSync(IMAGE_PATH); } catch {}
});

describe("@heliasar/sky-cua/phone", () => {
  test("keeps the root Computer facade unchanged and the Phone singleton lazy", () => {
    expect(Object.keys(sky)).toEqual(["activate_window", "appshot_capture", "click", "drag", "get_screenshot", "move", "press_key", "scroll", "type_text"]);
    expect(typeof phone.connect).toBe("function");
    expect(Object.keys(phone)).toEqual([
      "request", "status", "list_devices", "pair_wireless", "connect", "bind", "observe",
      "refresh_capabilities", "disconnect", "screenshot", "tap", "swipe", "type_text",
      "press_key", "install_companion", "companion_status", "accessibility_tree", "notifications",
      "notification_open", "notification_dismiss", "notification_action", "notification_reply",
      "app_current", "app_list", "app_launch", "app_open_intent", "app_force_stop", "app_install",
      "open_settings", "disconnected", "close"
    ]);
    expect("disconnected" in phone).toBe(true);
    expect(phone.disconnected).toBe(false);
    phone.close();
    expect(phone.disconnected).toBe(true);
  });

  test("covers every Phone request and response operation with a bound session", async () => {
    const daemon = new PhoneDaemon();
    await daemon.start();
    const client = createPhoneClient({ serviceSocketPath: SOCKET });
    let responseMeta: Record<string, unknown> | undefined;
    metadata("turn-0");
    globalThis.nodeRepl!.setResponseMeta = (value) => { responseMeta = value; };
    try {
      await client.status({ refresh_devices: true });
      await client.list_devices({ include_mdns: true });
      await client.pair_wireless({ host_port: "phone:37123", pairing_code: "123456" });
      const device = await client.connect({ serial: "serial-1", backend: "adb", install_companion: true, start_scrcpy: false });
      await device.observe({ include_image_data: true, include_accessibility: true, include_notifications: true });
      await device.refresh_capabilities();
      await device.screenshot({ backend: "adb", include_image_data: true });
      await device.tap({ phone_snapshot_id: "snap-1", x: 1, y: 2 });
      await device.swipe({ phone_snapshot_id: "snap-1", start_x: 1, start_y: 2, end_x: 3, end_y: 4, duration_ms: 100 });
      await device.type_text({ text: "hello" });
      await device.press_key({ key: "KEYCODE_BACK" });
      await device.install_companion({ force_reinstall: true, allow_downgrade: false });
      await device.companion_status();
      await device.accessibility_tree({ node_limit: 10 });
      await device.notifications({ limit: 10 });
      await device.notification_open({ event_id: "event-1" });
      await device.notification_dismiss({ event_id: "event-1" });
      await device.notification_action({ event_id: "event-1", action_id: "action-1" });
      await device.notification_reply({ event_id: "event-1", action_id: "reply-1", text: "yes" });
      await device.app_current();
      await device.app_list({ include_system: true, limit: 20 });
      await device.app_launch({ package_name: "org.example" });
      await device.app_open_intent({ intent_uri: "https://example.test", package_name: "org.example" });
      await device.app_force_stop({ package_name: "org.example" });
      await device.app_install({ apk_paths: ["/tmp/base.apk", "/tmp/split.apk"], mode: "multiple", reinstall: true, allow_downgrade: true, allow_test_apk: true, grant_runtime_permissions: true });
      await device.open_settings({ screen: "app_details", package_name: "org.example" });
      await device.disconnect({ keep_wireless: true });

      const types = daemon.requests.map((item) => item.request.type);
      expect(types).toEqual([
        "status", "list_devices", "pair_wireless", "connect", "observe", "refresh_capabilities",
        "screenshot", "tap", "swipe", "type_text", "press_key", "install_companion",
        "companion_status", "accessibility_tree", "notifications", "notification_open",
        "notification_dismiss", "notification_action", "notification_reply", "app_current", "app_list",
        "app_launch", "app_open_intent", "app_force_stop", "app_install", "open_settings", "disconnect"
      ] satisfies PhoneRequest["type"][]);
      expect(new Set(types).size).toBe(27);
      expect(daemon.requests.map((item) => item.request)).toEqual([
        { type: "status", refresh_devices: true },
        { type: "list_devices", include_mdns: true },
        { type: "pair_wireless", host_port: "phone:37123", pairing_code: "123456" },
        { type: "connect", serial: "serial-1", backend: "adb", install_companion: true, start_scrcpy: false },
        { type: "observe", session_id: "phone-1", include_image_data: true, include_accessibility: true, include_notifications: true },
        { type: "refresh_capabilities", session_id: "phone-1" },
        { type: "screenshot", session_id: "phone-1", backend: "adb", include_image_data: true },
        { type: "tap", session_id: "phone-1", phone_snapshot_id: "snap-1", x: 1, y: 2 },
        { type: "swipe", session_id: "phone-1", phone_snapshot_id: "snap-1", start_x: 1, start_y: 2, end_x: 3, end_y: 4, duration_ms: 100 },
        { type: "type_text", session_id: "phone-1", text: "hello" },
        { type: "press_key", session_id: "phone-1", key: "KEYCODE_BACK" },
        { type: "install_companion", session_id: "phone-1", force_reinstall: true, allow_downgrade: false },
        { type: "companion_status", session_id: "phone-1" },
        { type: "accessibility_tree", session_id: "phone-1", node_limit: 10 },
        { type: "notifications", session_id: "phone-1", limit: 10 },
        { type: "notification_open", session_id: "phone-1", event_id: "event-1" },
        { type: "notification_dismiss", session_id: "phone-1", event_id: "event-1" },
        { type: "notification_action", session_id: "phone-1", event_id: "event-1", action_id: "action-1" },
        { type: "notification_reply", session_id: "phone-1", event_id: "event-1", action_id: "reply-1", text: "yes" },
        { type: "app_current", session_id: "phone-1" },
        { type: "app_list", session_id: "phone-1", include_system: true, limit: 20 },
        { type: "app_launch", session_id: "phone-1", package_name: "org.example" },
        { type: "app_open_intent", session_id: "phone-1", intent_uri: "https://example.test", package_name: "org.example" },
        { type: "app_force_stop", session_id: "phone-1", package_name: "org.example" },
        { type: "app_install", session_id: "phone-1", apk_paths: ["/tmp/base.apk", "/tmp/split.apk"], mode: "multiple", reinstall: true, allow_downgrade: true, allow_test_apk: true, grant_runtime_permissions: true },
        { type: "open_settings", session_id: "phone-1", screen: "app_details", package_name: "org.example" },
        { type: "disconnect", session_id: "phone-1", keep_wireless: true }
      ] satisfies PhoneRequest[]);
      expect(daemon.requests.every((item) => item.type === "phone")).toBe(true);
      expect(daemon.requests.every((item) => JSON.stringify(item.context) === JSON.stringify({ session_id: "model-session", turn_id: "turn-0", caller_provenance: "openclaw", identity_synthetic: true, client_info: { name: "OpenClaw", version: "2026.7.1", title: "OpenClaw host" } }))).toBe(true);
      expect(responseMeta).toEqual({ "codex/toolSurface": { app: null, kind: "phoneUse" } });
      expect(daemon.connections).toBe(27);
    } finally {
      client.close();
      await daemon.close();
    }
  });

  test("snapshots current metadata independently for every request and omits unknown context", async () => {
    const daemon = new PhoneDaemon();
    await daemon.start();
    const client = createPhoneClient({ serviceSocketPath: SOCKET });
    try {
      metadata("turn-a");
      await client.status();
      globalThis.nodeRepl = { requestMeta: { session_id: "s", turn_id: "turn-b", caller_provenance: "bogus", client_info: null } };
      await client.status();
      globalThis.nodeRepl = undefined;
      await client.status();
      expect(daemon.requests[0]?.context?.turn_id).toBe("turn-a");
      expect(daemon.requests[1]?.context).toEqual({ session_id: "s", turn_id: "turn-b" });
      expect("context" in daemon.requests[2]!).toBe(false);
    } finally {
      client.close();
      await daemon.close();
    }
  });

  test("provides inline and local-file screenshot bytes, data URLs, and emitImage", async () => {
    const inline = new PhoneScreenshot({ type: "screenshot", session_id: "s", serial: "serial", phone_snapshot_id: "snap", backend: "adb", capability_profile_id: "p", profile_refresh_state: "reused", inline_image: { mime_type: "image/png", data_base64: Buffer.from("png-bytes").toString("base64") }, device_size: { width: 1, height: 1 }, coordinate_mapping: {} as never, cursor_capabilities: {} as never, capture_contains_native_overlay: false });
    expect(Buffer.from(await inline.bytes()).toString()).toBe("png-bytes");
    expect(await inline.dataUrl()).toBe(`data:image/png;base64,${Buffer.from("png-bytes").toString("base64")}`);

    writeFileSync(IMAGE_PATH, "file-png");
    const local = new PhoneScreenshot({ type: "screenshot", session_id: "s", serial: "serial", phone_snapshot_id: "snap", backend: "adb", capability_profile_id: "p", profile_refresh_state: "reused", screenshot_path: IMAGE_PATH, device_size: { width: 1, height: 1 }, coordinate_mapping: {} as never, cursor_capabilities: {} as never, capture_contains_native_overlay: false });
    expect(Buffer.from(await local.bytes()).toString()).toBe("file-png");
    let emitted = "";
    globalThis.nodeRepl = { emitImage(value) { emitted = value; return "emitted"; } };
    expect(await local.emitImage()).toBe("emitted");
    expect(emitted).toBe(`data:image/png;base64,${Buffer.from("file-png").toString("base64")}`);
    unlinkSync(IMAGE_PATH);
    try { await local.bytes(); throw new Error("expected read failure"); } catch (error) {
      expect(error instanceof SkyPhoneFileError).toBe(true);
      expect((error as SkyPhoneFileError).path).toBe(IMAGE_PATH);
    }
  });

  test("preserves structural service errors and rejects mismatched envelopes", async () => {
    const daemon = new PhoneDaemon(() => ({ type: "error", ok: false, code: "SKY_CUA_INVALID_ARGUMENT", message: "bad phone request", retry: "never", session_id: "s", turn_id: "t" }));
    await daemon.start();
    const client = createPhoneClient({ serviceSocketPath: SOCKET });
    try {
      let caught: unknown;
      try { await client.status(); } catch (error) { caught = error; }
      expect(caught instanceof SkyCuaError).toBe(true);
      expect((caught as SkyCuaError).code).toBe("SKY_CUA_INVALID_ARGUMENT");
      expect((caught as SkyCuaError).session_id).toBe("s");
    } finally { client.close(); await daemon.close(); }

    const malformed = new PhoneDaemon(() => ({ type: "phone", response: null }));
    await malformed.start();
    const malformedClient = createPhoneClient({ serviceSocketPath: SOCKET });
    try { await malformedClient.status(); throw new Error("expected malformed rejection"); } catch (error) {
      expect((error as SkyCuaError).code).toBe("SKY_CUA_INVALID_REQUEST");
    } finally { malformedClient.close(); await malformed.close(); }
  });

  test("never retries post-write disconnects and reports ambiguous mutations", async () => {
    const daemon = new PhoneDaemon((_request, socket) => { socket.destroy(); return undefined; });
    await daemon.start();
    const client = createPhoneClient({ serviceSocketPath: SOCKET });
    metadata("turn-disconnect");
    try {
      try { await client.tap({ session_id: "phone-1", x: 1, y: 1, use_device_coordinates: true }); throw new Error("expected disconnect"); } catch (error) {
        expect((error as SkyCuaError).code).toBe("SKY_CUA_ACTION_OUTCOME_UNKNOWN");
        expect((error as SkyCuaError).retry).toBe("never");
      }
      expect(daemon.requests.length).toBe(1);
      expect(daemon.connections).toBe(1);
    } finally { client.close(); await daemon.close(); }
  });

  test("rejects missing required app fields before daemon I/O", async () => {
    const daemon = new PhoneDaemon();
    await daemon.start();
    const client = createPhoneClient({ serviceSocketPath: SOCKET });
    try {
      for (const operation of [
        () => client.app_launch({ session_id: "phone-1" } as never),
        () => client.app_open_intent({ session_id: "phone-1" } as never),
        () => client.app_force_stop({ session_id: "phone-1" } as never),
      ]) {
        try { await operation(); throw new Error("expected required-field rejection"); } catch (error) {
          expect((error as SkyCuaError).code).toBe("SKY_CUA_INVALID_ARGUMENT");
        }
      }
      expect(daemon.requests.length).toBe(0);
      expect(daemon.connections).toBe(0);
    } finally { client.close(); await daemon.close(); }
  });

  test("matches direct Phone coordinate and zero-duration validation", async () => {
    const client = createPhoneClient({ serviceSocketPath: SOCKET });
    for (const operation of [
      () => client.tap({ session_id: "phone-1", x: -1, y: 0 }),
      () => client.swipe({ session_id: "phone-1", start_x: 0, start_y: -1, end_x: 1, end_y: 1 }),
    ]) {
      try { await operation(); throw new Error("expected coordinate rejection"); } catch (error) {
        expect((error as SkyCuaError).code).toBe("SKY_CUA_INVALID_ARGUMENT");
      }
    }
    const daemon = new PhoneDaemon();
    await daemon.start();
    try {
      await client.swipe({ session_id: "phone-1", start_x: 0, start_y: 0, end_x: 1, end_y: 1, duration_ms: 0 });
      expect((daemon.requests.at(-1)?.request as { duration_ms?: number }).duration_ms).toBe(0);
    } finally { client.close(); await daemon.close(); }
  });

  test("makes bound sessions terminal after disconnect and truthful no-session results", async () => {
    const daemon = new PhoneDaemon();
    await daemon.start();
    const client = createPhoneClient({ serviceSocketPath: SOCKET });
    try {
      const device = await client.connect({ serial: "serial-1" });
      const result = await device.disconnect();
      expect(result.disconnected).toBe(true);
      expect(device.disconnected).toBe(true);
      const requestCount = daemon.requests.length;
      try { await device.app_current(); throw new Error("expected terminal session"); } catch (error) {
        expect(error instanceof PhoneDisconnectedError).toBe(true);
        expect((error as PhoneDisconnectedError).code).toBe("SKY_CUA_SERVICE_DISCONNECTED");
        expect((error as PhoneDisconnectedError).retry).toBe("never");
      }
      expect(daemon.requests.length).toBe(requestCount);
    } finally { client.close(); await daemon.close(); }

    const noSession = new PhoneDaemon((request) => request.request.type === "observe"
      ? phoneResponse({ type: "observe", session: { session_id: "gone", serial: "serial-1" }, backend: "none", capability_profile_id: "missing", profile_refresh_state: "stale", diagnostics: [{ code: "PhoneNoSession", message: "No active phone session" }] })
      : defaultHandler(request));
    await noSession.start();
    const noSessionClient = createPhoneClient({ serviceSocketPath: SOCKET });
    try {
      const device = noSessionClient.bind({ session_id: "gone" });
      await device.observe();
      expect(device.disconnected).toBe(true);
      const requestCount = noSession.requests.length;
      try { await device.screenshot(); throw new Error("expected terminal session"); } catch (error) {
        expect(error instanceof PhoneDisconnectedError).toBe(true);
      }
      expect(noSession.requests.length).toBe(requestCount);
    } finally { noSessionClient.close(); await noSession.close(); }

    const closedClient = createPhoneClient({ serviceSocketPath: SOCKET });
    const closedDevice = closedClient.bind({ session_id: "closed" });
    closedClient.close();
    expect(closedDevice.disconnected).toBe(true);
    try { await closedDevice.app_current(); throw new Error("expected closed client session"); } catch (error) {
      expect(error instanceof PhoneDisconnectedError).toBe(true);
    }

    const missingError = new PhoneDaemon(() => ({ type: "error", ok: false, code: "SKY_CUA_PHONE_SESSION_NOT_FOUND", message: "Phone session not found", retry: "never" }));
    await missingError.start();
    const missingClient = createPhoneClient({ serviceSocketPath: SOCKET });
    try {
      const device = missingClient.bind({ session_id: "gone" });
      try { await device.companion_status(); } catch {}
      expect(device.disconnected).toBe(true);
      const requestCount = missingError.requests.length;
      try { await device.notifications(); throw new Error("expected terminal session"); } catch (error) {
        expect(error instanceof PhoneDisconnectedError).toBe(true);
      }
      expect(missingError.requests.length).toBe(requestCount);
    } finally { missingClient.close(); await missingError.close(); }
  });
});
