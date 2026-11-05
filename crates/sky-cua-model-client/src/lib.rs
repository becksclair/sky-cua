//! Provider-neutral JSON model requests over a reusable HTTP connection pool.
//!
//! Callers own the endpoint, authentication headers, payload schema and response
//! interpretation. Errors never include credentials, request/response bodies or URLs.

use std::fmt;
use std::io::Read;
use std::time::{Duration, Instant};

use serde_json::Value;

#[derive(Clone)]
pub struct ModelClient {
    agent: ureq::Agent,
    max_response_bytes: usize,
}

#[derive(Debug)]
pub struct ModelResponse {
    pub body: Value,
    pub elapsed: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelError {
    InvalidConfiguration,
    InvalidRequest,
    Http(u16),
    Transport,
    InvalidResponse,
    ResponseTooLarge,
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(status) => write!(f, "model HTTP {status}"),
            other => write!(f, "model request failed: {other:?}"),
        }
    }
}

impl std::error::Error for ModelError {}

impl ModelClient {
    pub fn new(timeout: Duration, max_response_bytes: usize) -> Result<Self, ModelError> {
        if timeout.is_zero() || max_response_bytes == 0 {
            return Err(ModelError::InvalidConfiguration);
        }
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(0)
            .build();
        Ok(Self {
            agent: config.into(),
            max_response_bytes,
        })
    }

    pub fn post_json(
        &self,
        endpoint: &str,
        headers: &[(&str, &str)],
        payload: &Value,
    ) -> Result<ModelResponse, ModelError> {
        let body = serde_json::to_vec(payload).map_err(|_| ModelError::InvalidRequest)?;
        let mut request = self
            .agent
            .post(endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let started = Instant::now();
        let mut response = request.send(body.as_slice()).map_err(|error| match error {
            ureq::Error::StatusCode(status) => ModelError::Http(status),
            _ => ModelError::Transport,
        })?;
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(self.max_response_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| ModelError::Transport)?;
        if bytes.len() > self.max_response_bytes {
            return Err(ModelError::ResponseTooLarge);
        }
        let body = serde_json::from_slice(&bytes).map_err(|_| ModelError::InvalidResponse)?;
        Ok(ModelResponse {
            body,
            elapsed: started.elapsed(),
        })
    }
}
