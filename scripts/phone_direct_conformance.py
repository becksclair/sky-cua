"""Pure phone-control.v2 conformance helpers.

The production transports are implemented in Rust/Kotlin.  This module is a
small, deterministic oracle used by the cross-language fixture tests: it does
not open sockets, use clocks implicitly, or retain binary payloads outside a
transfer's temporary in-memory buffer.
"""

from __future__ import annotations

import hashlib
import hmac
import json
from dataclasses import dataclass
from typing import Any, Literal

PROTOCOL = "phone-control.v2"
DEFAULT_CHUNK_BYTES = 256 * 1024
MAX_JSON_BYTES = 1024 * 1024
CONTROL_ERROR = "invalid_control_frame"


def canonical_json(value: Any) -> bytes:
    """Encode JSON control values in the stable fixture form."""

    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def _length_prefixed(value: str) -> bytes:
    encoded = value.encode("utf-8")
    if len(encoded) > 0xFFFFFFFF:
        raise ValueError("auth field is too large")
    return len(encoded).to_bytes(4, "big") + encoded


def auth_transcript(
    *,
    protocol: str,
    device_id: str,
    server_nonce: str,
    client_nonce: str,
    link_epoch: int,
    role: Literal["saga", "companion"],
) -> bytes:
    """Encode six ordered UTF-8 fields, each with a four-byte BE length prefix."""

    if role not in {"companion", "saga"}:
        raise ValueError("invalid role")
    if link_epoch < 0 or link_epoch > 0xFFFFFFFFFFFFFFFF:
        raise ValueError("invalid link epoch")
    return b"".join(
        (
            _length_prefixed(protocol),
            _length_prefixed(device_id),
            _length_prefixed(server_nonce),
            _length_prefixed(client_nonce),
            _length_prefixed(str(link_epoch)),
            _length_prefixed(role),
        )
    )


def make_proof(secret: bytes, **fields: Any) -> str:
    return hmac.new(secret, auth_transcript(**fields), hashlib.sha256).hexdigest()


def verify_proof(secret: bytes, proof: str, **fields: Any) -> bool:
    expected = make_proof(secret, **fields)
    return hmac.compare_digest(expected, proof)


def enrollment_ack_transcript(
    *, protocol: str, enrollment_id: str, device_id: str, client_nonce: str
) -> bytes:
    """Encode the domain-separated acknowledgement proof transcript."""

    return b"".join(
        (
            _length_prefixed(protocol),
            _length_prefixed("enrollment_ack"),
            _length_prefixed(enrollment_id),
            _length_prefixed(device_id),
            _length_prefixed(client_nonce),
        )
    )


def make_enrollment_ack_proof(secret: bytes, **fields: str) -> str:
    return hmac.new(secret, enrollment_ack_transcript(**fields), hashlib.sha256).hexdigest()


def validate_control_frame(
    frame: dict[str, Any],
    *,
    device_id: str,
    link_epoch: int,
    seen_request_ids: set[str] | None = None,
) -> str | None:
    """Return a stable named error class, or ``None`` for an admissible frame."""

    frame_type = frame.get("type")
    if frame_type == "request":
        required = {"request_id", "device_id", "link_epoch", "idempotent"}
        if not required <= frame.keys():
            return "missing_field"
        if frame["device_id"] != device_id:
            return "device_mismatch"
        if frame["link_epoch"] != link_epoch:
            return "old_epoch"
        request_id = frame["request_id"]
        if not isinstance(request_id, str) or not request_id:
            return "invalid_request_id"
        if seen_request_ids is not None and request_id in seen_request_ids:
            return "replay_non_idempotent" if not frame["idempotent"] else "duplicate_idempotent"
        return None
    if frame_type == "auth_proof":
        return None if frame.get("role") in {"companion", "saga"} else "wrong_role"
    return CONTROL_ERROR


def validate_binary_header(
    header: dict[str, Any],
    payload: bytes,
    *,
    transfer_id: str,
    link_epoch: int,
    max_chunk_bytes: int,
) -> str | None:
    if header.get("transfer_id") != transfer_id:
        return "transfer_mismatch"
    if header.get("link_epoch") != link_epoch:
        return "old_epoch"
    if not isinstance(header.get("chunk_index"), int) or header["chunk_index"] < 0:
        return "invalid_chunk_index"
    if not isinstance(header.get("offset"), int) or header["offset"] < 0:
        return "invalid_offset"
    if header.get("length") != len(payload):
        return "payload_length_mismatch"
    if len(payload) > max_chunk_bytes:
        return "chunk_overflow"
    return None


@dataclass(frozen=True)
class Enrollment:
    enrollment_id: str
    bootstrap_credential: str
    expires_at_ms: int
    redeemed: bool = False


class EnrollmentRegistry:
    """Deterministic single-use/expiry model; caller supplies ``now_ms``."""

    def __init__(self) -> None:
        self._entries: dict[str, Enrollment] = {}

    def add(self, enrollment: Enrollment) -> None:
        self._entries[enrollment.enrollment_id] = enrollment

    def redeem(self, enrollment_id: str, credential: str, now_ms: int) -> bool:
        entry = self._entries.get(enrollment_id)
        if entry is None or entry.redeemed or now_ms >= entry.expires_at_ms:
            return False
        if not hmac.compare_digest(entry.bootstrap_credential, credential):
            return False
        self._entries[enrollment_id] = Enrollment(
            entry.enrollment_id,
            entry.bootstrap_credential,
            entry.expires_at_ms,
            redeemed=True,
        )
        return True


def epoch_matches(expected: int, actual: int) -> bool:
    """Old link epochs must be rejected before dispatching a side effect."""

    return expected == actual


@dataclass(frozen=True)
class ContentRefIdentity:
    content_id: str
    device_id: str
    link_epoch: int


@dataclass(frozen=True)
class TransferDeclaration:
    transfer_id: str
    device_id: str
    link_epoch: int
    content: ContentRefIdentity
    size_bytes: int
    sha256: str
    chunk_bytes: int
    chunk_count: int

    def __post_init__(self) -> None:
        if not self.device_id or self.content.device_id != self.device_id:
            raise ValueError("content device identity mismatch")
        if self.link_epoch != self.content.link_epoch:
            raise ValueError("content epoch identity mismatch")
        if self.size_bytes < 0 or self.chunk_bytes <= 0:
            raise ValueError("invalid transfer dimensions")
        expected = (self.size_bytes + self.chunk_bytes - 1) // self.chunk_bytes
        if self.chunk_count != expected:
            raise ValueError("chunk count formula mismatch")


class TransferVerifier:
    """Finite transfer verifier with atomic commit and interrupted cleanup."""

    def __init__(self, declaration: TransferDeclaration) -> None:
        self.declaration = declaration
        self._chunks: dict[int, bytes] = {}
        self.aborted = False
        self.committed = False

    def accept_chunk(
        self,
        *,
        chunk_index: int,
        offset: int,
        payload: bytes,
        device_id: str,
        link_epoch: int,
    ) -> None:
        d = self.declaration
        if self.aborted or self.committed:
            raise ValueError("transfer is closed")
        if device_id != d.device_id or link_epoch != d.link_epoch:
            raise ValueError("epoch mismatch")
        if chunk_index < 0 or chunk_index >= d.chunk_count:
            raise ValueError("chunk index out of range")
        if offset != chunk_index * d.chunk_bytes:
            raise ValueError("unexpected chunk offset")
        if not payload or len(payload) > d.chunk_bytes:
            raise ValueError("invalid chunk length")
        if offset + len(payload) > d.size_bytes:
            raise ValueError("chunk exceeds declared length")
        if chunk_index in self._chunks:
            raise ValueError("duplicate chunk")
        if chunk_index != len(self._chunks):
            raise ValueError("non-contiguous chunk arrival")
        self._chunks[chunk_index] = bytes(payload)

    def commit(self, *, size_bytes: int, sha256: str, link_epoch: int) -> bytes:
        d = self.declaration
        if self.aborted or self.committed:
            raise ValueError("transfer is closed")
        if link_epoch != d.link_epoch:
            raise ValueError("epoch mismatch")
        if len(self._chunks) != d.chunk_count:
            raise ValueError("incomplete transfer")
        payload = b"".join(self._chunks[i] for i in range(d.chunk_count))
        digest = hashlib.sha256(payload).hexdigest()
        if size_bytes != d.size_bytes or sha256 != d.sha256:
            raise ValueError("content digest mismatch")
        if len(payload) != size_bytes or digest != sha256:
            raise ValueError("content digest mismatch")
        self.committed = True
        self._chunks.clear()
        return payload

    def abort(self) -> None:
        self._chunks.clear()
        self.aborted = True

    @property
    def temporary_bytes(self) -> int:
        return sum(map(len, self._chunks.values()))


class PriorityScheduler:
    """Prioritize control while allowing bounded bulk progress."""

    def __init__(self, max_control_burst: int = 2) -> None:
        if max_control_burst <= 0:
            raise ValueError("max_control_burst must be positive")
        self.max_control_burst = max_control_burst
        self._control: list[str] = []
        self._bulk: list[str] = []

    def push_control(self, frame_id: str) -> None:
        self._control.append(frame_id)

    def push_bulk(self, chunk_id: str) -> None:
        self._bulk.append(chunk_id)

    def drain(self) -> list[str]:
        result: list[str] = []
        controls_since_bulk = 0
        while self._control or self._bulk:
            if self._control and (not self._bulk or controls_since_bulk < self.max_control_burst):
                result.append(self._control.pop(0))
                controls_since_bulk += 1
            else:
                result.append(self._bulk.pop(0))
                controls_since_bulk = 0
        return result
