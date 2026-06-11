use serde_json::{Value, json};

/// MCP ToolAnnotations advertised with every tool definition.
///
/// Hosts gate per-tool approval on these hints: Codex's "auto" approval mode
/// treats unannotated tools as destructive and open-world, so without
/// annotations every sky-cua call raises a user approval prompt. The hints
/// must stay honest — a host that auto-approves read-only tools relies on
/// `read_only: true` meaning no environment mutation at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolAnnotations {
    pub(crate) read_only: bool,
    pub(crate) destructive: bool,
    pub(crate) idempotent: bool,
    pub(crate) open_world: bool,
}

impl ToolAnnotations {
    pub(crate) fn to_value(self) -> Value {
        json!({
            "readOnlyHint": self.read_only,
            "destructiveHint": self.destructive,
            "idempotentHint": self.idempotent,
            "openWorldHint": self.open_world,
        })
    }
}

/// Observation-only tools: no desktop, browser, or system mutation.
pub(crate) const READ_ONLY_TOOL: ToolAnnotations = ToolAnnotations {
    read_only: true,
    destructive: false,
    idempotent: true,
    open_world: false,
};

/// Local state changes that are safe to repeat and cannot trigger arbitrary
/// app behavior: focus moves, selection, expand/collapse, window activation,
/// setup toggles, tab claims, cursor moves.
pub(crate) const LOCAL_NAVIGATION_ACTION: ToolAnnotations = ToolAnnotations {
    read_only: false,
    destructive: false,
    idempotent: true,
    open_world: false,
};

/// Local state changes where repeating is not idempotent but the action
/// itself cannot trigger arbitrary app behavior: scrolling, toggling.
pub(crate) const LOCAL_STATEFUL_ACTION: ToolAnnotations = ToolAnnotations {
    read_only: false,
    destructive: false,
    idempotent: false,
    open_world: false,
};

/// Arbitrary desktop input: clicks, key presses, typed text, drags, AT-SPI
/// default/custom actions. These can press any button in any app, so the
/// destructive hint must stay true.
pub(crate) const LOCAL_DESTRUCTIVE_ACTION: ToolAnnotations = ToolAnnotations {
    read_only: false,
    destructive: true,
    idempotent: false,
    open_world: false,
};

/// Arbitrary input into live web pages: clicks, typing, key presses, eval.
pub(crate) const OPEN_WORLD_DESTRUCTIVE_ACTION: ToolAnnotations = ToolAnnotations {
    read_only: false,
    destructive: true,
    idempotent: false,
    open_world: true,
};
