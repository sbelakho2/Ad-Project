#!/usr/bin/env python3
"""A local Mollie API simulator for provider-sim and live evidence.

It speaks the subset Apex uses, with Mollie's real wire shapes:

  POST /v2/customers                 bearer auth, idempotent
  POST /v2/payments                  first / recurring / oneoff
  GET  /v2/payments/{id}             bearer auth, server-side webhook fetch
  POST /v2/payments             bearer auth, idempotent
  GET  /checkout/{token}             shopper page
  POST /checkout/{token}             complete the payment and deliver the webhook
  GET  /__admin/state                simulator state (tests only)
  POST /__admin/complete/{payment}   complete + deliver
  POST /__admin/deliver/{payment}    deliver again (replay)
  POST /__admin/fail-next            fault injection

Mollie webhooks are unsigned: a delivery is form-encoded `id=tr_...` only.
The simulator never puts status, amount or metadata in the delivery, so a
consumer that trusts the body instead of fetching the payment cannot pass
the live drive.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import threading
import urllib.error
import urllib.parse
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

API_KEY = os.environ.get("MOLLIE_API_KEY", "test_mollie_key")
PAYMENT_ID = re.compile(r"^tr_[A-Za-z0-9_]{1,64}$")
LINK_ID = re.compile(r"^pl_[A-Za-z0-9_]{1,64}$")


def new_id(prefix: str) -> str:
    return f"{prefix}_{uuid.uuid4().hex[:18]}"


class State:
    def __init__(self) -> None:
        self.lock = threading.RLock()
        self.base = "http://127.0.0.1:0"
        self.customers: dict[str, dict[str, Any]] = {}
        self.mandates: dict[str, dict[str, Any]] = {}
        self.payments: dict[str, dict[str, Any]] = {}
        self.links: dict[str, dict[str, Any]] = {}
        self.idempotency: dict[tuple[str, str], tuple[int, dict[str, Any]]] = {}
        self.deliveries: list[dict[str, Any]] = []
        self.failures: list[dict[str, Any]] = []

    def amount(self, value: Any) -> tuple[str, str]:
        if not isinstance(value, dict):
            raise ValueError("amount must be an object")
        currency = value.get("currency")
        text = value.get("value")
        if (
            not isinstance(currency, str)
            or len(currency) != 3
            or not currency.isupper()
            or not isinstance(text, str)
            or not re.fullmatch(r"[0-9]{1,15}\.[0-9]{2}", text)
        ):
            raise ValueError("amount must be {currency: ISO, value: decimal string}")
        return currency, text

    def payment_view(self, payment: dict[str, Any]) -> dict[str, Any]:
        view = {
            "resource": "payment",
            "id": payment["id"],
            "status": payment["status"],
            "amount": payment["amount"],
            "description": payment["description"],
            "redirectUrl": payment["redirectUrl"],
            "webhookUrl": payment["webhookUrl"],
            "metadata": payment["metadata"],
            "sequenceType": payment.get("sequenceType", "oneoff"),
            "createdAt": payment["createdAt"],
        }
        if payment.get("customerId"):
            view["customerId"] = payment["customerId"]
        if payment.get("mandateId"):
            view["mandateId"] = payment["mandateId"]
        if payment["status"] == "paid":
            view["paidAt"] = payment["createdAt"]
        if payment.get("checkout"):
            view["_links"] = {"checkout": {"href": payment["checkout"], "type": "text/html"}}
        return view

    def deliver(self, payment_id: str) -> tuple[int, str]:
        payment = self.payments.get(payment_id)
        if payment is None:
            return 0, "unknown payment"
        body = urllib.parse.urlencode({"id": payment_id}).encode("ascii")
        request = urllib.request.Request(
            payment["webhookUrl"],
            data=body,
            method="POST",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        )
        status = 0
        text = ""
        try:
            with urllib.request.urlopen(request, timeout=20) as response:
                status = response.status
                text = response.read().decode("utf-8", "replace")
        except urllib.error.HTTPError as error:
            status = error.code
            text = error.read().decode("utf-8", "replace")
        except Exception as error:  # noqa: BLE001 - evidence, not control flow
            text = f"{type(error).__name__}: {error}"
        delivery = {"payment": payment_id, "status": status, "body": text}
        with self.lock:
            self.deliveries.append(delivery)
        return status, text


STATE = State()


class Handler(BaseHTTPRequestHandler):
    server_version = "mollie-sim/1.0"

    def log_message(self, fmt: str, *args: Any) -> None:  # noqa: ANN401
        if os.environ.get("MOLLIE_SIM_VERBOSE") == "1":
            print(f"[sim] {fmt % args}", flush=True)

    def body(self) -> bytes:
        length = int(self.headers.get("Content-Length", "0") or "0")
        return self.rfile.read(length) if length else b""

    def json_body(self) -> dict[str, Any]:
        raw = self.body()
        if not raw:
            raise ValueError("body is required")
        value = json.loads(raw)
        if not isinstance(value, dict):
            raise ValueError("body must be a JSON object")
        return value

    def send_json(self, status: int, value: dict[str, Any]) -> None:
        raw = json.dumps(value, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/hal+json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def send_text(self, status: int, text: str, content_type: str = "text/plain") -> None:
        raw = text.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def fail_injected(self, path: str) -> bool:
        with STATE.lock:
            for index, item in enumerate(STATE.failures):
                if path.startswith(item.get("prefix", "")):
                    if item["times"] <= 1:
                        STATE.failures.pop(index)
                    else:
                        item["times"] -= 1
                    self.send_json(item["status"], {"status": item["status"], "title": "Injected fault"})
                    return True
        return False

    def authorized(self) -> bool:
        header = self.headers.get("Authorization", "")
        if header != f"Bearer {API_KEY}":
            self.send_json(401, {"status": 401, "title": "Unauthorized"})
            return False
        return True

    def do_GET(self) -> None:  # noqa: N802
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        if path == "/__admin/state":
            with STATE.lock:
                self.send_json(
                    200,
                    {
                        "base": STATE.base,
                        "apiKey": API_KEY,
                        "customers": STATE.customers,
                        "mandates": STATE.mandates,
                        "payments": {key: STATE.payment_view(value) for key, value in STATE.payments.items()},
                        "links": STATE.links,
                        "deliveries": STATE.deliveries,
                    },
                )
            return
        if path.startswith("/checkout/"):
            token = path[len("/checkout/") :]
            with STATE.lock:
                payment_id = STATE.links.get(token, {}).get("paymentId") or (
                    token if token in STATE.payments else None
                )
                payment = STATE.payments.get(payment_id or "")
            if payment is None:
                self.send_text(404, "unknown checkout")
                return
            self.send_text(
                200,
                f"""<!doctype html><title>Mollie test checkout</title>
<p>Amount {payment['amount']['value']} {payment['amount']['currency']}</p>
<form method="post" action="/checkout/{token}"><button type="submit">Pay</button></form>""",
                "text/html; charset=utf-8",
            )
            return
        if path.startswith("/v2/payments/"):
            if not self.authorized():
                return
            payment_id = path.rsplit("/", 1)[-1]
            with STATE.lock:
                payment = STATE.payments.get(payment_id)
            if payment is None:
                self.send_json(404, {"status": 404, "title": "Not Found"})
            else:
                self.send_json(200, STATE.payment_view(payment))
            return
        self.send_json(404, {"status": 404, "title": "Not Found"})

    def do_POST(self) -> None:  # noqa: N802
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        if path == "/__admin/reset":
            with STATE.lock:
                STATE.customers.clear()
                STATE.mandates.clear()
                STATE.payments.clear()
                STATE.links.clear()
                STATE.idempotency.clear()
                STATE.deliveries.clear()
                STATE.failures.clear()
            self.send_json(200, {"reset": True})
            return
        if path == "/__admin/fail-next":
            try:
                value = self.json_body()
                item = {
                    "prefix": str(value.get("prefix", "")),
                    "status": int(value.get("status", 500)),
                    "times": int(value.get("times", 1)),
                }
            except (ValueError, TypeError) as error:
                self.send_json(400, {"error": str(error)})
                return
            with STATE.lock:
                STATE.failures.append(item)
            self.send_json(200, {"failures": STATE.failures})
            return
        if path.startswith("/__admin/complete/") or path.startswith("/__admin/deliver/"):
            payment_id = path.rsplit("/", 1)[-1]
            with STATE.lock:
                payment = STATE.payments.get(payment_id)
                if payment is None:
                    self.send_json(404, {"error": "unknown payment"})
                    return
                payment["status"] = "paid"
                payment["paidAt"] = "2026-01-01T00:00:00+00:00"
                if payment.get("sequenceType") == "first" and payment.get("customerId"):
                    mandate_id = payment.setdefault("mandateId", new_id("mdt"))
                    STATE.mandates[mandate_id] = {
                        "customerId": payment["customerId"],
                        "status": "valid",
                    }
            status, text = STATE.deliver(payment_id)
            self.send_json(200, {"deliveryStatus": status, "deliveryBody": text})
            return
        if path.startswith("/checkout/"):
            token = path[len("/checkout/") :]
            with STATE.lock:
                payment_id = STATE.links.get(token, {}).get("paymentId") or (
                    token if token in STATE.payments else None
                )
                payment = STATE.payments.get(payment_id or "")
                if payment is not None:
                    payment["status"] = "paid"
                    payment["paidAt"] = "2026-01-01T00:00:00+00:00"
                    if payment.get("sequenceType") == "first" and payment.get("customerId"):
                        mandate_id = payment.setdefault("mandateId", new_id("mdt"))
                        STATE.mandates[mandate_id] = {
                            "customerId": payment["customerId"],
                            "status": "valid",
                        }
            if payment is None:
                self.send_text(404, "unknown checkout")
                return
            STATE.deliver(payment["id"])
            self.send_text(200, "paid; webhook delivered", "text/html; charset=utf-8")
            return
        if self.fail_injected(path):
            return
        if not path.startswith("/v2/"):
            self.send_json(404, {"status": 404, "title": "Not Found"})
            return
        if not self.authorized():
            return
        idempotency = self.headers.get("Idempotency-Key")
        if idempotency:
            with STATE.lock:
                cached = STATE.idempotency.get((path, idempotency))
            if cached is not None:
                self.send_json(*cached)
                return
        try:
            value = self.json_body()
        except ValueError as error:
            self.send_json(422, {"status": 422, "title": str(error)})
            return
        if path == "/v2/customers":
            name = value.get("name")
            if not isinstance(name, str) or not name.strip():
                self.send_json(422, {"status": 422, "title": "name is required"})
                return
            customer_id = new_id("cst")
            with STATE.lock:
                STATE.customers[customer_id] = {"id": customer_id, "name": name}
            response = (201, {"resource": "customer", "id": customer_id, "name": name})
        elif path == "/v2/payments":
            try:
                amount = STATE.amount(value.get("amount"))
                for key in ("description", "redirectUrl", "webhookUrl"):
                    if not isinstance(value.get(key), str) or not value[key]:
                        raise ValueError(f"{key} is required")
                metadata = value.get("metadata")
                if not isinstance(metadata, dict) or not isinstance(metadata.get("reference"), str):
                    raise ValueError("metadata.reference is required")
                sequence = value.get("sequenceType", "oneoff")
                if sequence not in ("first", "recurring", "oneoff"):
                    raise ValueError("sequenceType must be first, recurring or oneoff")
                customer_id = value.get("customerId")
                mandate_id = value.get("mandateId")
                if sequence == "first" and not customer_id:
                    raise ValueError("first payment requires customerId")
                if sequence == "recurring":
                    if not customer_id or not mandate_id:
                        raise ValueError("recurring payment requires customerId and mandateId")
                    mandate = STATE.mandates.get(mandate_id)
                    if mandate is None or mandate["customerId"] != customer_id:
                        raise ValueError("unknown mandate for customer")
            except ValueError as error:
                self.send_json(422, {"status": 422, "title": str(error)})
                return
            payment_id = new_id("tr")
            status = "paid" if sequence == "recurring" else "open"
            payment = {
                "id": payment_id,
                "status": status,
                "amount": {"currency": amount[0], "value": amount[1]},
                "description": value["description"],
                "redirectUrl": value["redirectUrl"],
                "webhookUrl": value["webhookUrl"],
                "metadata": value["metadata"],
                "sequenceType": sequence,
                "createdAt": "2026-01-01T00:00:00+00:00",
                "checkout": f"{STATE.base}/checkout/{payment_id}",
            }
            if customer_id:
                payment["customerId"] = customer_id
            if mandate_id:
                payment["mandateId"] = mandate_id
            with STATE.lock:
                STATE.payments[payment_id] = payment
            response = (201, STATE.payment_view(payment))
            if status == "paid":
                STATE.deliver(payment_id)
        else:
            self.send_json(404, {"status": 404, "title": "Not Found"})
            return
        if idempotency:
            with STATE.lock:
                STATE.idempotency[(path, idempotency)] = response
        self.send_json(*response)


def main() -> int:
    global API_KEY
    parser = argparse.ArgumentParser()
    parser.add_argument("--bind", default="127.0.0.1:0")
    parser.add_argument("--api-key", default=API_KEY)
    args = parser.parse_args()
    API_KEY = args.api_key
    host, port_text = args.bind.rsplit(":", 1)
    server = ThreadingHTTPServer((host, int(port_text)), Handler)
    STATE.base = f"http://{host}:{server.server_address[1]}"
    print(json.dumps({"listening": STATE.base}), flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
