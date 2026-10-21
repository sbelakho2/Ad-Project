#!/usr/bin/env python3
"""Drive the real apex-api and apex-watch through the Adyen simulator.

This harness starts:

  * tools/provider-sim/adyen/server.py on a free loopback port,
  * rust/target/debug/apex-api pointed at the simulator,
  * rust/target/debug/apex-watch `bill` for monthly stored-card charges,

and exercises the real HTTP/form/webhook paths end to end. Every assertion is
an observable result from a real process or from the simulator's state:

  signup -> payment link -> shopper checkout -> HMAC webhook -> active watch
  duplicate/out-of-order webhooks, refund/chargeback/dispute events
  monthly stored-card charge, rerun (no double charge), refund transitions
  injected 401/429/500/malformed-JSON/timeout faults
  bad HMAC, malformed webhook, oversized body

No credentials are used. The Adyen HMAC key is a random local hex value and
the simulator signs with the raw bytes it decodes to, exactly as the product
does. Results are written to `<workdir>/live-evidence.json`.
"""

from __future__ import annotations

import argparse
import datetime as dt
import http.client
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import time
import urllib.parse
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
SIM = Path(__file__).resolve().with_name("server.py")
HMAC_HEX = "00112233445566778899aabbccddeeff"
API_KEY = "adyen_live_sim_key"
MERCHANT = "ApexMerchant"
TENANT = "live-provider-sim.example"


class Evidence:
    def __init__(self) -> None:
        self.checks: list[dict[str, Any]] = []
        self.commands: list[dict[str, Any]] = []

    def check(self, name: str, condition: bool, detail: Any = None) -> None:
        record = {"name": name, "ok": bool(condition), "detail": detail}
        self.checks.append(record)
        marker = "PASS" if condition else "FAIL"
        print(f"[{marker}] {name}" + (f" :: {detail}" if detail is not None else ""))
        if not condition:
            raise AssertionError(f"{name}: {detail}")

    def note(self, name: str, detail: Any) -> None:
        self.checks.append({"name": name, "ok": True, "detail": detail, "note": True})
        print(f"[note] {name} :: {detail}")

    def command(self, command: str, result: subprocess.CompletedProcess[str]) -> None:
        self.commands.append(
            {
                "command": command,
                "exit": result.returncode,
                "stdout": result.stdout,
                "stderr": result.stderr,
            }
        )

    def write(self, path: Path) -> None:
        path.write_text(
            json.dumps({"checks": self.checks, "commands": self.commands}, indent=2),
            encoding="utf-8",
        )


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def http_call(
    method: str,
    url: str,
    body: bytes | str | None = None,
    headers: dict[str, str] | None = None,
    timeout: float = 60.0,
) -> tuple[int, dict[str, str], bytes]:
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != "http":
        raise ValueError(f"harness only speaks http: {url}")
    connection = http.client.HTTPConnection(
        parsed.hostname, parsed.port or 80, timeout=timeout
    )
    path = parsed.path or "/"
    if parsed.query:
        path += f"?{parsed.query}"
    if isinstance(body, str):
        body = body.encode("utf-8")
    connection.request(method, path, body=body, headers=headers or {})
    response = connection.getresponse()
    payload = response.read()
    response_headers = {name.lower(): value for name, value in response.getheaders()}
    connection.close()
    return response.status, response_headers, payload


def json_call(
    method: str,
    url: str,
    payload: Any = None,
    headers: dict[str, str] | None = None,
    timeout: float = 60.0,
) -> tuple[int, dict[str, str], Any]:
    body = None
    all_headers = dict(headers or {})
    if payload is not None:
        body = json.dumps(payload).encode("utf-8")
        all_headers["Content-Type"] = "application/json"
    status, response_headers, raw = http_call(
        method, url, body, all_headers, timeout=timeout
    )
    text = raw.decode("utf-8", "replace")
    try:
        parsed = json.loads(text) if text else None
    except json.JSONDecodeError:
        parsed = text
    return status, response_headers, parsed


def form_call(
    url: str, fields: dict[str, str], timeout: float = 60.0
) -> tuple[int, dict[str, str], str]:
    body = urllib.parse.urlencode(fields)
    status, headers, raw = http_call(
        "POST",
        url,
        body,
        {"Content-Type": "application/x-www-form-urlencoded"},
        timeout=timeout,
    )
    return status, headers, raw.decode("utf-8", "replace")


def wait_health(url: str, process: subprocess.Popen[Any], timeout: float = 90.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"process exited before health: {process.args}")
        try:
            status, _, _ = http_call("GET", url, timeout=2)
            if status == 200:
                return
        except OSError:
            pass
        time.sleep(0.2)
    raise RuntimeError(f"{url} did not become healthy")


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            rows.append(json.loads(line))
    return rows


def latest_rows(path: Path) -> dict[str, dict[str, Any]]:
    folded: dict[str, dict[str, Any]] = {}
    for row in read_jsonl(path):
        folded[row["id"]] = row
    return folded


def add_months(month: str, count: int) -> str:
    year, number = (int(part) for part in month.split("-"))
    index = year * 12 + (number - 1) + count
    return f"{index // 12:04d}-{index % 12 + 1:02d}"


def current_month() -> str:
    today = dt.datetime.now(dt.timezone.utc).date()
    return f"{today.year:04d}-{today.month:02d}"


def parse_checkout_url(html: str) -> str | None:
    matches = re.findall(r'href="(http://[^"]+/checkout/PL[^"]+)"', html)
    if matches:
        return matches[0]
    matches = re.findall(r'(http://[^\s"<]+/checkout/PL[^\s"<]+)', html)
    return matches[0] if matches else None


def parse_hidden(html: str, name: str) -> str | None:
    match = re.search(
        rf'name="{re.escape(name)}"\s+value="([^"]*)"', html
    )
    return match.group(1) if match else None


def parse_confirm_tokens(mail: str) -> list[str]:
    # Each message is multipart (text + HTML), so dedupe while preserving
    # order to get one token per email.
    return list(
        dict.fromkeys(re.findall(r"/watch/confirm\?token=([0-9a-f]{32})", mail))
    )


def parse_confirm_token(mail: str) -> str | None:
    tokens = parse_confirm_tokens(mail)
    return tokens[0] if tokens else None


class Live:
    def __init__(self, workdir: Path, evidence: Evidence) -> None:
        self.workdir = workdir
        self.evidence = evidence
        self.sim_port = free_port()
        self.api_port = free_port()
        self.api = f"http://127.0.0.1:{self.api_port}"
        self.sim = f"http://127.0.0.1:{self.sim_port}"
        self.sim_state = workdir / "sim-state.json"
        self.collection_path = workdir / "collection.jsonl"
        self.watch_path = workdir / "watch" / "subscribers.jsonl"
        self.outcomes_path = workdir / "outcomes.jsonl"
        self.mail_path = workdir / "mail.log"
        self.seller_path = workdir / "seller.json"
        self.sendmail_path = workdir / "sendmail_capture.sh"
        self.api_log = workdir / "apex-api.log"
        self.sim_log = workdir / "adyen-sim.log"
        self.api_process: subprocess.Popen[Any] | None = None
        self.sim_process: subprocess.Popen[Any] | None = None

    # -- setup -----------------------------------------------------------

    def prepare(self) -> None:
        self.workdir.mkdir(parents=True, exist_ok=True)
        (self.workdir / "watch").mkdir(exist_ok=True)
        self.mail_path.write_text("", encoding="utf-8")
        self.seller_path.write_text(
            json.dumps(
                {
                    "legalName": "Apex Media OU",
                    "addressLines": ["Sepapaja tn 6", "15551 Tallinn"],
                    "country": "EE",
                    "vatNumber": "EE123456789",
                    "vatRateBp": 0,
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        self.sendmail_path.write_text(
            "#!/bin/sh\n"
            f"cat >> {self.mail_path}\n"
            f"printf '\\n---APEX-MAIL-END---\\n' >> {self.mail_path}\n"
            "exit 0\n",
            encoding="utf-8",
        )
        self.sendmail_path.chmod(0o755)

    def process_env(self) -> dict[str, str]:
        env = dict(os.environ)
        env.update(
            {
                "APEX_API_BIND": f"127.0.0.1:{self.api_port}",
                "APEX_CONSOLE_URL": self.api,
                "APEX_WATCH_ROOT": str(self.workdir / "watch"),
                "APEX_COLLECTION_FILE": str(self.collection_path),
                "APEX_LEADS_FILE": str(self.workdir / "leads.jsonl"),
                "APEX_SNAPSHOT_ROOT": str(self.workdir / "snapshots"),
                "APEX_LEDGER_ROOT": str(self.workdir / "ledgers"),
                "APEX_SALES_OUTCOMES_FILE": str(self.outcomes_path),
                "APEX_BILLING_PROFILE": str(self.seller_path),
                "APEX_BILLING_VAT_RATE_BP": "0",
                "APEX_ADYEN_API_KEY": API_KEY,
                "APEX_ADYEN_MERCHANT_ACCOUNT": MERCHANT,
                "APEX_ADYEN_CHECKOUT_BASE": self.sim,
                "APEX_ADYEN_HMAC_KEY": HMAC_HEX,
                "APEX_NOTIFY_SINK": "email",
                "APEX_NOTIFY_FROM": "billing@apex.test",
                "APEX_NOTIFY_SENDMAIL": str(self.sendmail_path),
                "RUST_LOG": "info",
            }
        )
        return env

    def build(self) -> None:
        command = ["cargo", "build", "-p", "apex-api", "-p", "apex-watch"]
        result = subprocess.run(
            command,
            cwd=ROOT / "rust",
            capture_output=True,
            text=True,
        )
        self.evidence.command(" ".join(command), result)
        if result.returncode != 0:
            raise RuntimeError(f"build failed: {result.stderr[-4000:]}")

    def start_simulator(self) -> None:
        command = [
            sys.executable,
            str(SIM),
            "--port",
            str(self.sim_port),
            "--api-key",
            API_KEY,
            "--merchant-account",
            MERCHANT,
            "--hmac-key-hex",
            HMAC_HEX,
            "--webhook-url",
            f"{self.api}/v1/adyen/webhook",
            "--state-file",
            str(self.sim_state),
        ]
        log = self.sim_log.open("w", encoding="utf-8")
        self.sim_process = subprocess.Popen(
            command,
            cwd=self.workdir,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        wait_health(f"{self.sim}/health", self.sim_process)

    def start_api(self) -> None:
        command = [str(ROOT / "rust" / "target" / "debug" / "apex-api")]
        log = self.api_log.open("w", encoding="utf-8")
        self.api_process = subprocess.Popen(
            command,
            cwd=self.workdir,
            env=self.process_env(),
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        wait_health(f"{self.api}/healthz", self.api_process)

    def stop(self) -> None:
        for process in (self.api_process, self.sim_process):
            if process is None or process.poll() is not None:
                continue
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

    # -- simulator helpers ----------------------------------------------

    def sim_state_view(self) -> dict[str, Any]:
        status, _, payload = json_call("GET", f"{self.sim}/_control/state")
        assert status == 200, payload
        return payload

    def sim_config(self, **values: Any) -> dict[str, Any]:
        status, _, payload = json_call("POST", f"{self.sim}/_control/config", values)
        assert status == 200, payload
        return payload

    def sim_fault(self, **fault: Any) -> dict[str, Any]:
        status, _, payload = json_call("POST", f"{self.sim}/_control/faults", fault)
        assert status == 200, payload
        return payload

    def clear_faults(self) -> None:
        json_call("POST", f"{self.sim}/_control/faults/clear", {})

    def control(self, path: str, payload: dict[str, Any] | None = None) -> Any:
        status, _, body = json_call(
            "POST", f"{self.sim}/_control/{path}", payload or {}
        )
        assert status == 200, (path, status, body)
        return body

    def deliver(self, event_id: str, **options: Any) -> dict[str, Any]:
        return self.control(f"events/{event_id}/deliver", options)

    def payments_for(self, reference: str) -> list[dict[str, Any]]:
        state = self.sim_state_view()
        return [
            payment
            for payment in state["payments"]
            if payment["reference"] == reference
        ]

    def events_for(self, event_code: str, psp: str | None = None) -> list[dict[str, Any]]:
        state = self.sim_state_view()
        return [
            event
            for event in state["events"]
            if event["eventCode"] == event_code
            and (psp is None or event["pspReference"] == psp)
        ]

    def pending_3ds(self) -> dict[str, Any] | None:
        state = json.loads(self.sim_state.read_text(encoding="utf-8"))
        pending = state.get("idempotency", {}).get("pending3ds", {})
        return next(iter(pending.values()), None) if pending else None

    # -- product helpers -------------------------------------------------

    def signup(self, domain: str, email: str, apps: str = "com.example.app1") -> tuple[str, str]:
        status, headers, html = form_call(
            f"{self.api}/watch",
            {"domain": domain, "email": email, "apps": apps},
        )
        self.evidence.check(
            f"POST /watch {domain} is a 303 redirect",
            status == 303,
            {"status": status, "location": headers.get("location")},
        )
        location = headers.get("location", "")
        reference = location.rsplit("ref=", 1)[-1]
        self.evidence.check(
            f"signup {domain} returned a subscriber reference",
            reference.startswith("sub_"),
            reference,
        )
        confirmation_status, _, page = http_call(
            "GET", f"{self.api}{location}"
        )[:3]
        self.evidence.check(
            "confirmation page renders",
            confirmation_status == 200,
            confirmation_status,
        )
        return reference, page.decode("utf-8", "replace")

    def run_watch_bill(self, month: str, timeout: float = 120.0) -> subprocess.CompletedProcess[str]:
        command = [
            str(ROOT / "rust" / "target" / "debug" / "apex-watch"),
            "bill",
            month,
        ]
        result = subprocess.run(
            command,
            cwd=self.workdir,
            env=self.process_env(),
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        self.evidence.command(" ".join(command), result)
        return result

    # -- scenarios -------------------------------------------------------

    def scenario_signup_webhook(self) -> str:
        ref, page = self.signup(
            TENANT, f"ops@{TENANT}", "com.example.app1\ncom.example.app2"
        )
        # Q: the checkout link waits for the billing profile.
        self.evidence.check(
            "signup renders without a checkout link",
            parse_checkout_url(page) is None,
            page[:200],
        )
        self.confirm_and_profile(ref)
        status, _, receipt = http_call(
            "GET", f"{self.api}/watch/requested?ref={ref}"
        )
        checkout = parse_checkout_url(receipt.decode("utf-8", "replace"))
        self.evidence.check(
            "the receipt carries the simulator checkout link after the profile",
            status == 200 and checkout is not None and checkout.startswith(self.sim),
            {"status": status, "checkout": checkout},
        )
        assert checkout is not None
        status, _, checkout_page = http_call("GET", checkout)
        self.evidence.check(
            "the returned checkout link is a real shopper page",
            status == 200 and b"Apex setup watch checkout" in checkout_page,
            status,
        )
        # Use the link's own form, exactly as a shopper would.
        status, _, paid_page = form_call(
            f"{checkout}/pay", {"result": "Authorised"}
        )
        self.evidence.check(
            "shopper completes the link with Authorised",
            status == 200 and "Payment Authorised" in paid_page,
            status,
        )

        auth = self.events_for("AUTHORISATION")
        self.evidence.check(
            "simulator emitted a signed AUTHORISATION webhook",
            len(auth) == 1 and auth[0]["deliveries"] == 1,
            auth,
        )
        self.evidence.check(
            "webhook delivery was accepted by apex-api (200)",
            auth[0]["lastDelivery"]["status"] == 200,
            auth[0]["lastDelivery"],
        )
        rows = latest_rows(self.watch_path)
        subscriber = rows[ref]
        self.evidence.check(
            "real subscriber row is active after the webhook",
            subscriber["status"] == "active" and subscriber.get("paidAt"),
            {"status": subscriber["status"], "paidAt": subscriber.get("paidAt")},
        )
        self.evidence.check(
            "the stored card reported by the webhook is recorded",
            subscriber.get("storedPaymentMethodId")
            == auth[0]["storedPaymentMethodId"],
            subscriber.get("storedPaymentMethodId"),
        )
        collection = read_jsonl(self.collection_path)
        self.evidence.check(
            "apex-api recorded exactly one accepted collection event",
            len(collection) == 1
            and collection[0]["eventCode"] == "AUTHORISATION"
            and collection[0]["success"] == "true",
            collection,
        )

        # Replay the same signed delivery: accepted, not recorded again.
        rows_before = len(read_jsonl(self.watch_path))
        replay = self.control(
            f"events/{auth[0]['id']}/deliver", {"count": 3}
        )
        statuses = [item["status"] for item in replay["deliveries"]]
        self.evidence.check(
            "three duplicate deliveries are all accepted",
            statuses == [200, 200, 200],
            statuses,
        )
        responses = [json.loads(item["response"]) for item in replay["deliveries"]]
        self.evidence.check(
            "duplicates report new=0 (replay-protected)",
            all(response["new"] == 0 for response in responses),
            responses,
        )
        self.evidence.check(
            "replays append no subscriber row and keep one collection event",
            len(read_jsonl(self.watch_path)) == rows_before
            and len(read_jsonl(self.collection_path)) == 1,
            {
                "subscriberRows": len(read_jsonl(self.watch_path)),
                "collectionEvents": len(read_jsonl(self.collection_path)),
            },
        )
        return ref

    def scenario_out_of_order(self) -> str:
        ref, page = self.signup(
            f"outoforder.{TENANT}",
            f"ops@outoforder.{TENANT}",
            "com.example.app1",
        )
        self.confirm_and_profile(ref)
        status, _, receipt = http_call(
            "GET", f"{self.api}/watch/requested?ref={ref}"
        )
        self.evidence.check(
            "the out-of-order receipt carries the checkout link after the profile",
            status == 200,
            status,
        )
        checkout = parse_checkout_url(receipt.decode("utf-8", "replace"))
        assert checkout is not None
        link_id = checkout.rsplit("/", 1)[-1]
        # Create the authorisation without delivering it, then deliver a
        # refund first: Adyen retries out of order.
        outcome = self.control(
            f"links/{link_id}/complete",
            {"result": "Authorised", "deliver": False},
        )
        psp = outcome["payment"]["pspReference"]
        auth_event = outcome["event"]["id"]
        refund = self.control(
            f"payments/{psp}/refund",
            {"deliver": True},
        )
        self.evidence.check(
            "REFUND delivered before AUTHORISATION is accepted",
            refund["event"]["deliveries"][-1]["status"] == 200,
            refund["event"]["deliveries"],
        )
        pending = latest_rows(self.watch_path)[ref]
        self.evidence.check(
            "a refund alone does not activate the watch",
            pending["status"] == "pending",
            pending["status"],
        )
        delivered = self.deliver(auth_event)
        self.evidence.check(
            "the late AUTHORISATION still activates the watch",
            delivered["deliveries"][-1]["status"] == 200
            and latest_rows(self.watch_path)[ref]["status"] == "active",
            {
                "delivery": delivered["deliveries"][-1],
                "status": latest_rows(self.watch_path)[ref]["status"],
            },
        )
        # The remaining signed funds events are recorded and never activate.
        for kind, event_code in (
            ("notification-of-chargeback", "NOTIFICATION_OF_CHARGEBACK"),
            ("dispute", "DISPUTE"),
            ("chargeback", "CHARGEBACK"),
        ):
            payload = {"deliver": True}
            if kind == "chargeback":
                payload["amount"] = 9_900
            outcome = self.control(f"payments/{psp}/{kind}", payload)
            self.evidence.check(
                f"{event_code} event delivered",
                outcome["event"]["deliveries"][-1]["status"] == 200,
                outcome["event"]["deliveries"],
            )
        events = [row["eventCode"] for row in read_jsonl(self.collection_path)]
        for code in (
            "REFUND",
            "AUTHORISATION",
            "NOTIFICATION_OF_CHARGEBACK",
            "DISPUTE",
            "CHARGEBACK",
        ):
            self.evidence.check(
                f"apex-api collection log contains {code}",
                code in events,
                events,
            )
        self.evidence.check(
            "funds events leave the subscriber active (no double activation)",
            latest_rows(self.watch_path)[ref]["status"] == "active",
            latest_rows(self.watch_path)[ref]["status"],
        )
        return ref

    def confirm_and_profile(self, ref: str, token_index: int | None = None) -> None:
        mail = self.mail_path.read_text(encoding="utf-8")
        tokens = parse_confirm_tokens(mail)
        token = (
            tokens[-1]
            if token_index is None and tokens
            else (tokens[token_index] if token_index is not None and len(tokens) > token_index else None)
        )
        self.evidence.check(
            "the real confirmation email carries a one-time token",
            token is not None,
            mail[-400:],
        )
        assert token is not None
        status, _, landing = http_call(
            "GET", f"{self.api}/watch/confirm?token={token}"
        )
        self.evidence.check(
            "confirmation GET is inert",
            status == 200 and b"<form" in landing,
            status,
        )
        status, _, confirmed = form_call(
            f"{self.api}/watch/confirm", {"token": token}
        )
        confirmed_text = confirmed
        profile_token = parse_hidden(confirmed_text, "profile_token")
        self.evidence.check(
            "confirmation POST yields the billing-profile token",
            status == 200 and profile_token is not None,
            {"status": status, "profileToken": bool(profile_token)},
        )
        assert profile_token is not None
        status, headers, _ = form_call(
            f"{self.api}/watch/billing-profile",
            {
                "profile_token": profile_token,
                "reference": ref,
                "legal_name": "Live Buyer LLC",
                "address_lines": "1 Market Street\nSan Francisco, CA 94105",
                "country": "US",
                "vat_number": "",
            },
        )
        self.evidence.check(
            "billing profile is stored before payment",
            status == 303,
            {"status": status, "location": headers.get("location")},
        )
        row = latest_rows(self.watch_path)[ref]
        # Q: no invoice exists until the payment is accepted; the profile
        # alone must not create a billable stage.
        self.evidence.check(
            "no first invoice is issued before payment",
            row.get("firstInvoiceNumber") is None,
            row.get("firstInvoiceNumber"),
        )

    def scenario_monthly(self, ref: str, current: str) -> str:
        month = add_months(current, 1)
        result = self.run_watch_bill(month)
        self.evidence.check(
            "real apex-watch bill charges the stored card",
            result.returncode == 0 and "charged" in result.stdout,
            {"exit": result.returncode, "stdout": result.stdout, "stderr": result.stderr},
        )
        reference = f"watch:{ref}:{month}"
        payments = self.payments_for(reference)
        self.evidence.check(
            "simulator has exactly one captured stored-card payment",
            len(payments) == 1
            and payments[0]["captured"] == 9_900
            and payments[0]["amount"]["currency"] == "USD"
            and payments[0]["storedPaymentMethodId"],
            payments,
        )
        psp = payments[0]["pspReference"]
        row = latest_rows(self.watch_path)[ref]
        self.evidence.check(
            "subscriber row records the paid month and no charge error",
            row.get("lastPaidMonth") == month
            and row.get("lastChargeError") is None,
            {"paidMonth": row.get("lastPaidMonth"), "error": row.get("lastChargeError")},
        )

        # The same month again must not issue or charge a second time.
        again = self.run_watch_bill(month)
        self.evidence.check(
            "rerunning the month skips the paid invoice",
            again.returncode == 0 and "skipped" in again.stdout,
            {"exit": again.returncode, "stdout": again.stdout},
        )
        self.evidence.check(
            "rerun created no second payment (no double charge)",
            len(self.payments_for(reference)) == 1,
            self.payments_for(reference),
        )
        return psp

    def scenario_refund_chargeback(self, ref: str, psp: str, current: str) -> None:
        month = add_months(current, 1)
        reference = f"watch:{ref}:{month}"
        refund = self.control(f"payments/{psp}/refund", {"deliver": True})
        self.evidence.check(
            "REFUND transition captures the refund and delivers a webhook",
            refund["payment"]["refunded"] == 9_900
            and refund["payment"]["status"] == "refunded"
            and refund["event"]["deliveries"][-1]["status"] == 200,
            {"payment": refund["payment"], "delivery": refund["event"]["deliveries"]},
        )
        events = read_jsonl(self.collection_path)
        refund_events = [
            event
            for event in events
            if event["eventCode"] == "REFUND"
            and event["pspReference"] == refund["response"]["pspReference"]
        ]
        self.evidence.check(
            "apex-api recorded the REFUND against the original payment",
            len(refund_events) == 1
            and refund_events[0]["merchantReference"] == reference
            and refund_events[0]["amountValue"] == 9_900,
            refund_events,
        )
        replay = self.control(
            f"events/{refund['event']['id']}/deliver", {"count": 2}
        )
        self.evidence.check(
            "REFUND replay is accepted and recorded once",
            all(
                item["status"] == 200 and json.loads(item["response"])["new"] == 0
                for item in replay["deliveries"]
            ),
            replay["deliveries"],
        )
        for kind, code in (
            ("notification-of-chargeback", "NOTIFICATION_OF_CHARGEBACK"),
            ("dispute", "DISPUTE"),
            ("chargeback", "CHARGEBACK"),
        ):
            outcome = self.control(f"payments/{psp}/{kind}", {"deliver": True})
            self.evidence.check(
                f"{code} transition delivered to the real product",
                outcome["event"]["deliveries"][-1]["status"] == 200,
                outcome["event"]["deliveries"],
            )
        codes = {event["eventCode"] for event in read_jsonl(self.collection_path)}
        for code in ("REFUND", "CHARGEBACK", "DISPUTE", "NOTIFICATION_OF_CHARGEBACK"):
            self.evidence.check(
                f"collection log contains {code}",
                code in codes,
                sorted(codes),
            )
        self.evidence.check(
            "funds events never changed the active subscriber",
            latest_rows(self.watch_path)[ref]["status"] == "active",
            latest_rows(self.watch_path)[ref]["status"],
        )

    def scenario_bad_deliveries(self) -> None:
        auth = self.events_for("AUTHORISATION")[0]
        before = len(read_jsonl(self.collection_path))
        corrupt = self.control(
            f"events/{auth['id']}/deliver", {"corrupt_hmac": True}
        )
        self.evidence.check(
            "bad HMAC is refused by the real webhook (400 invalid_signature)",
            corrupt["deliveries"][-1]["status"] == 400
            and json.loads(corrupt["deliveries"][-1]["response"])["error"]
            == "invalid_signature",
            corrupt["deliveries"][-1],
        )
        tampered = self.control(
            f"events/{auth['id']}/deliver", {"mutate_amount": 1}
        )
        self.evidence.check(
            "tampered amount with the old signature is refused",
            tampered["deliveries"][-1]["status"] == 400
            and json.loads(tampered["deliveries"][-1]["response"])["error"]
            == "invalid_signature",
            tampered["deliveries"][-1],
        )
        self.evidence.check(
            "invalid deliveries record nothing",
            len(read_jsonl(self.collection_path)) == before,
            {"before": before, "after": len(read_jsonl(self.collection_path))},
        )
        status, _, raw = http_call(
            "POST",
            f"{self.api}/v1/adyen/webhook",
            b"{not json",
            {"Content-Type": "application/json"},
        )
        self.evidence.check(
            "malformed JSON is 400 malformed_event",
            status == 400 and b"malformed_event" in raw,
            raw.decode("utf-8", "replace"),
        )
        status, _, raw = http_call(
            "POST",
            f"{self.api}/v1/adyen/webhook",
            b"x" * (256 * 1024 + 1),
            {"Content-Type": "application/json"},
        )
        self.evidence.check(
            "oversized webhook body is a JSON 413",
            status == 413 and b"bad_request" in raw,
            {"status": status, "body": raw.decode("utf-8", "replace")},
        )

    def scenario_link_faults(self) -> None:
        # 401
        self.sim_fault(endpoint="paymentLinks", status=401, count=1)
        ref, page = self.signup(
            f"unauth.{TENANT}", f"ops@unauth.{TENANT}", "com.example.app1"
        )
        self.evidence.check(
            "401 from paymentLinks leaves an honest pending signup without a link",
            parse_checkout_url(page) is None
            and "We will email a payment link" in page
            and latest_rows(self.watch_path)[ref]["status"] == "pending",
            {"status": latest_rows(self.watch_path)[ref]["status"], "hasLink": parse_checkout_url(page)},
        )
        self.clear_faults()

        # 429 + Retry-After: direct observation, then the real signup.
        self.sim_fault(
            endpoint="paymentLinks",
            status=429,
            retry_after=2,
            count=2,
        )
        status, headers, raw = json_call(
            "POST",
            f"{self.sim}/v71/paymentLinks",
            {
                "reference": "fault-429",
                "amount": {"value": 100, "currency": "USD"},
                "merchantAccount": MERCHANT,
                "shopperReference": "fault-429",
                "recurringProcessingModel": "Subscription",
                "storePaymentMethod": True,
            },
            headers={"X-API-Key": API_KEY, "Idempotency-Key": "fault-429"},
        )
        self.evidence.check(
            "429 carries Retry-After and no link response",
            status == 429 and headers.get("retry-after") == "2",
            {"status": status, "retryAfter": headers.get("retry-after"), "body": raw},
        )
        ref, page = self.signup(
            f"ratelimit.{TENANT}", f"ops@ratelimit.{TENANT}", "com.example.app1"
        )
        self.evidence.check(
            "429 from paymentLinks leaves the pending signup without a link",
            parse_checkout_url(page) is None
            and "We will email a payment link" in page,
            page[-500:],
        )
        self.clear_faults()

    def scenario_charge_faults(self, ref: str, current: str) -> None:
        # 500 is retried by apex-watch's RetryTransport (Idempotency-Key).
        month = add_months(current, 2)
        self.sim_fault(endpoint="payments", status=500, count=1)
        result = self.run_watch_bill(month)
        reference = f"watch:{ref}:{month}"
        payments = self.payments_for(reference)
        self.evidence.check(
            "500 on the first charge is retried and charged once",
            result.returncode == 0
            and len(payments) == 1
            and payments[0]["captured"] == 9_900,
            {"exit": result.returncode, "payments": payments, "stderr": result.stderr},
        )
        attempts = [
            row
            for row in self.sim_state_view()["requests"]
            if row["path"] == "/v71/payments"
            and row["idempotencyKey"] == reference
        ]
        self.evidence.check(
            "the 500 attempt was retried with the same idempotency key",
            len(attempts) == 2,
            attempts,
        )

        # 429 + Retry-After is honored by the real retry transport.
        month = add_months(current, 3)
        self.sim_fault(
            endpoint="payments", status=429, retry_after=1, count=1
        )
        started = time.time()
        result = self.run_watch_bill(month)
        elapsed = time.time() - started
        reference = f"watch:{ref}:{month}"
        self.evidence.check(
            "429 Retry-After is honored before the retry succeeds",
            result.returncode == 0
            and len(self.payments_for(reference)) == 1
            and elapsed >= 1.0,
            {"exit": result.returncode, "elapsed": round(elapsed, 2), "stderr": result.stderr},
        )

        # Malformed JSON on a 200: typed failure, no commit; the retry with
        # the same reference then succeeds.
        month = add_months(current, 4)
        self.sim_fault(
            endpoint="payments", mode="malformed_json", status=200, count=1
        )
        failed = self.run_watch_bill(month)
        reference = f"watch:{ref}:{month}"
        self.evidence.check(
            "malformed JSON is an honest failure with no committed charge",
            failed.returncode != 0
            and len(self.payments_for(reference)) == 0
            and "malformed" in (failed.stdout + failed.stderr).lower(),
            {
                "exit": failed.returncode,
                "stdout": failed.stdout,
                "stderr": failed.stderr,
                "payments": self.payments_for(reference),
            },
        )
        retried = self.run_watch_bill(month)
        self.evidence.check(
            "the next run retries the same idempotent reference and charges once",
            retried.returncode == 0
            and len(self.payments_for(reference)) == 1,
            {"exit": retried.returncode, "payments": self.payments_for(reference)},
        )

        # Timeout after commit: the product times out at 30s, retries with the
        # same key, and must see the already-committed charge, not a second one.
        month = add_months(current, 5)
        self.sim_fault(
            endpoint="payments_after_commit",
            mode="timeout",
            timeout_seconds=31,
            count=1,
        )
        started = time.time()
        timed = self.run_watch_bill(month, timeout=180)
        elapsed = time.time() - started
        reference = f"watch:{ref}:{month}"
        self.evidence.check(
            "timeout after commit retries idempotently and charges exactly once",
            timed.returncode == 0
            and len(self.payments_for(reference)) == 1
            and elapsed >= 30.0,
            {
                "exit": timed.returncode,
                "elapsed": round(elapsed, 2),
                "payments": self.payments_for(reference),
                "stdout": timed.stdout,
                "stderr": timed.stderr,
            },
        )

        # Refusal is a typed, honest failure; nothing is captured.
        month = add_months(current, 6)
        self.sim_config(charge_result="Refused")
        refused = self.run_watch_bill(month)
        reference = f"watch:{ref}:{month}"
        payments = self.payments_for(reference)
        self.evidence.check(
            "Refused charge is reported honestly and captures nothing",
            refused.returncode != 0
            and len(payments) == 1
            and payments[0]["resultCode"] == "Refused"
            and payments[0]["captured"] == 0,
            {
                "exit": refused.returncode,
                "payments": payments,
                "stdout": refused.stdout,
                "stderr": refused.stderr,
            },
        )

        # Action-required/3DS: the stored charge returns an action and the
        # product reports the typed IdentifyShopper failure; payment details
        # can still finish the authentication.
        month = add_months(current, 7)
        self.sim_config(charge_result="IdentifyShopper")
        three_ds = self.run_watch_bill(month)
        reference = f"watch:{ref}:{month}"
        pending = self.pending_3ds()
        self.evidence.check(
            "IdentifyShopper is an action-required result with no capture",
            three_ds.returncode != 0
            and pending is not None
            and pending["resultCode"] == "IdentifyShopper"
            and len(self.payments_for(reference)) == 0,
            {
                "exit": three_ds.returncode,
                "pending": pending,
                "stderr": three_ds.stderr,
            },
        )
        assert pending is not None
        status, _, details = json_call(
            "POST",
            f"{self.sim}/v71/payments/details",
            {
                "paymentData": pending["paymentData"],
                "details": {"threeDSResult": "challenge-complete"},
            },
            headers={"X-API-Key": API_KEY},
        )
        self.evidence.check(
            "payment details completes the 3DS action as Authorised",
            status == 200 and details.get("resultCode") == "Authorised",
            details,
        )
        self.sim_config(charge_result="Authorised")

    def scenario_webhook_events(self) -> None:
        # The charge webhooks use merchant references outside the watch
        # namespace; they are recorded and must not activate anything.
        codes = [event["eventCode"] for event in read_jsonl(self.collection_path)]
        self.evidence.check(
            "monthly AUTHORISATION webhooks were recorded by the product",
            "AUTHORISATION" in codes,
            sorted(set(codes)),
        )

    def run(self) -> None:
        self.prepare()
        self.start_simulator()
        self.start_api()
        try:
            current = current_month()
            ref = self.scenario_signup_webhook()
            out_of_order_ref = self.scenario_out_of_order()
            psp = self.scenario_monthly(ref, current)
            self.scenario_refund_chargeback(ref, psp, current)
            self.scenario_bad_deliveries()
            self.scenario_link_faults()
            self.scenario_charge_faults(ref, current)
            self.scenario_webhook_events()

            state = self.sim_state_view()
            by_reference: dict[str, int] = {}
            for payment in state["payments"]:
                by_reference[payment["reference"]] = (
                    by_reference.get(payment["reference"], 0) + 1
                )
            duplicates = {
                reference: count
                for reference, count in by_reference.items()
                if count > 1
            }
            self.evidence.check(
                "no reference was charged twice",
                not duplicates,
                duplicates,
            )
            total_captured = sum(payment["captured"] for payment in state["payments"])
            total_refunded = sum(payment["refunded"] for payment in state["payments"])
            total_chargeback = sum(payment["chargeback"] for payment in state["payments"])
            self.evidence.note(
                "provider totals",
                {
                    "payments": len(state["payments"]),
                    "capturedMinorUnits": total_captured,
                    "refundedMinorUnits": total_refunded,
                    "chargebackMinorUnits": total_chargeback,
                    "events": len(state["events"]),
                    "webhooksDelivered": state["counters"]["webhooks"],
                },
            )
        finally:
            self.stop()
            self.evidence.write(self.workdir / "live-evidence.json")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workdir", default=None)
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    workdir = (
        Path(args.workdir)
        if args.workdir
        else Path(tempfile.mkdtemp(prefix="apex-adyen-live-"))
    )
    evidence = Evidence()
    live = Live(workdir, evidence)
    try:
        if not args.skip_build:
            live.build()
        live.run()
    except Exception as error:  # noqa: BLE001 - report the failing assertion
        evidence.note("harness failure", f"{type(error).__name__}: {error}")
        evidence.write(workdir / "live-evidence.json")
        print(f"\nFAILED: {type(error).__name__}: {error}", file=sys.stderr)
        print(f"workdir: {workdir}", file=sys.stderr)
        print(f"api log: {workdir / 'apex-api.log'}", file=sys.stderr)
        print(f"sim log: {workdir / 'adyen-sim.log'}", file=sys.stderr)
        return 1
    print(f"\nLIVE PASS. workdir: {workdir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
