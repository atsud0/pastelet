import json
import os
from pathlib import Path
import sys
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


base = os.environ["SMOKE_BASE"]
state_file = Path(os.environ["SMOKE_STATE"])


def request(path, payload=None, headers=None, expected=200):
    outgoing = Request(base + path, data=payload, headers=headers or {})
    try:
        response = urlopen(outgoing, timeout=5)
    except HTTPError as error:
        response = error
    with response:
        content = response.read()
        assert response.status == expected, (response.status, content)
        return json.loads(content)


def wait_ready():
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        try:
            with urlopen(base + "/", timeout=2) as response:
                if response.status == 200:
                    return
        except (URLError, TimeoutError, ConnectionError):
            pass
        time.sleep(0.5)
    raise RuntimeError("Container did not become ready")


wait_ready()
if sys.argv[1] == "create":
    content = "Container persistence: 中文\nsecond line\n"
    response = request("/api/paste", b"unauthorized", {"Content-Type": "text/plain"}, 401)
    assert response["error"]["code"] == "unauthorized"
    headers = {"Content-Type": "application/json", "Authorization": "Bearer smoke-token"}
    created = request("/api/paste", json.dumps({"content": content, "language": "text", "password": "smoke-password", "expires_in": "never"}).encode(), headers, 201)
    paste_path = "/api/paste/" + created["id"]
    request(paste_path, expected=401)
    assert request(paste_path, headers={"X-Paste-Password": "smoke-password"})["content"] == content
    burned = request("/api/paste?burn_after_read=true", b"once", {"Content-Type": "text/plain", "Authorization": "Bearer smoke-token"}, 201)
    burn_path = "/api/paste/" + burned["id"]
    assert request(burn_path)["content"] == "once"
    request(burn_path, expected=404)
    state_file.write_text(json.dumps({"path": paste_path, "content": content}), encoding="utf-8")
elif sys.argv[1] == "verify":
    state = json.loads(state_file.read_text(encoding="utf-8"))
    assert request(state["path"], headers={"X-Paste-Password": "smoke-password"})["content"] == state["content"]
else:
    raise ValueError("Expected create or verify")
