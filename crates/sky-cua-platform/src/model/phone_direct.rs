//! Serialized phone-control.v2 control-plane contracts.

use serde::{Deserialize, Serialize};

use super::{ContentTransferCommit, ContentTransferDeclaration};

pub const PHONE_CONTROL_PROTOCOL_V2: &str = "phone-control.v2";
pub const PHONE_ENROLLMENT_DEFAULT_TTL_MS: u64 = 5 * 60 * 1000;
/// Window after secret issuance for Android to prove its durable credential
/// commit. This is deliberately separate from the bootstrap expiry.
pub const PHONE_ENROLLMENT_PENDING_TTL_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneDirectRole {
    Saga,
    Companion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneEnrollmentPayload {
    pub protocol: String,
    pub endpoint: String,
    pub enrollment_id: String,
    pub bootstrap_credential: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneEnrollmentResult {
    pub protocol: String,
    pub enrollment_id: String,
    pub device_id: String,
    /// URL-safe, unpadded 256-bit secret returned exactly once over the
    /// one-time bootstrap connection.
    pub device_secret: String,
    pub enrolled_at_ms: u64,
    /// Deadline for proving that Android durably stored the credential.
    pub pending_expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneEnrollmentRedeem {
    pub protocol: String,
    pub enrollment_id: String,
    pub bootstrap_credential: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneEnrollmentAck {
    pub protocol: String,
    pub enrollment_id: String,
    pub device_id: String,
    /// URL-safe, unpadded 256-bit random nonce generated after Android's
    /// credential commit completes.
    pub client_nonce: String,
    /// Lowercase HMAC-SHA256 over the canonical, domain-separated enrollment
    /// acknowledgement transcript.
    pub client_proof: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneEnrollmentCommitted {
    pub protocol: String,
    pub enrollment_id: String,
    pub device_id: String,
    pub activated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAuthHello {
    pub protocol: String,
    pub device_id: String,
    pub client_nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAuthChallenge {
    pub protocol: String,
    pub server_nonce: String,
    pub link_epoch: String,
    pub server_proof: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAuthProof {
    pub link_epoch: String,
    pub client_proof: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAuthOk {
    pub protocol: String,
    pub device_id: String,
    pub link_epoch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhoneDirectControlFrame {
    EnrollmentRedeem(PhoneEnrollmentRedeem),
    EnrollmentOk(PhoneEnrollmentResult),
    EnrollmentAck(PhoneEnrollmentAck),
    EnrollmentCommitted(PhoneEnrollmentCommitted),
    AuthHello(PhoneAuthHello),
    AuthChallenge(PhoneAuthChallenge),
    AuthProof(PhoneAuthProof),
    AuthOk(PhoneAuthOk),
    Request {
        request_id: String,
        device_id: String,
        link_epoch: u64,
        idempotent: bool,
        expires_at_ms: u64,
        method: String,
        params: serde_json::Value,
    },
    Response {
        request_id: String,
        device_id: String,
        link_epoch: u64,
        result: serde_json::Value,
    },
    Error {
        request_id: Option<String>,
        device_id: Option<String>,
        link_epoch: Option<u64>,
        code: String,
        message: String,
    },
    Event {
        event_id: String,
        device_id: String,
        link_epoch: u64,
        event: String,
        payload: serde_json::Value,
    },
    ContentDeclare(ContentTransferDeclaration),
    ContentCommit(ContentTransferCommit),
    ContentAbort {
        transfer_id: String,
        link_epoch: u64,
        reason: String,
    },
    Ping {
        nonce: String,
    },
    Pong {
        nonce: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_request_carries_epoch_ttl_and_retry_class() {
        let frame = PhoneDirectControlFrame::Request {
            request_id: "r1".into(),
            device_id: "d1".into(),
            link_epoch: 3,
            idempotent: false,
            expires_at_ms: 55,
            method: "ui.tap".into(),
            params: serde_json::json!({"x": 1, "y": 2}),
        };
        let value = serde_json::to_value(&frame).expect("serializes");
        assert_eq!(value["type"], "request");
        assert_eq!(value["link_epoch"], 3);
        assert_eq!(value["idempotent"], false);
        assert_eq!(value["expires_at_ms"], 55);
    }

    #[test]
    fn auth_handshake_has_explicit_hello_and_string_epoch_boundary() {
        let hello = PhoneDirectControlFrame::AuthHello(PhoneAuthHello {
            protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
            device_id: "00000000-0000-4000-8000-000000000001".into(),
            client_nonce: "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8".into(),
        });
        let value = serde_json::to_value(hello).expect("hello serializes");
        assert_eq!(value["type"], "auth_hello");
        assert!(value.get("server_nonce").is_none());

        let challenge = PhoneDirectControlFrame::AuthChallenge(PhoneAuthChallenge {
            protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
            server_nonce: "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8".into(),
            link_epoch: "4".into(),
            server_proof: "00".repeat(32),
        });
        let value = serde_json::to_value(challenge).expect("challenge serializes");
        assert_eq!(value["link_epoch"], "4");
        assert!(value["link_epoch"].is_string());
    }

    #[test]
    fn enrollment_ack_and_commit_have_explicit_stable_wire_shapes() {
        let ack = PhoneDirectControlFrame::EnrollmentAck(PhoneEnrollmentAck {
            protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
            enrollment_id: "11111111-1111-4111-8111-111111111111".into(),
            device_id: "00000000-0000-4000-8000-000000000001".into(),
            client_nonce: "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8".into(),
            client_proof: "fc1c16c7f3a7b0b482615ff47e28a77dd172a6237eafffb1cba792603fa972b0".into(),
        });
        let ack = serde_json::to_value(ack).expect("ack serializes");
        assert_eq!(ack["type"], "enrollment_ack");
        assert_eq!(ack["protocol"], PHONE_CONTROL_PROTOCOL_V2);
        assert!(ack.get("device_secret").is_none());

        let committed = PhoneDirectControlFrame::EnrollmentCommitted(PhoneEnrollmentCommitted {
            protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
            enrollment_id: "11111111-1111-4111-8111-111111111111".into(),
            device_id: "00000000-0000-4000-8000-000000000001".into(),
            activated_at_ms: 42,
        });
        let committed = serde_json::to_value(committed).expect("commit serializes");
        assert_eq!(committed["type"], "enrollment_committed");
        assert_eq!(committed["activated_at_ms"], 42);
        assert!(committed.get("device_secret").is_none());
    }
}
