#!/usr/bin/env python3
"""Drive the real apex-api against the Mollie simulator.

The simulator is the only payment provider. The run proves, over real HTTP:

  * a payment is created with bearer auth, a decimal amount string, the
    description/redirectUrl/webhookUrl and metadata reference;
  * the webhook is an unsigned form `id=...`, and the API fetches the payment
    server-side before recording it;
  * a forged body (status/amount in the form) is ignored;
  * a replay is acknowledged but recorded once;
  * an unknown or underpaid payment is refused;
  * payment links and recurring customer/mandate payments use Mollie's shapes.

Results are written to `<workdir>/live-evidence.json`.
"""

from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
SIM = Path(__file__).resolve().with_name("server.py")
API_KEY = "mollie_live_sim_key"


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(method: str, url: str, body: bytes | None = None, headers: dict[str, str] | None = None) -> tuple[int, bytes]:
    req = urllib.request.Request(url, data=body, method=method, headers=headers or {})
    try:
        with urllib.request.urlopen(req, timeout=30) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def json_request(method: str, url: str, value: Any | None = None, headers: dict[str, str] | None = None) -> tuple[int, Any]:
    hdrs = {"Content-Type": "application/json", **(headers or {})}
    body = None if value is None else json.dumps(value).encode("utf-8")
    status, raw = request(method, url, body, hdrs)
    try:
        return status, json.loads(raw) if raw else None
    except json.JSONDecodeError:
        return status, raw.decode("utf-8", "replace")


class Evidence:
    def __init__(self, workdir: Path) -> None:
        self.workdir = workdir
        self.checks: list[dict[str, Any]] = []

    def check(self, name: str, condition: bool, detail: Any = None) -> None:
        record = {"name": name, "ok": bool(condition), "detail": detail}
        self.checks.append(record)
        print(f"[{'PASS' if condition else 'FAIL'}] {name}" + (f" :: {detail}" if detail is not None else ""))
        if not condition:
            raise AssertionError(f"{name}: {detail}")

    def write(self) -> Path:
        path = self.workdir / "live-evidence.json"
        path.write_text(json.dumps({"checks": self.checks}, indent=2), encoding="utf-8")
        return path


def wait_for(url: str, timeout: float = 20.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            status, _ = request("GET", url)
            if status < 500:
                return
        except OSError:
            pass
        time.sleep(0.1)
    raise TimeoutError(url)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--workdir", default=None)
    args = parser.parse_args()
    workdir = Path(args.workdir or tempfile.mkdtemp(prefix="apex-mollie-live-")).resolve()
    workdir.mkdir(parents=True, exist_ok=True)
    evidence = Evidence(workdir)

    sim_port = free_port()
    api_port = free_port()
    sim = f"http://127.0.0.1:{sim_port}"
    api = f"http://127.0.0.1:{api_port}"
    webhook = f"{api}/v1/mollie/webhook"
    collection = workdir / "collection.jsonl"

    sim_proc = subprocess.Popen(
        [sys.executable, str(SIM), "--bind", f"127.0.0.1:{sim_port}", "--api-key", API_KEY],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    api_proc: subprocess.Popen[str] | None = None
    try:
        wait_for(f"{sim}/__admin/state")
        binary = ROOT / "rust" / "target" / "debug" / "apex-api"
        if not binary.exists():
            subprocess.run(["cargo", "build", "-p", "apex-api"], cwd=ROOT / "rust", check=True)
        log = workdir / "apex-api.log"
        log_file = log.open("w", encoding="utf-8")
        env = {
            **os.environ,
            "APEX_API_BIND": f"127.0.0.1:{api_port}",
            "APEX_CONSOLE_URL": api,
            "APEX_COLLECTION_FILE": str(collection),
            "APEX_MOLLIE_API_KEY": API_KEY,
            "APEX_MOLLIE_API_BASE": sim,
            "APEX_MOLLIE_WEBHOOK_URL": webhook,
            "APEX_MOLLIE_REDIRECT_URL": f"{api}/watch/requested",
            "RUST_LOG": "warn",
        }
        api_proc = subprocess.Popen(
            [str(binary)],
            cwd=ROOT,
            env=env,
            stdout=log_file,
            stderr=subprocess.STDOUT,
            text=True,
        )
        wait_for(f"{api}/metrics")
        auth = {"Authorization": f"Bearer {API_KEY}"}

        def create_payment(reference: str, value: str, metadata_amount: str | None = None, **extra: Any) -> tuple[int, Any]:
            payload = {
                "amount": {"currency": "USD", "value": value},
                "description": f"live {reference}",
                "redirectUrl": f"{api}/watch/requested",
                "webhookUrl": webhook,
                "metadata": {
                    "reference": reference,
                    "publisher": "publisher-pub-live",
                    "amount": metadata_amount or value,
                },
                "sequenceType": "oneoff",
                **extra,
            }
            return json_request("POST", f"{sim}/v2/payments", payload, auth)

        status, payment = create_payment("apex-fee:pub-live:exp-live:2026-10", "13.38")
        evidence.check("payment create", status == 201 and payment["status"] == "open", payment)
        payment_id = payment["id"]
        evidence.check("payment id shape", payment_id.startswith("tr_"), payment_id)

        status, _ = request("GET", f"{sim}/v2/payments/{payment_id}")
        evidence.check("fetch requires bearer auth", status == 401, status)

        status, completed = json_request("POST", f"{sim}/__admin/complete/{payment_id}")
        evidence.check("webhook delivered", status == 200 and completed["deliveryStatus"] == 200, completed)
        lines = [json.loads(line) for line in collection.read_text(encoding="utf-8").splitlines() if line]
        evidence.check("fetched payment recorded", len(lines) == 1 and lines[0]["paymentId"] == payment_id and lines[0]["status"] == "paid", lines)

        # A replay is acknowledged but deduplicated.
        status, completed = json_request("POST", f"{sim}/__admin/deliver/{payment_id}")
        evidence.check(
            "replay is deduplicated",
            status == 200 and completed["deliveryStatus"] == 200 and json.loads(completed["deliveryBody"]).get("new") is False,
            completed,
        )
        lines = [json.loads(line) for line in collection.read_text(encoding="utf-8").splitlines() if line]
        evidence.check("replay wrote no second event", len(lines) == 1, len(lines))

        # The form body is not trusted: forged fields change nothing.
        forged = urllib.parse.urlencode({"id": payment_id, "status": "failed", "amount": "0.01"}).encode()
        status, raw = request("POST", webhook, forged, {"Content-Type": "application/x-www-form-urlencoded"})
        ack = json.loads(raw) if raw else {}
        evidence.check("forged form fields ignored", status == 200 and ack.get("status") == "paid", ack)

        # Unknown ids fail closed.
        status, raw = request(
            "POST",
            webhook,
            urllib.parse.urlencode({"id": "tr_unknown"}).encode(),
            {"Content-Type": "application/x-www-form-urlencoded"},
        )
        evidence.check("unknown webhook id is refused", status == 400 and json.loads(raw)["error"] == "unknown_payment", raw.decode())

        # Underpaid / inconsistent metadata is refused.
        status, underpaid = create_payment("apex-fee:pub-live:underpaid:2026-10", "1.00", metadata_amount="2.00")
        evidence.check("underpaid payment created for the hostile case", status == 201, underpaid)
        status, completed = json_request("POST", f"{sim}/__admin/complete/{underpaid['id']}")
        evidence.check(
            "underpaid webhook is refused",
            status == 200 and completed["deliveryStatus"] == 400 and json.loads(completed["deliveryBody"])["error"] == "invalid_payment",
            completed,
        )

        # Payment links use /v2/payment-links and a checkout URL.
        status, link = json_request(
            "POST",
            f"{sim}/v2/payment-links",
            {
                "amount": {"currency": "USD", "value": "4.00"},
                "description": "live link",
                "redirectUrl": f"{api}/watch/requested",
                "webhookUrl": webhook,
                "metadata": {"reference": "apex-fee:pub-live:link:2026-10", "publisher": "publisher-pub-live", "amount": "4.00"},
            },
            {**auth, "Idempotency-Key": "apex-fee:pub-live:link:2026-10"},
        )
        evidence.check(
            "payment link shape",
            status == 201 and link["id"].startswith("pl_") and link["_links"]["paymentLink"]["href"].startswith(f"{sim}/checkout/"),
            link,
        )
        status, page = request("GET", link["_links"]["paymentLink"]["href"])
        evidence.check("shopper checkout renders", status == 200 and b"Pay" in page, status)

        # Recurring: create a customer, complete a first payment, charge the mandate.
        status, customer = json_request("POST", f"{sim}/v2/customers", {"name": "publisher-pub-live"}, auth)
        evidence.check("customer created", status == 201 and customer["id"].startswith("cst_"), customer)
        customer_id = customer["id"]
        status, first = create_payment(
            "apex-fee:pub-live:first:2026-10",
            "13.38",
            customerId=customer_id,
            sequenceType="first",
        )
        evidence.check("first payment created", status == 201 and first["sequenceType"] == "first", first)
        status, completed = json_request("POST", f"{sim}/__admin/complete/{first['id']}")
        evidence.check("first payment delivered", status == 200 and completed["deliveryStatus"] == 200, completed)
        status, state = json_request("GET", f"{sim}/__admin/state")
        mandate = next(iter(state["mandates"].items()))[0]
        status, recurring = create_payment(
            "apex-fee:pub-live:recurring:2026-10",
            "13.38",
            customerId=customer_id,
            mandateId=mandate,
            sequenceType="recurring",
        )
        evidence.check(
            "recurring payment uses customer and mandate",
            status == 201 and recurring["status"] == "paid" and recurring["customerId"] == customer_id and recurring["mandateId"] == mandate,
            recurring,
        )

        path = evidence.write()
        print(f"evidence: {path}")
        return 0
    finally:
        if api_proc is not None:
            api_proc.terminate()
            try:
                api_proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                api_proc.kill()
        sim_proc.terminate()
        try:
            sim_proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            sim_proc.kill()


if __name__ == "__main__":
    raise SystemExit(main())
