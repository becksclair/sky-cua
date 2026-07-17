import { targetUnavailable } from "../errors";

export type MacPlaceholderClient = {
  click(...args: readonly unknown[]): Promise<never>;
  drag(...args: readonly unknown[]): Promise<never>;
  get_app_state(...args: readonly unknown[]): Promise<never>;
  list_apps(...args: readonly unknown[]): Promise<never>;
  perform_secondary_action(...args: readonly unknown[]): Promise<never>;
  press_key(...args: readonly unknown[]): Promise<never>;
  scroll(...args: readonly unknown[]): Promise<never>;
  select_text(...args: readonly unknown[]): Promise<never>;
  set_value(...args: readonly unknown[]): Promise<never>;
  type_text(...args: readonly unknown[]): Promise<never>;
};

const MAC_KEYS = [
  "click",
  "drag",
  "get_app_state",
  "list_apps",
  "perform_secondary_action",
  "press_key",
  "scroll",
  "select_text",
  "set_value",
  "type_text"
] as const;

export function createMacPlaceholder(): MacPlaceholderClient {
  const fail = async (): Promise<never> => {
    throw targetUnavailable("darwin");
  };
  return {
    click: fail,
    drag: fail,
    get_app_state: fail,
    list_apps: fail,
    perform_secondary_action: fail,
    press_key: fail,
    scroll: fail,
    select_text: fail,
    set_value: fail,
    type_text: fail
  };
}

export function macOwnKeys(): readonly string[] {
  return MAC_KEYS;
}
