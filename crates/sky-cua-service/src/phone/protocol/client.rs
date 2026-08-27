use super::{
    AccessibilityTreeResult, AppOpResult, CurrentAppResult, GestureResult, NotificationsResult,
    ScreenshotResult,
};

#[derive(Debug, Clone)]
pub struct CompanionError(pub String);
impl CompanionError {
    pub fn is_fallback(&self) -> bool {
        true
    }
    pub fn code(&self) -> &str {
        &self.0
    }
    pub fn message(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Display for CompanionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for CompanionError {}

#[derive(Debug)]
pub struct CompanionClient;

impl CompanionClient {
    pub fn new(_port: u16, _token: impl Into<String>) -> Self {
        Self
    }
    pub fn new_with_addr(_addr: std::net::SocketAddr, _token: impl Into<String>) -> Self {
        Self
    }
    pub async fn health(&mut self) -> Result<super::HealthResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
    pub async fn capabilities(&mut self) -> Result<super::CapabilitiesResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
    pub async fn accessibility_tree(
        &mut self,
        _max: u32,
    ) -> Result<AccessibilityTreeResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
    pub async fn screenshot(&mut self, _include: bool) -> Result<ScreenshotResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
    pub async fn gesture(
        &mut self,
        _kind: super::GestureKind,
        _points: Vec<super::GesturePoint>,
        _duration_ms: u32,
    ) -> Result<GestureResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
    pub async fn notifications(
        &mut self,
        _max: u32,
    ) -> Result<NotificationsResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
    pub async fn current_app(&mut self) -> Result<CurrentAppResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
    pub async fn app_op(
        &mut self,
        _op: super::AppOp,
        _package: Option<String>,
        _intent_uri: Option<String>,
    ) -> Result<AppOpResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
    pub async fn notification_op(
        &mut self,
        _params: super::NotificationOpParams,
    ) -> Result<super::NotificationOpResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
    pub async fn overlay_active(
        &mut self,
        _active: bool,
    ) -> Result<super::OverlayActiveResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
    pub async fn overlay_gesture(
        &mut self,
        _kind: &str,
        _points: Vec<super::GesturePoint>,
        _duration_ms: u32,
    ) -> Result<super::OverlayGestureResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
    pub async fn cursor_overlay(
        &mut self,
        _visible: bool,
        _x: f64,
        _y: f64,
    ) -> Result<super::CursorOverlayResult, CompanionError> {
        Err(CompanionError("companion RPC removed".to_string()))
    }
}
