from __future__ import annotations

import hashlib
import json
from pathlib import Path

import pytest

from phone_direct_conformance import (
    ContentRefIdentity,
    Enrollment,
    EnrollmentRegistry,
    PriorityScheduler,
    TransferDeclaration,
    TransferVerifier,
    auth_transcript,
    enrollment_ack_transcript,
    epoch_matches,
    make_enrollment_ack_proof,
    make_proof,
    validate_binary_header,
    validate_control_frame,
    verify_proof,
)

FIXTURES = Path(__file__).parents[2] / "docs/runtime/fixtures/phone-control-v2"


def test_protocol_fixture_is_canonical_and_has_frozen_limits() -> None:
    payload = json.loads((FIXTURES / "protocol.json").read_text())
    assert payload["protocol"] == "phone-control.v2"
    assert payload["limits"]["chunk_bytes"] == 262144
    assert payload["limits"]["max_json_bytes"] == 1048576
    assert payload["content_sources"] == sorted(payload["content_sources"])


def test_control_and_binary_fixture_vectors_are_json_or_hex_only() -> None:
    controls = json.loads((FIXTURES / "control_frames.json").read_text())
    assert json.loads(controls["valid"][0])["type"] == "request"
    assert all(isinstance(item["json"], str) for item in controls["invalid"])
    binary = json.loads((FIXTURES / "binary_frames.json").read_text())
    for frame in binary["valid"] + binary["invalid"]:
        bytes.fromhex(frame["payload_hex"])


def test_every_invalid_fixture_has_named_validator_error() -> None:
    controls = json.loads((FIXTURES / "control_frames.json").read_text())
    assert (
        validate_control_frame(
            json.loads(controls["valid"][0]),
            device_id="device-fixture",
            link_epoch=4,
            seen_request_ids=set(),
        )
        is None
    )
    for item in controls["invalid"]:
        frame = json.loads(item["json"])
        seen = {"request-1"} if item["name"].startswith("replay") else set()
        assert (
            validate_control_frame(
                frame, device_id="device-fixture", link_epoch=4, seen_request_ids=seen
            )
            == item["expected_error"]
        )
    binary = json.loads((FIXTURES / "binary_frames.json").read_text())
    valid = binary["valid"][0]
    valid_header = {
        key: valid[key] for key in ("transfer_id", "chunk_index", "offset", "length", "link_epoch")
    }
    assert (
        validate_binary_header(
            valid_header,
            bytes.fromhex(valid["payload_hex"]),
            transfer_id="transfer-1",
            link_epoch=9,
            max_chunk_bytes=8,
        )
        is None
    )
    for item in binary["invalid"]:
        header = {
            key: item[key]
            for key in ("transfer_id", "chunk_index", "offset", "length", "link_epoch")
        }
        assert (
            validate_binary_header(
                header,
                bytes.fromhex(item["payload_hex"]),
                transfer_id="transfer-1",
                link_epoch=9,
                max_chunk_bytes=8,
            )
            == item["expected_error"]
        )


def test_hmac_transcript_is_stable_and_replay_fields_are_bound() -> None:
    fields = {
        "protocol": "phone-control.v2",
        "device_id": "00000000-0000-4000-8000-000000000001",
        "server_nonce": "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8",
        "client_nonce": "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8",
        "link_epoch": 4,
        "role": "companion",
    }
    auth_fixture = json.loads((FIXTURES / "auth_transcript.json").read_text())
    secret = bytes.fromhex(auth_fixture["secret_hex"])
    assert (
        auth_transcript(**fields).hex()
        == json.loads((FIXTURES / "auth_transcript.json").read_text())["transcript_hex"]
    )
    proof = make_proof(secret, **fields)
    assert verify_proof(secret, proof, **fields)
    assert proof == auth_fixture["proofs"]["companion"]
    assert make_proof(secret, **{**fields, "role": "saga"}) == auth_fixture["proofs"]["saga"]
    assert not verify_proof(secret, proof, **{**fields, "link_epoch": 5})
    assert not verify_proof(secret, proof, **{**fields, "client_nonce": "replay"})
    assert not verify_proof(secret, proof, **{**fields, "role": "saga"})
    with pytest.raises(ValueError, match="role"):
        auth_transcript(**{**fields, "role": "unknown"})


def test_enrollment_is_single_use_and_expires_at_boundary() -> None:
    registry = EnrollmentRegistry()
    registry.add(Enrollment("enroll-1", "bootstrap-fixture", 5_000))
    assert registry.redeem("enroll-1", "bootstrap-fixture", 4_999)
    assert not registry.redeem("enroll-1", "bootstrap-fixture", 4_999)
    registry.add(Enrollment("enroll-2", "bootstrap-fixture", 5_000))
    assert not registry.redeem("enroll-2", "bootstrap-fixture", 5_000)
    assert not registry.redeem("missing", "bootstrap-fixture", 1)
    registry.add(Enrollment("enroll-3", "bootstrap-fixture", 5_000))
    assert not registry.redeem("enroll-3", "wrong", 1)
    assert registry.redeem("enroll-3", "bootstrap-fixture", 1)


def test_enrollment_ack_transcript_matches_cross_language_fixture() -> None:
    fixture = json.loads((FIXTURES / "enrollment_ack_transcript.json").read_text())
    fields = {
        "protocol": fixture["protocol"],
        "enrollment_id": fixture["enrollment_id"],
        "device_id": fixture["device_id"],
        "client_nonce": fixture["client_nonce"],
    }
    secret = bytes.fromhex(fixture["secret_hex"])
    assert enrollment_ack_transcript(**fields).hex() == fixture["transcript_hex"]
    assert make_enrollment_ack_proof(secret, **fields) == fixture["proof"]
    assert make_enrollment_ack_proof(secret, **{**fields, "device_id": "other"}) != fixture["proof"]


def test_epoch_fencing_is_explicit() -> None:
    assert epoch_matches(7, 7)
    assert not epoch_matches(6, 7)


def test_transfer_requires_complete_verified_atomic_commit() -> None:
    data = b"phone-control-fixture-payload"
    declaration = TransferDeclaration(
        "transfer-1",
        "device-fixture",
        9,
        ContentRefIdentity("content-1", "device-fixture", 9),
        len(data),
        hashlib.sha256(data).hexdigest(),
        8,
        4,
    )
    transfer = TransferVerifier(declaration)
    for index in range(4):
        start = index * 8
        transfer.accept_chunk(
            chunk_index=index,
            offset=start,
            payload=data[start : start + 8],
            device_id="device-fixture",
            link_epoch=9,
        )
    assert transfer.commit(size_bytes=len(data), sha256=declaration.sha256, link_epoch=9) == data
    assert transfer.temporary_bytes == 0


def test_transfer_rejects_epoch_duplicates_and_digest_mismatch() -> None:
    data = b"0123456789"
    declaration = TransferDeclaration(
        "transfer-2",
        "device-fixture",
        2,
        ContentRefIdentity("content-2", "device-fixture", 2),
        len(data),
        hashlib.sha256(data).hexdigest(),
        8,
        2,
    )
    transfer = TransferVerifier(declaration)
    with pytest.raises(ValueError, match="epoch"):
        transfer.accept_chunk(
            chunk_index=0, offset=0, payload=data[:8], device_id="device-fixture", link_epoch=3
        )
    transfer.accept_chunk(
        chunk_index=0, offset=0, payload=data[:8], device_id="device-fixture", link_epoch=2
    )
    with pytest.raises(ValueError, match="duplicate"):
        transfer.accept_chunk(
            chunk_index=0, offset=0, payload=data[:8], device_id="device-fixture", link_epoch=2
        )
    transfer.accept_chunk(
        chunk_index=1, offset=8, payload=data[8:], device_id="device-fixture", link_epoch=2
    )
    with pytest.raises(ValueError, match="digest"):
        transfer.commit(size_bytes=len(data), sha256="0" * 64, link_epoch=2)


def test_interrupted_transfer_cleans_temporary_bytes() -> None:
    transfer = TransferVerifier(
        TransferDeclaration(
            "transfer-3",
            "device-fixture",
            1,
            ContentRefIdentity("content-3", "device-fixture", 1),
            4,
            "0" * 64,
            4,
            1,
        )
    )
    transfer.accept_chunk(
        chunk_index=0, offset=0, payload=b"dead", device_id="device-fixture", link_epoch=1
    )
    assert transfer.temporary_bytes == 4
    transfer.abort()
    assert transfer.temporary_bytes == 0
    with pytest.raises(ValueError, match="closed"):
        transfer.commit(size_bytes=4, sha256="0" * 64, link_epoch=1)


def test_control_frames_preempt_bulk_chunks() -> None:
    scheduler = PriorityScheduler()
    scheduler.push_bulk("chunk-1")
    scheduler.push_bulk("chunk-2")
    scheduler.push_control("response-1")
    scheduler.push_bulk("chunk-3")
    scheduler.push_control("event-1")
    scheduler.push_control("event-2")
    assert scheduler.drain() == [
        "response-1",
        "event-1",
        "chunk-1",
        "event-2",
        "chunk-2",
        "chunk-3",
    ]


def test_transfer_formula_identity_and_empty_file() -> None:
    with pytest.raises(ValueError, match="identity"):
        TransferDeclaration(
            "bad", "device-a", 1, ContentRefIdentity("c", "device-b", 1), 1, "0" * 64, 4, 1
        )
    with pytest.raises(ValueError, match="formula"):
        TransferDeclaration(
            "bad", "device-a", 1, ContentRefIdentity("c", "device-a", 1), 5, "0" * 64, 4, 1
        )
    empty = TransferVerifier(
        TransferDeclaration(
            "empty",
            "device-a",
            1,
            ContentRefIdentity("c", "device-a", 1),
            0,
            hashlib.sha256(b"").hexdigest(),
            4,
            0,
        )
    )
    assert empty.commit(size_bytes=0, sha256=hashlib.sha256(b"").hexdigest(), link_epoch=1) == b""


def test_transfer_rejects_non_contiguous_missing_and_overflow_cases() -> None:
    data = b"12345678"
    declaration = TransferDeclaration(
        "ordered",
        "device-a",
        1,
        ContentRefIdentity("c", "device-a", 1),
        len(data),
        hashlib.sha256(data).hexdigest(),
        4,
        2,
    )
    transfer = TransferVerifier(declaration)
    with pytest.raises(ValueError, match="non-contiguous"):
        transfer.accept_chunk(
            chunk_index=1, offset=4, payload=data[4:], device_id="device-a", link_epoch=1
        )
    transfer.accept_chunk(
        chunk_index=0, offset=0, payload=data[:4], device_id="device-a", link_epoch=1
    )
    with pytest.raises(ValueError, match="length"):
        transfer.accept_chunk(
            chunk_index=1, offset=4, payload=data[4:] + b"x", device_id="device-a", link_epoch=1
        )
    with pytest.raises(ValueError, match="incomplete"):
        transfer.commit(size_bytes=len(data), sha256=declaration.sha256, link_epoch=1)
