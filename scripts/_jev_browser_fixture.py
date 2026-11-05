"""Frozen loopback pages for browser selection experiments; no model or browser clients.

Only HTTP rendering is public observation. ``cases`` and ``/state`` are private
harness interfaces and must never be copied into model observations.
"""

from __future__ import annotations

import hashlib
import html
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Literal, NotRequired, TypedDict
from urllib.parse import parse_qs, unquote, urlsplit


class Control(TypedDict):
    key: str
    label: str
    context: str
    disabled: NotRequired[bool]


class Case(TypedDict):
    id: str
    family: str
    kind: Literal["exact", "semantic", "negative"]
    intent: str
    exact_name: NotRequired[str]
    expected_key: str | None
    controls: list[Control]


def _case(
    case_id: str,
    family: str,
    kind: Literal["exact", "semantic", "negative"],
    intent: str,
    selected: int | None,
    rows: list[tuple[str, str] | tuple[str, str, bool]],
) -> Case:
    controls: list[Control] = []
    for index, row in enumerate(rows):
        control: Control = {
            "key": f"private-{case_id}-{index}",
            "label": row[0],
            "context": row[1],
        }
        if len(row) == 3:
            control["disabled"] = row[2]
        controls.append(control)
    result: Case = {
        "id": case_id,
        "family": family,
        "kind": kind,
        "intent": intent,
        "expected_key": controls[selected]["key"] if selected is not None else None,
        "controls": controls,
    }
    if kind == "exact":
        assert selected is not None
        result["exact_name"] = controls[selected]["label"]
    return result


def build_cases() -> list[Case]:
    """Return fresh copies of the twenty authored cases, in frozen evaluation order."""
    return [
        _case(
            "editor-01",
            "editor",
            "exact",
            "Click Word count.",
            1,
            [
                ("Find in draft", "Review tools — article: Night trains"),
                ("Word count", "Review tools — article: Night trains"),
                ("Reading mode", "Preview — article: Night trains"),
                ("Print proof", "Preview — article: Night trains"),
            ],
        ),
        _case(
            "editor-02",
            "editor",
            "semantic",
            "Keep my current edits without making them visible to readers.",
            2,
            [
                ("Publish now", "Release — this article is currently private"),
                ("Schedule", "Release — choose a future publication time"),
                ("Save draft", "Working copy — changes since 14:20 have not been saved"),
                ("Discard edits", "Working copy — restore the last saved version"),
            ],
        ),
        _case(
            "editor-03",
            "editor",
            "semantic",
            "Bring back the previous version of the paragraph I just changed.",
            0,
            [
                ("Undo", "Editing — most recent change: replaced a paragraph"),
                ("Redo", "Editing — reapply an undone change"),
                ("Restore revision", "History — replace the entire article with yesterday's copy"),
                ("Clear formatting", "Typography — retain text and remove styling"),
            ],
        ),
        _case(
            "editor-04",
            "editor",
            "semantic",
            "Let me see how this draft will look on a phone before publishing.",
            3,
            [
                ("Open", "Desktop preview — full-width page"),
                ("Publish", "Release — make this draft public"),
                ("Open", "Tablet preview — medium-width page"),
                ("Open", "Mobile preview — narrow screen with stacked sections"),
            ],
        ),
        _case(
            "editor-05",
            "editor",
            "negative",
            "Translate this article into Italian.",
            None,
            [
                ("Spellcheck", "Language tools — check spelling in English"),
                ("Find and replace", "Editing — replace matching text"),
                ("Choose language", "Proofreading — change the spelling dictionary"),
                ("Export text", "Download — plain text file"),
            ],
        ),
        _case(
            "notification-01",
            "notification setup",
            "exact",
            "Click Test sound.",
            2,
            [
                ("Mute until morning", "Quiet hours — personal notification settings"),
                ("Change tone", "Audio — chime selected"),
                ("Test sound", "Audio — play the selected chime once"),
                ("Save preferences", "Personal notification settings"),
            ],
        ),
        _case(
            "notification-02",
            "notification setup",
            "semantic",
            "Stop the overnight pings but allow them again automatically in the morning.",
            1,
            [
                ("Turn off", "All alerts — remains off until manually enabled"),
                ("Enable", "Quiet hours — suppress alerts daily from 22:00 to 08:00"),
                ("Enable", "Priority bypass — urgent alerts ignore quiet hours"),
                ("Silence audio", "Sound only — banners continue to appear"),
            ],
        ),
        _case(
            "notification-03",
            "notification setup",
            "semantic",
            "Send me one email with the day's activity instead of a separate email for every update.",
            3,
            [
                ("Use this", "Immediate delivery — an email for each event"),
                ("Use this", "Weekly summary — Monday morning digest"),
                ("Pause email", "Delivery — retain in-app alerts only"),
                ("Use this", "Daily digest — combined activity email at 18:00"),
            ],
        ),
        _case(
            "notification-04",
            "notification setup",
            "semantic",
            "Make sure I hear about outages even while my scheduled quiet period is on.",
            0,
            [
                ("Allow", "Critical incidents — bypass quiet hours for service outages"),
                ("Allow", "Product news — announcements and tips"),
                ("Edit schedule", "Quiet hours — daily suppression window"),
                ("Increase volume", "Audio — changes chime loudness"),
            ],
        ),
        _case(
            "notification-05",
            "notification setup",
            "negative",
            "Turn on text messages to my phone.",
            None,
            [
                (
                    "Enable SMS",
                    "Text messages — unavailable until a phone number is verified",
                    True,
                ),
                ("Enable email", "Email — verified address on file"),
                ("Enable push", "Desktop — browser notification permission granted"),
                ("Save", "Delivery preferences — no pending changes"),
            ],
        ),
        _case(
            "record-01",
            "record management",
            "exact",
            "Click Add customer.",
            0,
            [
                ("Add customer", "Customer directory — 28 active accounts"),
                ("Import CSV", "Bulk operations — customer directory"),
                ("Export list", "Bulk operations — customer directory"),
                ("Show archived", "View options — customer directory"),
            ],
        ),
        _case(
            "record-02",
            "record management",
            "semantic",
            "Remove this closed account from the active list while keeping its history available.",
            2,
            [
                ("Delete permanently", "Closed account — Cedar Books; erases account and history"),
                ("Mark active", "Closed account — Cedar Books; return to active service"),
                ("Archive", "Closed account — Cedar Books; retain records outside active list"),
                ("Remove filter", "Directory view — show all account states"),
            ],
        ),
        _case(
            "record-03",
            "record management",
            "semantic",
            "Start a separate invoice using the same line items as the one on screen.",
            1,
            [
                ("Edit", "Invoice INV-206 — changes this existing invoice"),
                ("Duplicate", "Invoice INV-206 — create a new draft with copied line items"),
                ("Export PDF", "Invoice INV-206 — download a read-only copy"),
                ("Issue credit", "Invoice INV-206 — reverse billed amounts"),
            ],
        ),
        _case(
            "record-04",
            "record management",
            "semantic",
            "Asha Patel's address is outdated; open her details so I can fix it.",
            3,
            [
                ("Edit", "Arun Patel — account owner; Mumbai office"),
                ("Edit", "Asha Rao — billing contact; Pune office"),
                ("View audit log", "Contacts — recent changes across all accounts"),
                ("Edit", "Asha Patel — billing contact; Jaipur office"),
            ],
        ),
        _case(
            "record-05",
            "record management",
            "negative",
            "Open the account for Morgan Lee.",
            None,
            [
                ("Open account", "Morgan Lee — North branch; customer since 2021"),
                ("Open account", "Morgan Lee — South branch; customer since 2021"),
                ("Add contact", "Directory — create a new person"),
                ("Export contacts", "Directory — download the visible list"),
            ],
        ),
        _case(
            "file-01",
            "file workflow",
            "exact",
            "Click New folder.",
            3,
            [
                ("Upload files", "Project files — Community garden"),
                ("Sort by date", "Project files — newest first"),
                ("List view", "Project files — display preferences"),
                ("New folder", "Project files — Community garden"),
            ],
        ),
        _case(
            "file-02",
            "file workflow",
            "semantic",
            "Get this proposal onto my laptop without changing the shared version.",
            0,
            [
                ("Download", "Proposal.odt — copy to this device"),
                ("Move", "Proposal.odt — relocate the shared file"),
                ("Make available offline", "Proposal.odt — cache inside this app"),
                ("Replace file", "Proposal.odt — upload a newer shared version"),
            ],
        ),
        _case(
            "file-03",
            "file workflow",
            "semantic",
            "Put the accidentally deleted budget workbook back where it was.",
            2,
            [
                ("Download", "Trash — Budget.xlsx; save a local copy"),
                ("Delete forever", "Trash — Budget.xlsx; cannot be recovered afterward"),
                ("Restore", "Trash — Budget.xlsx; return to Finance folder"),
                ("Restore", "Trash — Venue-plan.pdf; return to Events folder"),
            ],
        ),
        _case(
            "file-04",
            "file workflow",
            "semantic",
            "Prepare one compressed download containing every file in this selected folder.",
            1,
            [
                ("Download", "Selected item — readme.txt only"),
                ("Download ZIP", "Selected folder — Workshop handouts; 12 files"),
                ("Share link", "Selected folder — Workshop handouts; online access"),
                ("Compress images", "Optimization — reduce image sizes in place"),
            ],
        ),
        _case(
            "file-05",
            "file workflow",
            "negative",
            "Recover an earlier version of this file.",
            None,
            [
                ("Download", "Sketch.svg — current version only"),
                ("Rename", "Sketch.svg — change the filename"),
                ("Move to trash", "Sketch.svg — remove from active files"),
                ("Share", "Sketch.svg — grant access to collaborators"),
            ],
        ),
    ]


def _token(case: Case, index: int, run: str) -> str:
    # Independent of the oracle, labels, and control keys.
    return (
        "b"
        + hashlib.sha256(f"browser-fixture-v1:{run}:{case['id']}:{index}".encode()).hexdigest()[:18]
    )


def _render(case: Case, run: str) -> bytes:
    groups: dict[str, list[str]] = {}
    for index, control in enumerate(case["controls"]):
        disabled = " disabled" if control.get("disabled", False) else ""
        token = _token(case, index, run)
        groups.setdefault(control["context"], []).append(
            f'<button data-ref="{token}"{disabled}>{html.escape(control["label"])}</button>'
        )
    content = "".join(
        f"<fieldset><legend>{html.escape(context)}</legend>{''.join(buttons)}</fieldset>"
        for context, buttons in groups.items()
    )
    # No case metadata or private key is embedded; the run nonce is harness-generated.
    return (
        """<!doctype html><html lang="en"><meta charset="utf-8">
<title>Workspace</title><style>
body {font: 17px system-ui; margin: 28px; max-width: 1000px; color: #18232e; background: #f6f8fa}
h1 {font-size: 24px} fieldset {margin: 12px 0; padding: 14px; border: 1px solid #aab4bf; border-radius: 6px}
legend {padding: 0 6px} button {font: inherit; padding: 9px 16px; margin: 3px 8px 3px 0}
#status {min-height: 28px; margin-top: 20px}
</style><body><h1>Workspace</h1><main>"""
        + content
        + """</main>
<footer id="status" role="status">Ready.</footer><script>
const run = """
        + json.dumps(run).replace("<", "\\u003c")
        + """;
document.querySelectorAll('button').forEach(button => {
  button.addEventListener('click', async () => {
    const response = await fetch('/action', {method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({run, token: button.dataset.ref})});
    document.getElementById('status').textContent = response.ok ? 'Action recorded.' : 'Action unavailable.';
  });
});
</script></body></html>"""
    ).encode()


class _Run(TypedDict):
    case_id: str
    clicked_keys: list[str]


class Fixture:
    """Serve real pages on an ephemeral IPv4 loopback port until closed."""

    def __init__(self) -> None:
        self.cases = build_cases()
        self._by_id = {case["id"]: case for case in self.cases}
        self._runs: dict[str, _Run] = {}
        self._lock = threading.Lock()
        self._closed = False
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, format: str, *args: object) -> None:
                pass

            def reply(self, status: int, payload: object, *, page: bool = False) -> None:
                body = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
                self.send_response(status)
                self.send_header(
                    "Content-Type", "text/html; charset=utf-8" if page else "application/json"
                )
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self) -> None:
                url = urlsplit(self.path)
                if url.path == "/health":
                    self.reply(200, {"ok": True})
                    return
                if url.path.startswith("/case/"):
                    case = fixture._by_id.get(unquote(url.path.removeprefix("/case/")))
                    run = parse_qs(url.query).get("run", [""])[0]
                    if case is None or not run:
                        self.reply(404, {"error": "Unknown page or missing run"})
                        return
                    with fixture._lock:
                        fixture._runs[run] = {"case_id": case["id"], "clicked_keys": []}
                    self.reply(200, _render(case, run), page=True)
                    return
                if url.path.startswith("/state/"):
                    run = unquote(url.path.removeprefix("/state/"))
                    with fixture._lock:
                        state = fixture._runs.get(run)
                        if state is None:
                            self.reply(404, {"error": "Unknown run"})
                            return
                        case = fixture._by_id[state["case_id"]]
                        clicked = list(state["clicked_keys"])
                    outcome = "pending" if case["expected_key"] else "abstained"
                    if clicked:
                        outcome = "matched" if clicked == [case["expected_key"]] else "mismatched"
                    self.reply(
                        200,
                        {
                            **state,
                            "clicked_keys": clicked,
                            "outcome": outcome,
                            "acknowledged": bool(clicked),
                        },
                    )
                    return
                self.reply(404, {"error": "Unknown route"})

            def do_POST(self) -> None:
                if self.path != "/action":
                    self.reply(404, {"error": "Unknown route"})
                    return
                try:
                    payload = json.loads(
                        self.rfile.read(int(self.headers.get("Content-Length", "0")))
                    )
                    if (
                        not isinstance(payload, dict)
                        or not isinstance(payload.get("run"), str)
                        or not isinstance(payload.get("token"), str)
                    ):
                        raise ValueError("Invalid action")
                except (ValueError, TypeError):
                    self.reply(400, {"error": "Invalid action"})
                    return
                with fixture._lock:
                    state = fixture._runs.get(payload["run"])
                    if state is None:
                        self.reply(404, {"error": "Unknown run"})
                        return
                    case = fixture._by_id[state["case_id"]]
                    control = next(
                        (
                            control
                            for index, control in enumerate(case["controls"])
                            if _token(case, index, payload["run"]) == payload["token"]
                        ),
                        None,
                    )
                    if control is None or control.get("disabled", False):
                        self.reply(409, {"error": "Unavailable control"})
                        return
                    state["clicked_keys"].append(control["key"])
                self.reply(200, {"ok": True})

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self.origin = f"http://127.0.0.1:{self._server.server_port}"
        self._thread.start()

    def __enter__(self) -> Fixture:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def close(self) -> None:
        if not self._closed:
            self._closed = True
            self._server.shutdown()
            self._server.server_close()
            self._thread.join(timeout=5)
