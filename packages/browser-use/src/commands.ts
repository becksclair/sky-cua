export const COMMANDS = [
  "browser_user_claim_tab",
  "browser_user_history",
  "browser_user_open_tabs",
  "close_tab",
  "create_tab",
  "cua_click",
  "cua_double_click",
  "cua_download_media",
  "cua_drag",
  "cua_keypress",
  "cua_move",
  "cua_scroll",
  "cua_type",
  "dom_cua_click",
  "dom_cua_double_click",
  "dom_cua_download_media",
  "dom_cua_get_visible_dom",
  "dom_cua_keypress",
  "dom_cua_scroll",
  "dom_cua_type",
  "finalize_tabs",
  "list_tabs",
  "name_session",
  "navigate_tab_back",
  "navigate_tab_forward",
  "navigate_tab_reload",
  "navigate_tab_url",
  "playwright_dom_snapshot",
  "playwright_download_path",
  "playwright_element_info",
  "playwright_element_screenshot",
  "playwright_evaluate",
  "playwright_file_chooser_set_files",
  "playwright_locator_all_text_contents",
  "playwright_locator_click",
  "playwright_locator_count",
  "playwright_locator_dblclick",
  "playwright_locator_download_media",
  "playwright_locator_fill",
  "playwright_locator_get_attribute",
  "playwright_locator_inner_text",
  "playwright_locator_is_enabled",
  "playwright_locator_is_visible",
  "playwright_locator_press",
  "playwright_locator_read_all",
  "playwright_locator_select_option",
  "playwright_locator_set_checked",
  "playwright_locator_text_content",
  "playwright_locator_wait_for",
  "playwright_wait_for_download",
  "playwright_wait_for_file_chooser",
  "playwright_wait_for_load_state",
  "playwright_wait_for_timeout",
  "playwright_wait_for_url",
  "selected_tab",
  "tab_bot_detection_report",
  "tab_browser_auth_handoff",
  "tab_cdp_call",
  "tab_cdp_events",
  "tab_clipboard_read",
  "tab_clipboard_read_text",
  "tab_clipboard_write",
  "tab_clipboard_write_text",
  "tab_content_export",
  "tab_content_export_gsuite",
  "tab_dev_logs",
  "tab_get_js_dialog",
  "tab_handle_js_dialog",
  "tab_id",
  "tab_page_assets_bundle",
  "tab_page_assets_list",
  "tab_screenshot",
] as const;

export type BrowserCommand = (typeof COMMANDS)[number];

export const RAW_PROTOCOL_UNSUPPORTED_COMMANDS = [] as const satisfies readonly BrowserCommand[];

const RAW_PROTOCOL_UNSUPPORTED_SET = new Set<BrowserCommand>(RAW_PROTOCOL_UNSUPPORTED_COMMANDS);

export const RAW_PROTOCOL_SUPPORTED_COMMANDS = Object.freeze(
  COMMANDS.filter((command) => !RAW_PROTOCOL_UNSUPPORTED_SET.has(command)),
);

export type CommandEnvelope = Record<string, unknown> & {
  type: BrowserCommand;
  browser_id: string;
};

export const COMMAND_GROUPS = Object.freeze({
  browser_user: COMMANDS.filter((value) => value.startsWith("browser_user_")),
  tabs: COMMANDS.filter((value) =>
    ["close_tab", "create_tab", "finalize_tabs", "list_tabs", "selected_tab"].includes(value),
  ),
  navigation: COMMANDS.filter((value) => value.startsWith("navigate_tab_")),
  cua: COMMANDS.filter((value) => value.startsWith("cua_")),
  dom_cua: COMMANDS.filter((value) => value.startsWith("dom_cua_")),
  playwright: COMMANDS.filter((value) => value.startsWith("playwright_")),
  tab: COMMANDS.filter((value) => value.startsWith("tab_")),
  browser: ["name_session"],
});
