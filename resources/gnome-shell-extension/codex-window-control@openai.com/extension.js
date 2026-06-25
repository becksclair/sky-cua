import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const SERVICE_NAME = 'com.openai.Codex.WindowControl';
const OBJECT_PATH = '/com/openai/Codex/WindowControl';
const BACKEND = 'gnome-shell-extension';
const AGENT_CURSOR_RETIRED_MESSAGE =
    'GNOME Shell agent cursor rendering is retired; no WGPU GNOME overlay host is available';

const WINDOW_CONTROL_XML = `
<node>
  <interface name="${SERVICE_NAME}">
    <method name="ListWindows">
      <arg name="json" type="s" direction="out"/>
    </method>
    <method name="ActivateWindow">
      <arg name="window_id" type="t" direction="in"/>
      <arg name="ok" type="b" direction="out"/>
      <arg name="message" type="s" direction="out"/>
    </method>
    <method name="SetAgentCursorState">
      <arg name="json" type="s" direction="in"/>
      <arg name="ok" type="b" direction="out"/>
      <arg name="message" type="s" direction="out"/>
      <arg name="status_json" type="s" direction="out"/>
    </method>
    <method name="HideAgentCursor">
      <arg name="reason" type="s" direction="in"/>
      <arg name="ok" type="b" direction="out"/>
      <arg name="message" type="s" direction="out"/>
      <arg name="status_json" type="s" direction="out"/>
    </method>
    <method name="ShowAgentCursor">
      <arg name="ok" type="b" direction="out"/>
      <arg name="message" type="s" direction="out"/>
      <arg name="status_json" type="s" direction="out"/>
    </method>
    <method name="AgentCursorStatus">
      <arg name="status_json" type="s" direction="out"/>
    </method>
  </interface>
</node>
`;

const WindowControlDBus = GObject.registerClass(
class WindowControlDBus extends GObject.Object {
    constructor(extension) {
        super();

        this._extension = extension;
        this._agentCursorState = null;

        this._dbusObject = Gio.DBusExportedObject.wrapJSObject(
            WINDOW_CONTROL_XML, this);
        this._dbusObject.export(Gio.DBus.session, OBJECT_PATH);
        this._nameId = Gio.DBus.session.own_name(
            SERVICE_NAME,
            Gio.BusNameOwnerFlags.NONE,
            null,
            () => log(`Codex Window Control lost DBus name ${SERVICE_NAME}`));
    }

    destroy() {
        if (this._nameId) {
            Gio.DBus.session.unown_name(this._nameId);
            this._nameId = 0;
        }

        this._dbusObject?.unexport();
        this._dbusObject?.run_dispose();
        this._dbusObject = null;
    }

    ListWindowsAsync(_params, invocation) {
        this._returnJson(invocation, this._listWindows());
    }

    ActivateWindowAsync([windowId], invocation) {
        const requestedId = Number(windowId);
        const window = this._listMetaWindows().find(
            candidate => Number(candidate.get_id()) === requestedId);

        if (!window) {
            invocation.return_value(new GLib.Variant('(bs)', [
                false,
                `No window matched window_id ${requestedId}`,
            ]));
            return;
        }

        try {
            if (Main.overview.visible)
                Main.overview.hide();

            if (window.minimized && typeof window.unminimize === 'function')
                window.unminimize();

            Main.activateWindow(window, global.get_current_time());
            invocation.return_value(new GLib.Variant('(bs)', [
                true,
                `Activated window_id ${requestedId}`,
            ]));
        } catch (error) {
            invocation.return_value(new GLib.Variant('(bs)', [
                false,
                `Activation failed: ${error.message}`,
            ]));
        }
    }

    SetAgentCursorStateAsync([json], invocation) {
        let state;
        try {
            state = JSON.parse(json);
        } catch (error) {
            this._returnCursorResult(invocation, false, `Invalid cursor JSON: ${error.message}`);
            return;
        }

        const point = cursorPoint(state);
        if (state.visible && !point) {
            this._returnCursorResult(invocation, false,
                'Visible cursor state did not include native desktop coordinates');
            return;
        }

        this._agentCursorState = state;
        this._returnCursorResult(invocation, false, AGENT_CURSOR_RETIRED_MESSAGE);
    }

    HideAgentCursorAsync([reason], invocation) {
        this._agentCursorState = null;
        this._returnCursorResult(invocation, true,
            `${AGENT_CURSOR_RETIRED_MESSAGE}; hide acknowledged: ${reason || 'hide requested'}`);
    }

    ShowAgentCursorAsync(_params, invocation) {
        this._returnCursorResult(invocation, false, AGENT_CURSOR_RETIRED_MESSAGE);
    }

    AgentCursorStatusAsync(_params, invocation) {
        this._returnJson(invocation, this._cursorStatus());
    }

    _returnJson(invocation, value) {
        invocation.return_value(new GLib.Variant('(s)', [
            JSON.stringify(value),
        ]));
    }

    _returnCursorResult(invocation, ok, message) {
        invocation.return_value(new GLib.Variant('(bss)', [
            ok,
            message,
            JSON.stringify(this._cursorStatus()),
        ]));
    }

    _cursorStatus() {
        return {
            backend: BACKEND,
            visible: false,
            system_cursor_hide_supported: false,
            system_cursor_hidden: false,
            api: null,
            has_state: this._agentCursorState !== null,
            supported: false,
            reason: AGENT_CURSOR_RETIRED_MESSAGE,
        };
    }

    _listWindows() {
        return this._listMetaWindows()
            .map(window => this._windowInfo(window))
            .filter(window => window !== null);
    }

    _listMetaWindows() {
        return global.get_window_actors()
            .map(actor => actor.meta_window)
            .filter(window => window && !window.is_override_redirect?.())
            .filter(window => window.get_window_type?.() !== Meta.WindowType.DESKTOP);
    }

    _windowInfo(window) {
        if (!window)
            return null;

        const app = Shell.WindowTracker.get_default().get_window_app(window);
        const rect = window.get_frame_rect();
        const workspace = window.get_workspace?.();

        return {
            window_id: Number(window.get_id()),
            title: window.get_title?.() ?? null,
            app_id: app?.get_id?.() ?? null,
            wm_class: window.get_wm_class?.() ?? null,
            pid: window.get_pid?.() ?? null,
            bounds: rect ? {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            } : null,
            workspace: workspace?.index?.() ?? null,
            focused: global.display.focus_window === window && !Main.overview.visible,
            hidden: window.minimized ?? false,
            client_type: clientTypeName(window.get_client_type?.()),
            backend: BACKEND,
        };
    }
});

function cursorPoint(state) {
    if (!state)
        return null;
    const point = state.native_point ?? state.model_point;
    if (!point)
        return null;
    if (point.coordinate_space !== 'desktop_logical')
        return null;
    if (typeof point.x !== 'number' || typeof point.y !== 'number')
        return null;
    return point;
}

function clientTypeName(value) {
    if (value === undefined || value === null)
        return null;
    if (value === Meta.WindowClientType.WAYLAND)
        return 'wayland';
    if (value === Meta.WindowClientType.X11)
        return 'x11';
    return 'unknown';
}

export default class CodexWindowControlExtension extends Extension {
    enable() {
        this._dbusServer = new WindowControlDBus(this);
    }

    disable() {
        this._dbusServer?.destroy();
        this._dbusServer = null;
    }
}
