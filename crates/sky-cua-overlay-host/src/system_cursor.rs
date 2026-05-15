use anyhow::Result;
#[cfg(target_os = "linux")]
use x11rb::connection::Connection as X11Connection;

use sky_cua_platform::model::AgentCursorSystemCursorBackendKind;

#[derive(Debug)]
pub enum SystemCursorAdapter {
    Unsupported(UnsupportedSystemCursorAdapter),
    #[cfg(target_os = "linux")]
    X11(X11SystemCursorAdapter),
}

impl SystemCursorAdapter {
    #[must_use]
    pub fn wayland_client_unsupported(reason: impl Into<String>) -> Self {
        Self::unsupported_with_backend(
            AgentCursorSystemCursorBackendKind::WaylandClientUnsupported,
            reason,
        )
    }

    #[must_use]
    pub fn unsupported_with_backend(
        backend: AgentCursorSystemCursorBackendKind,
        reason: impl Into<String>,
    ) -> Self {
        Self::Unsupported(UnsupportedSystemCursorAdapter::new(backend, reason))
    }

    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn x11(
        conn: std::rc::Rc<x11rb::rust_connection::RustConnection>,
        root: x11rb::protocol::xproto::Window,
    ) -> Self {
        Self::X11(X11SystemCursorAdapter::new(conn, root))
    }

    #[must_use]
    pub fn backend(&self) -> AgentCursorSystemCursorBackendKind {
        match self {
            Self::Unsupported(adapter) => adapter.backend(),
            #[cfg(target_os = "linux")]
            Self::X11(adapter) => adapter.backend(),
        }
    }

    #[must_use]
    pub fn supported(&self) -> bool {
        match self {
            Self::Unsupported(adapter) => adapter.supported(),
            #[cfg(target_os = "linux")]
            Self::X11(adapter) => adapter.supported(),
        }
    }

    #[must_use]
    pub fn hidden(&self) -> bool {
        match self {
            Self::Unsupported(adapter) => adapter.hidden(),
            #[cfg(target_os = "linux")]
            Self::X11(adapter) => adapter.hidden(),
        }
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Unsupported(adapter) => adapter.reason(),
            #[cfg(target_os = "linux")]
            Self::X11(adapter) => adapter.reason(),
        }
    }

    pub fn set_hidden(&mut self, hidden: bool) -> Result<()> {
        match self {
            Self::Unsupported(adapter) => adapter.set_hidden(hidden),
            #[cfg(target_os = "linux")]
            Self::X11(adapter) => adapter.set_hidden(hidden),
        }
    }

    pub fn restore(&mut self) -> Result<()> {
        self.set_hidden(false)
    }
}

#[derive(Debug)]
pub struct UnsupportedSystemCursorAdapter {
    backend: AgentCursorSystemCursorBackendKind,
    reason: String,
}

impl UnsupportedSystemCursorAdapter {
    #[must_use]
    pub fn new(backend: AgentCursorSystemCursorBackendKind, reason: impl Into<String>) -> Self {
        Self {
            backend,
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn backend(&self) -> AgentCursorSystemCursorBackendKind {
        self.backend
    }

    #[must_use]
    pub fn supported(&self) -> bool {
        false
    }

    #[must_use]
    pub fn hidden(&self) -> bool {
        false
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        Some(self.reason.as_str())
    }

    pub fn set_hidden(&mut self, _hidden: bool) -> Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct X11SystemCursorAdapter {
    conn: std::rc::Rc<x11rb::rust_connection::RustConnection>,
    root: x11rb::protocol::xproto::Window,
    supported: bool,
    hidden: bool,
    reason: Option<String>,
}

#[cfg(target_os = "linux")]
impl X11SystemCursorAdapter {
    #[must_use]
    pub fn new(
        conn: std::rc::Rc<x11rb::rust_connection::RustConnection>,
        root: x11rb::protocol::xproto::Window,
    ) -> Self {
        let supported = x11rb::protocol::xfixes::query_version(conn.as_ref(), 4, 0)
            .is_ok_and(|cookie| cookie.reply().is_ok());
        Self {
            conn,
            root,
            supported,
            hidden: false,
            reason: (!supported).then(|| "XFixes HideCursor is unavailable".to_string()),
        }
    }

    #[must_use]
    pub fn backend(&self) -> AgentCursorSystemCursorBackendKind {
        AgentCursorSystemCursorBackendKind::X11Xfixes
    }

    #[must_use]
    pub fn supported(&self) -> bool {
        self.supported
    }

    #[must_use]
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    pub fn set_hidden(&mut self, hidden: bool) -> Result<()> {
        if !self.supported || self.hidden == hidden {
            return Ok(());
        }

        if hidden {
            x11rb::protocol::xfixes::hide_cursor(self.conn.as_ref(), self.root)?;
        } else {
            x11rb::protocol::xfixes::show_cursor(self.conn.as_ref(), self.root)?;
        }
        self.conn.flush()?;
        self.hidden = hidden;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SystemCursorAdapter;
    use sky_cua_platform::model::AgentCursorSystemCursorBackendKind;

    #[test]
    fn unsupported_adapter_reports_no_effective_hide() {
        let mut adapter = SystemCursorAdapter::unsupported_with_backend(
            AgentCursorSystemCursorBackendKind::Unsupported,
            "wayland clients cannot hide globally",
        );

        assert_eq!(
            adapter.backend(),
            AgentCursorSystemCursorBackendKind::Unsupported
        );
        assert!(!adapter.supported());
        adapter
            .set_hidden(true)
            .expect("unsupported hide is a no-op");
        assert!(!adapter.hidden());
        assert_eq!(
            adapter.reason(),
            Some("wayland clients cannot hide globally")
        );
    }

    #[test]
    fn wayland_adapter_reports_client_level_limitation() {
        let adapter =
            SystemCursorAdapter::wayland_client_unsupported("layer-shell cannot hide globally");

        assert_eq!(
            adapter.backend(),
            AgentCursorSystemCursorBackendKind::WaylandClientUnsupported
        );
        assert!(!adapter.supported());
        assert!(!adapter.hidden());
    }
}
