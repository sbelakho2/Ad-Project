#!/usr/bin/env python3
"""Stateful Adyen Checkout simulator for the Apex billing/watch paths.

The simulator implements the subset of the Adyen Checkout API the product
actually calls, with Adyen's documented HMAC scheme:

    POST /v71/paymentLinks
    POST /v71/payments
    POST /v71/payments/details
    POST /v71/payments/{pspReference}/refunds

It also serves a shopper-facing checkout page for a created payment link and
a local control plane (`/_control/*`) used by the live harness:

  * mutable charge/refund/chargeback/dispute state,
  * real HMAC-signed webhook delivery to the product,
  * duplicate/out-of-order webhook delivery,
  * configurable result codes (Authorised, Refused, 3DS action-required),
  * injected faults (401, 429 + Retry-After, 500, malformed JSON, timeout,
    connection drop, oversized request) with bounded counts.

Adyen signs eight fields joined with ":" after escaping backslash (`\\`) and
colon (`\\:`). `amount.value` and the other string fields are escaped exactly
as the documentation specifies. The HMAC key is consumed as raw bytes decoded
from the hex form Adyen publishes (the same value the product decodes).

Standard library only. Bind to localhost; the control plane is unauthenticated
by default (use `--control-token` when exposing it).
"""

from __future__ import annotations

import argparse
import base64
import copy
import hashlib
import hmac
import json
import os
import random
import re
import socket
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

MAX_API_BODY = 64 * 1024
MAX_CONTROL_BODY = 1024 * 1024
CHECKOUT_VERSION = "v71"
LINK_PATH = f"/{CHECKOUT_VERSION}/paymentLinks"
PAYMENT_PATH = f"/{CHECKOUT_VERSION}/payments"
DETAILS_PATH = f"/{CHECKOUT_VERSION}/payments/details"
REFUND_PATH = re.compile(rf"^/{CHECKOUT_VERSION}/payments/([^/]+)/refunds$")
CHECKOUT_PATH = re.compile(r"^/checkout/([^/]+)(?:/(pay|challenge|done))?$")
CONTROL_EVENT_DELIVER = re.compile(r"^/_control/events/([^/]+)/deliver$")
CONTROL_LINK_COMPLETE = re.compile(r"^/_control/links/([^/]+)/complete$")
CONTROL_PAY_ACTION = re.compile(
    r"^/_control/payments/([^/]+)/(refund|chargeback|dispute|notification-of-chargeback)$"
)

RESULT_CODES = {
    "Authorised",
    "Received",
    "Refused",
    "Error",
    "Cancelled",
    "IdentifyShopper",
    "ChallengeShopper",
    "RedirectShopper",
}
FUNDS_EVENT_CODES = {
    "AUTHORISATION",
    "REFUND",
    "CHARGEBACK",
    "DISPUTE",
    "NOTIFICATION_OF_CHARGEBACK",
}


def now_ms() -> int:
    return int(time.time() * 1000)


def random_hex(length: int) -> str:
    return "".join(random.choice("0123456789abcdef") for _ in range(length))


def psp_reference(prefix: str = "") -> str:
    # Adyen PSP references are long decimal strings; the prefix keeps the
    # simulator's evidence readable without changing the shape too much.
    return f"{prefix}{random.randint(10**15, 10**16 - 1)}"


def escape_signed_field(value: Any) -> str:
    """Adyen's documented HMAC field escaping: backslash first, then colon."""
    text = str(value)
    return text.replace("\\", "\\\\").replace(":", "\\:")


def signed_message(item: dict[str, Any]) -> str:
    amount = item["amount"]
    return ":".join(
        [
            escape_signed_field(item["pspReference"]),
            escape_signed_field(item.get("originalReference", "")),
            escape_signed_field(item["merchantAccountCode"]),
            escape_signed_field(item["merchantReference"]),
            str(amount["value"]),
            escape_signed_field(amount["currency"]),
            escape_signed_field(item["eventCode"]),
            escape_signed_field(item["success"]),
        ]
    )


def sign_item(hmac_key: bytes, item: dict[str, Any]) -> str:
    digest = hmac.new(
        hmac_key, signed_message(item).encode("utf-8"), hashlib.sha256
    ).digest()
    return base64.b64encode(digest).decode("ascii")


def envelope(item: dict[str, Any]) -> dict[str, Any]:
    return {"live": "false", "notificationItems": [{"NotificationRequestItem": item}]}


def canonical(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def api_error(status: int, code: str, message: str) -> dict[str, Any]:
    return {
        "status": status,
        "errorCode": code,
        "message": message,
        "errorType": "validation" if status < 500 else "internal",
    }


class SimulatorState:
    """All mutable provider state, guarded by one re-entrant lock."""

    def __init__(self, config: dict[str, Any], state_file: str | None) -> None:
        self.lock = threading.RLock()
        self.state_file = state_file
        self.config = {
            "api_key": config["api_key"],
            "merchant_account": config["merchant_account"],
            "hmac_key_hex": config["hmac_key_hex"],
            "webhook_url": config.get("webhook_url"),
            "public_base": config.get("public_base"),
            "auto_webhooks": bool(config.get("auto_webhooks", True)),
            "link_default_result": config.get("link_default_result", "Authorised"),
            "charge_result": config.get("charge_result", "Authorised"),
            "refund_result": config.get("refund_result", "received"),
            "control_token": config.get("control_token"),
        }
        self.hmac_key = bytes.fromhex(config["hmac_key_hex"])
        self.links: dict[str, dict[str, Any]] = {}
        self.payments: dict[str, dict[str, Any]] = {}
        self.events: dict[str, dict[str, Any]] = {}
        self.faults: list[dict[str, Any]] = []
        self.requests: list[dict[str, Any]] = []
        self.idempotency: dict[str, dict[str, dict[str, Any]]] = {
            "paymentLinks": {},
            "payments": {},
            "refunds": {},
        }
        self.counters = {
            "paymentLinks": 0,
            "payments": 0,
            "refunds": 0,
            "webhooks": 0,
            "replays": 0,
            "faults": 0,
        }
        self._load()

    # -- persistence -----------------------------------------------------

    def _load(self) -> None:
        if not self.state_file or not os.path.exists(self.state_file):
            return
        with open(self.state_file, "r", encoding="utf-8") as handle:
            saved = json.load(handle)
        self.links = saved.get("links", {})
        self.payments = saved.get("payments", {})
        self.events = saved.get("events", {})
        self.faults = saved.get("faults", [])
        self.requests = saved.get("requests", [])
        self.idempotency = saved.get(
            "idempotency", {"paymentLinks": {}, "payments": {}, "refunds": {}}
        )
        self.counters.update(saved.get("counters", {}))

    def save(self) -> None:
        if not self.state_file:
            return
        # Concurrent requests (the monthly bill charges subscribers in
        # parallel) must not race on one temp file: a failed os.replace used
        # to abort a committed charge's response and make the client retry.
        with self.lock:
            payload = {
                "links": self.links,
                "payments": self.payments,
                "events": self.events,
                "faults": self.faults,
                "requests": self.requests,
                "idempotency": self.idempotency,
                "counters": self.counters,
            }
            temporary = (
                f"{self.state_file}.{os.getpid()}.{threading.get_ident()}.tmp"
            )
            try:
                with open(temporary, "w", encoding="utf-8") as handle:
                    json.dump(payload, handle, indent=2, sort_keys=True)
                os.replace(temporary, self.state_file)
            except OSError as error:
                # Persistence is evidence, not a reason to fail a charge the
                # provider already committed. The in-memory state remains.
                self.counters["saveErrors"] = self.counters.get("saveErrors", 0) + 1
                print(
                    json.dumps(
                        {"event": "adyen-sim-state-save-failed", "error": str(error)}
                    ),
                    file=sys.stderr,
                    flush=True,
                )
                try:
                    os.unlink(temporary)
                except OSError:
                    pass

    # -- helpers ---------------------------------------------------------

    def log_request(self, record: dict[str, Any]) -> None:
        with self.lock:
            self.requests.append(record)
            del self.requests[:-2000]
            self.save()

    def public_base(self, handler: BaseHTTPRequestHandler) -> str:
        configured = self.config.get("public_base")
        if configured:
            return configured.rstrip("/")
        host = handler.headers.get("Host") or f"{handler.server.server_address[0]}:{handler.server.server_address[1]}"
        return f"http://{host}"

    def take_fault(self, endpoint: str) -> dict[str, Any] | None:
        with self.lock:
            for fault in self.faults:
                if fault.get("endpoint") not in (endpoint, "*", "any"):
                    continue
                remaining = int(fault.get("remaining", 1))
                if remaining == 0:
                    continue
                if remaining > 0:
                    fault["remaining"] = remaining - 1
                self.counters["faults"] += 1
                return copy.deepcopy(fault)
        return None

    def idempotency_record(self, scope: str, key: str | None, fingerprint: str) -> tuple[str, dict[str, Any] | None]:
        if not key:
            return "none", None
        with self.lock:
            record = self.idempotency[scope].get(key)
            if record is None:
                return "new", None
            if record["fingerprint"] != fingerprint:
                return "conflict", record
            return "replay", record

    def store_idempotency(
        self, scope: str, key: str | None, fingerprint: str, response: dict[str, Any]
    ) -> None:
        if not key:
            return
        with self.lock:
            self.idempotency[scope][key] = {
                "fingerprint": fingerprint,
                "response": response,
                "at": now_ms(),
            }
            self.save()

    def record_payment(self, payment: dict[str, Any]) -> None:
        with self.lock:
            self.payments[payment["pspReference"]] = payment
            self.save()

    def record_event(
        self,
        event_code: str,
        psp_reference_value: str,
        original_reference: str,
        merchant_reference: str,
        amount_value: int,
        currency: str,
        success: bool,
        stored_payment_method_id: str = "",
        deliver: bool = True,
        event_id: str | None = None,
    ) -> dict[str, Any]:
        item: dict[str, Any] = {
            "pspReference": psp_reference_value,
            "originalReference": original_reference,
            "merchantAccountCode": self.config["merchant_account"],
            "merchantReference": merchant_reference,
            "amount": {"value": amount_value, "currency": currency},
            "eventCode": event_code,
            "success": "true" if success else "false",
            "paymentMethod": "visa",
        }
        additional = {"hmacSignature": ""}
        if stored_payment_method_id:
            additional["storedPaymentMethodId"] = stored_payment_method_id
        item["additionalData"] = additional
        item["additionalData"]["hmacSignature"] = sign_item(self.hmac_key, item)
        event = {
            "id": event_id or f"evt_{random_hex(16)}",
            "createdAt": now_ms(),
            "eventCode": event_code,
            "pspReference": psp_reference_value,
            "originalReference": original_reference,
            "merchantReference": merchant_reference,
            "amount": {"value": amount_value, "currency": currency},
            "success": item["success"],
            "storedPaymentMethodId": stored_payment_method_id,
            "item": item,
            "body": envelope(item),
            "deliveries": [],
        }
        with self.lock:
            self.events[event["id"]] = event
            self.save()
        if deliver:
            self.deliver_event(event["id"])
        return self.events[event["id"]]

    def deliver_event(
        self,
        event_id: str,
        count: int = 1,
        corrupt_hmac: bool = False,
        mutate_amount: int | None = None,
        delay_ms: int = 0,
    ) -> dict[str, Any]:
        with self.lock:
            event = copy.deepcopy(self.events[event_id])
        if delay_ms > 0:
            time.sleep(delay_ms / 1000.0)
        body = copy.deepcopy(event["body"])
        item = body["notificationItems"][0]["NotificationRequestItem"]
        if mutate_amount is not None:
            item["amount"]["value"] = mutate_amount
        if corrupt_hmac:
            signature = item["additionalData"]["hmacSignature"]
            item["additionalData"]["hmacSignature"] = (
                ("A" if signature[:1] != "A" else "B") + signature[1:]
            )
        raw = json.dumps(body, separators=(",", ":")).encode("utf-8")
        url = self.config.get("webhook_url")
        results = []
        for _ in range(max(1, count)):
            result: dict[str, Any] = {
                "at": now_ms(),
                "url": url,
                "status": None,
                "response": None,
                "error": None,
            }
            if url:
                request = urllib.request.Request(
                    url,
                    data=raw,
                    headers={"Content-Type": "application/json"},
                    method="POST",
                )
                try:
                    with urllib.request.urlopen(request, timeout=10) as response:
                        payload = response.read().decode("utf-8", "replace")
                        result["status"] = response.status
                        result["response"] = payload[:2000]
                except urllib.error.HTTPError as error:
                    result["status"] = error.code
                    result["response"] = error.read().decode("utf-8", "replace")[:2000]
                except Exception as error:  # noqa: BLE001 - evidence is the point
                    result["error"] = f"{type(error).__name__}: {error}"
            else:
                result["error"] = "no webhook_url configured"
            results.append(result)
        with self.lock:
            stored = self.events[event_id]
            stored["deliveries"].extend(results)
            self.counters["webhooks"] += len(results)
            if len(results) > 1:
                self.counters["replays"] += len(results) - 1
            self.save()
        return {
            "eventId": event_id,
            "eventCode": event["eventCode"],
            "pspReference": event["pspReference"],
            "signature": item["additionalData"]["hmacSignature"],
            "body": body,
            "deliveries": results,
        }

    def deliver_raw(
        self,
        body: dict[str, Any],
        count: int = 1,
        corrupt_hmac: bool = False,
    ) -> dict[str, Any]:
        event_id = f"evt_raw_{random_hex(16)}"
        item = body["notificationItems"][0]["NotificationRequestItem"]
        if corrupt_hmac:
            signature = item["additionalData"]["hmacSignature"]
            item["additionalData"]["hmacSignature"] = (
                ("A" if signature[:1] != "A" else "B") + signature[1:]
            )
        event = {
            "id": event_id,
            "createdAt": now_ms(),
            "eventCode": item.get("eventCode"),
            "pspReference": item.get("pspReference"),
            "body": body,
            "item": item,
            "deliveries": [],
        }
        with self.lock:
            self.events[event_id] = event
            self.save()
        return self.deliver_event(event_id, count=count)

    # -- payment helpers -------------------------------------------------

    def create_payment_record(
        self,
        reference: str,
        amount_value: int,
        currency: str,
        result_code: str,
        stored_payment_method_id: str = "",
        shopper_reference: str = "",
        status: str | None = None,
        psp: str | None = None,
    ) -> dict[str, Any]:
        psp = psp or psp_reference()
        payment = {
            "pspReference": psp,
            "resultCode": result_code,
            "amount": {"value": amount_value, "currency": currency},
            "reference": reference,
            "merchantReference": reference,
            "merchantAccount": self.config["merchant_account"],
            "shopperReference": shopper_reference,
            "storedPaymentMethodId": stored_payment_method_id,
            "status": status
            or (
                "authorised"
                if result_code in ("Authorised", "Received")
                else "refused"
                if result_code in ("Refused", "Error", "Cancelled")
                else "pending_action"
            ),
            "captured": amount_value if result_code in ("Authorised", "Received") else 0,
            "refunded": 0,
            "chargeback": 0,
            "createdAt": now_ms(),
            "events": [],
        }
        self.record_payment(payment)
        return payment

    def finish_pending_link(self, link_id: str, result: str = "Authorised") -> dict[str, Any]:
        """Complete a link that stopped at a 3DS action requirement."""
        with self.lock:
            link = self.links.get(link_id)
            if link is None:
                return {"error": "unknown_link", "status": 404}
            payment = self.payments.get(link.get("paymentPspReference", ""))
            if payment is None:
                return {"error": "no_pending_payment", "status": 404}
            if payment["status"] != "pending_action":
                return {"link": link, "payment": payment, "replayed": True}
            success = result in ("Authorised", "Received")
            payment["resultCode"] = result
            payment["status"] = "authorised" if success else "refused"
            payment["captured"] = payment["amount"]["value"] if success else 0
            event = self.record_event(
                "AUTHORISATION",
                payment["pspReference"],
                "",
                link["reference"],
                link["amount"]["value"],
                link["amount"]["currency"],
                success,
                link.get("storedPaymentMethodId", ""),
                deliver=True,
            )
            payment["events"].append(event["id"])
            link["status"] = "completed" if success else "refused"
            link.setdefault("eventIds", []).append(event["id"])
            self.save()
            return {"link": link, "payment": payment, "event": event, "replayed": False}

    def complete_link(self, link_id: str, result: str, deliver: bool = True) -> dict[str, Any]:
        with self.lock:
            link = self.links.get(link_id)
            if link is None:
                return {"error": "unknown_link", "status": 404}
            if link.get("paymentPspReference"):
                # Completion is idempotent: the shopper refreshing the result
                # page must not create a second payment.
                payment = self.payments[link["paymentPspReference"]]
                event = next(
                    (
                        self.events[event_id]
                        for event_id in link.get("eventIds", [])
                        if self.events[event_id]["eventCode"] == "AUTHORISATION"
                    ),
                    None,
                )
                return {
                    "link": link,
                    "payment": payment,
                    "event": event,
                    "replayed": True,
                }
            if result not in RESULT_CODES:
                return {"error": "unknown_result_code", "status": 400}
            if result in ("IdentifyShopper", "ChallengeShopper", "RedirectShopper"):
                payment = self.create_payment_record(
                    link["reference"],
                    link["amount"]["value"],
                    link["amount"]["currency"],
                    result,
                    link.get("storedPaymentMethodId", ""),
                    link.get("shopperReference", ""),
                    status="pending_action",
                )
                payment["action"] = self.action_for(result, payment["pspReference"])
                payment["paymentData"] = f"paymentdata_{random_hex(24)}"
                link["paymentPspReference"] = payment["pspReference"]
                link["pendingResult"] = result
                self.save()
                return {"link": link, "payment": payment, "event": None, "action": payment["action"]}
            success = result in ("Authorised", "Received")
            payment = self.create_payment_record(
                link["reference"],
                link["amount"]["value"],
                link["amount"]["currency"],
                result,
                link.get("storedPaymentMethodId", ""),
                link.get("shopperReference", ""),
            )
            event = self.record_event(
                "AUTHORISATION",
                payment["pspReference"],
                "",
                link["reference"],
                link["amount"]["value"],
                link["amount"]["currency"],
                success,
                link.get("storedPaymentMethodId", ""),
                deliver=deliver,
            )
            payment["events"].append(event["id"])
            link["status"] = "completed" if success else "refused"
            link["paymentPspReference"] = payment["pspReference"]
            link["eventIds"] = [event["id"]]
            self.save()
            return {"link": link, "payment": payment, "event": event, "replayed": False}

    @staticmethod
    def action_for(result_code: str, psp: str) -> dict[str, Any]:
        if result_code == "RedirectShopper":
            return {
                "type": "redirect",
                "url": f"https://sim.adyen.local/3ds/redirect?psp={psp}",
                "method": "GET",
                "paymentData": f"paymentdata_{random_hex(12)}",
            }
        return {
            "type": "threeDS2",
            "subtype": "fingerprint" if result_code == "IdentifyShopper" else "challenge",
            "paymentMethodType": "visa",
            "token": f"3ds2_token_{random_hex(16)}",
            "paymentData": f"paymentdata_{random_hex(12)}",
        }

    def refund_payment(
        self,
        psp: str,
        amount_value: int | None,
        reference: str,
        deliver: bool = True,
    ) -> dict[str, Any]:
        with self.lock:
            payment = self.payments.get(psp)
            if payment is None:
                return {"error": "unknown_payment", "status": 404}
            if payment["status"] not in ("authorised", "partially_refunded", "refunded"):
                return {"error": "payment_not_refundable", "status": 422}
            refundable = payment["captured"] - payment["refunded"]
            value = refundable if amount_value is None else amount_value
            if value <= 0 or value > refundable:
                return {
                    "error": "refund_amount_exceeds_captured",
                    "status": 422,
                    "refundable": refundable,
                }
            refund_psp = psp_reference("R")
            payment["refunded"] += value
            payment["status"] = (
                "refunded" if payment["refunded"] >= payment["captured"] else "partially_refunded"
            )
            response = {
                "pspReference": refund_psp,
                "paymentPspReference": psp,
                "status": "received",
                "reference": reference,
                "merchantAccount": self.config["merchant_account"],
                "amount": {"value": value, "currency": payment["amount"]["currency"]},
            }
            event = self.record_event(
                "REFUND",
                refund_psp,
                psp,
                payment["merchantReference"],
                value,
                payment["amount"]["currency"],
                True,
                payment.get("storedPaymentMethodId", ""),
                deliver=deliver,
            )
            payment["events"].append(event["id"])
            self.save()
            return {"payment": payment, "response": response, "event": event}

    def transition(
        self, psp: str, kind: str, amount_value: int | None = None, deliver: bool = True
    ) -> dict[str, Any]:
        with self.lock:
            payment = self.payments.get(psp)
            if payment is None:
                return {"error": "unknown_payment", "status": 404}
            value = payment["captured"] if amount_value is None else amount_value
            if kind == "refund":
                return self.refund_payment(psp, amount_value, f"refund:{psp}", deliver)
            if kind == "chargeback":
                if value <= 0 or value > payment["captured"]:
                    return {"error": "invalid_chargeback_amount", "status": 422}
                payment["chargeback"] = value
                payment["status"] = "chargeback"
                event_code = "CHARGEBACK"
                event_psp = psp_reference("CB")
            elif kind == "notification-of-chargeback":
                event_code = "NOTIFICATION_OF_CHARGEBACK"
                event_psp = psp
            elif kind == "dispute":
                event_code = "DISPUTE"
                event_psp = psp_reference("DSP")
            else:
                return {"error": "unknown_transition", "status": 400}
            event = self.record_event(
                event_code,
                event_psp,
                psp,
                payment["merchantReference"],
                value,
                payment["amount"]["currency"],
                True,
                payment.get("storedPaymentMethodId", ""),
                deliver=deliver,
            )
            payment["events"].append(event["id"])
            self.save()
            return {"payment": payment, "event": event}


class Handler(BaseHTTPRequestHandler):
    server_version = "ApexAdyenSim/1.0"
    protocol_version = "HTTP/1.1"
    simulator: SimulatorState

    # Quiet the default per-request stderr line; we emit structured evidence.
    def log_message(self, fmt: str, *args: Any) -> None:
        return

    # -- plumbing --------------------------------------------------------

    def _read_body(self, cap: int) -> bytes | None:
        length_header = self.headers.get("Content-Length")
        if length_header is None:
            return b""
        try:
            length = int(length_header)
        except ValueError:
            return None
        if length < 0 or length > cap:
            self._json(
                HTTPStatus.REQUEST_ENTITY_TOO_LARGE,
                api_error(413, "000", f"request body exceeds {cap} bytes"),
            )
            return None
        return self.rfile.read(length)

    def _json(self, status: int, payload: Any, headers: dict[str, str] | None = None) -> None:
        raw = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(raw)

    def _html(self, status: int, html: str) -> None:
        raw = html.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def _drop(self) -> None:
        self.close_connection = True
        try:
            self.connection.close()
        except OSError:
            pass

    def _authorized(self) -> bool:
        presented = self.headers.get("X-API-Key") or self.headers.get("x-api-key") or ""
        return hmac.compare_digest(presented, self.simulator.config["api_key"])

    def _control_authorized(self) -> bool:
        token = self.simulator.config.get("control_token")
        if not token:
            return True
        return hmac.compare_digest(self.headers.get("X-Control-Token", ""), token)

    def _apply_fault(self, endpoint: str) -> str | None:
        fault = self.simulator.take_fault(endpoint)
        if fault is None:
            return None
        mode = fault.get("mode", "status")
        delay = float(fault.get("delay_seconds", 0))
        if delay > 0:
            time.sleep(delay)
        if mode == "drop":
            self._drop()
            return "dropped"
        if mode == "malformed_json":
            raw = (fault.get("body") or '{"id":').encode("utf-8")
            self.send_response(int(fault.get("status", 200)))
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)
            return "malformed"
        if mode in ("timeout", "status", "body"):
            status = int(fault.get("status", 500))
            if mode == "timeout":
                # A delay beyond the product's 30s client timeout.
                time.sleep(float(fault.get("timeout_seconds", 31)))
                self._drop()
                return "timeout"
            headers = {}
            if status == 429 and fault.get("retry_after") is not None:
                headers["Retry-After"] = str(int(fault["retry_after"]))
            payload = fault.get("body")
            if payload is None:
                payload = api_error(
                    status,
                    fault.get("error_code", "000"),
                    fault.get("message", f"injected {status}"),
                )
            elif isinstance(payload, str):
                try:
                    payload = json.loads(payload)
                except json.JSONDecodeError:
                    raw = payload.encode("utf-8")
                    self.send_response(status)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(raw)))
                    for name, value in headers.items():
                        self.send_header(name, value)
                    self.end_headers()
                    self.wfile.write(raw)
                    return "fault"
            self._json(status, payload, headers)
            return "fault"
        return None

    def _request_record(self, status: int, operation: str) -> None:
        self.simulator.log_request(
            {
                "at": now_ms(),
                "method": self.command,
                "path": urllib.parse.urlparse(self.path).path,
                "operation": operation,
                "status": status,
                "idempotencyKey": self.headers.get("Idempotency-Key"),
            }
        )

    # -- routing ---------------------------------------------------------

    def do_GET(self) -> None:  # noqa: N802 - stdlib handler API
        path = urllib.parse.urlparse(self.path).path
        if path == "/health":
            self._json(200, {"status": "ok", "now": now_ms()})
            return
        match = CHECKOUT_PATH.match(path)
        if match:
            link_id, action = match.groups()
            if action is None:
                self._checkout_page(link_id)
                return
            self._json(404, api_error(404, "000", "not found"))
            return
        if path == "/_control/state":
            if not self._control_authorized():
                self._json(401, api_error(401, "000", "control token required"))
                return
            self._json(200, self._state_view())
            return
        if path == "/_control/events":
            if not self._control_authorized():
                self._json(401, api_error(401, "000", "control token required"))
                return
            with self.simulator.lock:
                events = [copy.deepcopy(event) for event in self.simulator.events.values()]
            self._json(200, {"events": events})
            return
        if path == "/_control/requests":
            if not self._control_authorized():
                self._json(401, api_error(401, "000", "control token required"))
                return
            with self.simulator.lock:
                requests = copy.deepcopy(self.simulator.requests)
            self._json(200, {"requests": requests})
            return
        self._json(404, api_error(404, "000", "not found"))

    def do_POST(self) -> None:  # noqa: N802 - stdlib handler API
        path = urllib.parse.urlparse(self.path).path
        if path.startswith("/_control/"):
            self._handle_control(path)
            return
        if path == LINK_PATH:
            self._payment_links()
            return
        if path == PAYMENT_PATH:
            self._payments()
            return
        if path == DETAILS_PATH:
            self._payment_details()
            return
        refund = REFUND_PATH.match(path)
        if refund:
            self._refund(refund.group(1))
            return
        checkout = CHECKOUT_PATH.match(path)
        if checkout:
            link_id, action = checkout.groups()
            if action in ("pay", "challenge"):
                self._checkout_action(link_id, action)
                return
        self._json(404, api_error(404, "000", "not found"))

    # -- shopper checkout page ------------------------------------------

    def _checkout_page(self, link_id: str) -> None:
        with self.simulator.lock:
            link = copy.deepcopy(self.simulator.links.get(link_id))
        if link is None:
            self._html(404, "<h1>Unknown payment link</h1>")
            return
        amount = link["amount"]
        status = link.get("status", "active")
        if status in ("completed", "refused"):
            payment = self.simulator.payments.get(link.get("paymentPspReference", ""), {})
            self._html(
                200,
                f"""<!doctype html><html><body>
<h1>Payment {status}</h1>
<p>Reference: {link['reference']}</p>
<p>Amount: {amount['value']} {amount['currency']}</p>
<p>PSP reference: {payment.get('pspReference', 'n/a')}</p>
<p><a href="{self.simulator.public_base(self)}/checkout/{link_id}">Refresh</a></p>
</body></html>""",
            )
            return
        self._html(
            200,
            f"""<!doctype html><html><body>
<h1>Apex setup watch checkout</h1>
<p>Reference: {link['reference']}</p>
<p>Amount: {amount['value'] / 100:.2f} {amount['currency']}</p>
<form method="post" action="/checkout/{link_id}/pay">
  <button name="result" value="Authorised">Pay</button>
  <button name="result" value="Refused">Simulate refusal</button>
  <button name="result" value="IdentifyShopper">Pay with 3DS action</button>
</form>
</body></html>""",
        )

    def _checkout_action(self, link_id: str, action: str) -> None:
        body = self._read_body(MAX_API_BODY)
        if body is None:
            return
        fields = urllib.parse.parse_qs(body.decode("utf-8", "replace"))
        with self.simulator.lock:
            link = copy.deepcopy(self.simulator.links.get(link_id))
        if link is None:
            self._html(404, "<h1>Unknown payment link</h1>")
            return
        pending = link.get("pendingResult")
        if action == "challenge" or pending:
            outcome = self.simulator.finish_pending_link(link_id, "Authorised")
            if "error" in outcome:
                self._html(outcome.get("status", 400), f"<h1>{outcome['error']}</h1>")
                return
            payment = outcome["payment"]
            self._html(
                200,
                f"""<!doctype html><html><body>
<h1>Payment authorised after 3DS</h1>
<p>PSP reference: {payment.get('pspReference', 'n/a')}</p>
</body></html>""",
            )
            return
        result = (fields.get("result") or ["Authorised"])[0]
        outcome = self.simulator.complete_link(link_id, result, deliver=True)
        if "error" in outcome:
            self._html(outcome.get("status", 400), f"<h1>{outcome['error']}</h1>")
            return
        if outcome.get("action"):
            payment = outcome["payment"]
            self._html(
                200,
                f"""<!doctype html><html><body>
<h1>3DS action required</h1>
<p>Result: {payment['resultCode']}</p>
<pre>{json.dumps(outcome['action'], indent=2)}</pre>
<form method="post" action="/checkout/{link_id}/challenge">
  <button name="details" value="challenge-complete">Complete 3DS challenge</button>
</form>
</body></html>""",
            )
            return
        payment = outcome["payment"]
        self._html(
            200,
            f"""<!doctype html><html><body>
<h1>Payment {payment['resultCode']}</h1>
<p>Reference: {payment['reference']}</p>
<p>PSP reference: {payment['pspReference']}</p>
<p>Captured: {payment['captured']} {payment['amount']['currency']}</p>
</body></html>""",
        )

    # -- Checkout API ----------------------------------------------------

    def _payment_links(self) -> None:
        if not self._authorized():
            self._request_record(401, "createPaymentLink")
            self._json(
                401,
                api_error(401, "000", "Invalid API key"),
                {"WWW-Authenticate": 'Basic realm="Adyen"'},
            )
            return
        body = self._read_body(MAX_API_BODY)
        if body is None:
            return
        fault = self._apply_fault("paymentLinks")
        if fault is not None:
            self._request_record(0, "createPaymentLink")
            return
        try:
            payload = json.loads(body)
        except json.JSONDecodeError as error:
            self._request_record(400, "createPaymentLink")
            self._json(400, api_error(400, "000", f"Malformed JSON: {error.msg}"))
            return
        with self.simulator.lock:
            self.simulator.counters["paymentLinks"] += 1
        key = self.headers.get("Idempotency-Key")
        fingerprint = canonical(payload)
        status, record = self.simulator.idempotency_record("paymentLinks", key, fingerprint)
        if status == "conflict":
            self._request_record(422, "createPaymentLink")
            self._json(
                422,
                api_error(422, "702", "Idempotency key already used for a different request"),
            )
            return
        if status == "replay":
            self._request_record(201, "createPaymentLink")
            self._json(201, record["response"], {"X-Sim-Replayed": "true"})
            return
        reference = payload.get("reference")
        amount = payload.get("amount") or {}
        currency = amount.get("currency")
        value = amount.get("value")
        if not isinstance(reference, str) or not reference or len(reference) > 80:
            self._request_record(422, "createPaymentLink")
            self._json(422, api_error(422, "000", "reference is required and must be <= 80 chars"))
            return
        if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
            self._request_record(422, "createPaymentLink")
            self._json(422, api_error(422, "000", "amount.value must be a positive integer"))
            return
        if not isinstance(currency, str) or len(currency) != 3:
            self._request_record(422, "createPaymentLink")
            self._json(422, api_error(422, "000", "amount.currency must be an ISO code"))
            return
        if payload.get("merchantAccount") != self.simulator.config["merchant_account"]:
            self._request_record(422, "createPaymentLink")
            self._json(422, api_error(422, "000", "merchantAccount does not match"))
            return
        link_id = f"PL{random_hex(24).upper()}"
        base = self.simulator.public_base(self)
        response = {
            "id": link_id,
            "url": f"{base}/checkout/{link_id}",
            "status": "active",
            "amount": {"value": value, "currency": currency},
            "reference": reference,
            "merchantAccount": self.simulator.config["merchant_account"],
            "shopperReference": payload.get("shopperReference", ""),
            "recurringProcessingModel": payload.get("recurringProcessingModel"),
            "storePaymentMethod": bool(payload.get("storePaymentMethod")),
        }
        link = {
            "id": link_id,
            "reference": reference,
            "amount": {"value": value, "currency": currency},
            "merchantAccount": self.simulator.config["merchant_account"],
            "shopperReference": payload.get("shopperReference", ""),
            "storedPaymentMethodId": "",
            "storePaymentMethod": bool(payload.get("storePaymentMethod")),
            "recurringProcessingModel": payload.get("recurringProcessingModel"),
            "status": "active",
            "createdAt": now_ms(),
        }
        if link["storePaymentMethod"]:
            link["storedPaymentMethodId"] = f"pm_sim_{random_hex(16)}"
        with self.simulator.lock:
            self.simulator.links[link_id] = link
            self.simulator.save()
        self.simulator.store_idempotency("paymentLinks", key, fingerprint, response)
        self._request_record(201, "createPaymentLink")
        self._json(201, response)

    def _payments(self) -> None:
        if not self._authorized():
            self._request_record(401, "chargeStoredPaymentMethod")
            self._json(401, api_error(401, "000", "Invalid API key"))
            return
        body = self._read_body(MAX_API_BODY)
        if body is None:
            return
        fault = self._apply_fault("payments")
        if fault is not None:
            self._request_record(0, "chargeStoredPaymentMethod")
            return
        try:
            payload = json.loads(body)
        except json.JSONDecodeError as error:
            self._request_record(400, "chargeStoredPaymentMethod")
            self._json(400, api_error(400, "000", f"Malformed JSON: {error.msg}"))
            return
        with self.simulator.lock:
            self.simulator.counters["payments"] += 1
        key = self.headers.get("Idempotency-Key")
        fingerprint = canonical(payload)
        status, record = self.simulator.idempotency_record("payments", key, fingerprint)
        if status == "conflict":
            self._request_record(422, "chargeStoredPaymentMethod")
            self._json(
                422,
                api_error(422, "702", "Idempotency key already used for a different request"),
            )
            return
        if status == "replay":
            self._request_record(200, "chargeStoredPaymentMethod")
            self._json(200, record["response"], {"X-Sim-Replayed": "true"})
            return
        amount = payload.get("amount") or {}
        value = amount.get("value")
        currency = amount.get("currency")
        reference = payload.get("reference")
        if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
            self._request_record(422, "chargeStoredPaymentMethod")
            self._json(422, api_error(422, "000", "amount.value must be a positive integer"))
            return
        if not isinstance(currency, str) or len(currency) != 3:
            self._request_record(422, "chargeStoredPaymentMethod")
            self._json(422, api_error(422, "000", "amount.currency must be an ISO code"))
            return
        if not isinstance(reference, str) or not reference:
            self._request_record(422, "chargeStoredPaymentMethod")
            self._json(422, api_error(422, "000", "reference is required"))
            return
        if payload.get("merchantAccount") != self.simulator.config["merchant_account"]:
            self._request_record(422, "chargeStoredPaymentMethod")
            self._json(422, api_error(422, "000", "merchantAccount does not match"))
            return
        stored = payload.get("storedPaymentMethodId") or ""
        if not stored and not payload.get("paymentMethod"):
            self._request_record(422, "chargeStoredPaymentMethod")
            self._json(422, api_error(422, "000", "storedPaymentMethodId is required"))
            return
        result = self.simulator.config["charge_result"]
        if result not in RESULT_CODES:
            result = "Authorised"
        action = None
        if result in ("IdentifyShopper", "ChallengeShopper", "RedirectShopper"):
            payment_data = f"paymentdata_{random_hex(24)}"
            response = {
                "pspReference": psp_reference(),
                "resultCode": result,
                "action": self.simulator.action_for(result, "pending"),
                "paymentData": payment_data,
                "merchantReference": reference,
            }
            response["action"]["paymentData"] = payment_data
            pending = {
                "paymentData": payment_data,
                "reference": reference,
                "amount": {"value": value, "currency": currency},
                "storedPaymentMethodId": stored,
                "shopperReference": payload.get("shopperReference", ""),
                "pspReference": response["pspReference"],
                "resultCode": result,
            }
            with self.simulator.lock:
                self.simulator.idempotency.setdefault("pending3ds", {})[payment_data] = pending
                self.simulator.save()
            self.simulator.store_idempotency("payments", key, fingerprint, response)
            self._request_record(200, "chargeStoredPaymentMethod")
            self._json(200, response)
            return
        payment = self.simulator.create_payment_record(
            reference,
            value,
            currency,
            result,
            stored,
            payload.get("shopperReference", ""),
        )
        response = {
            "pspReference": payment["pspReference"],
            "resultCode": result,
            "merchantReference": reference,
            "amount": payment["amount"],
        }
        if result not in ("Authorised", "Received"):
            response["refusalReason"] = result
        if stored:
            response.setdefault("additionalData", {})["storedPaymentMethodId"] = stored
        if action:
            response["action"] = action
        # Claim the idempotency key before the webhook side effects: if any
        # later step fails, the provider's retry must see this charge, never
        # create a second one.
        self.simulator.store_idempotency("payments", key, fingerprint, response)
        if self.simulator.config.get("auto_webhooks") and result in (
            "Authorised",
            "Received",
            "Refused",
            "Error",
            "Cancelled",
        ):
            event = self.simulator.record_event(
                "AUTHORISATION",
                payment["pspReference"],
                "",
                reference,
                value,
                currency,
                result in ("Authorised", "Received"),
                stored,
                deliver=True,
            )
            payment["events"].append(event["id"])
            self.simulator.save()
        fault = self.simulator.take_fault("payments_after_commit")
        if fault is not None and fault.get("mode") == "timeout":
            time.sleep(float(fault.get("timeout_seconds", 31)))
            self._drop()
            return
        self._request_record(200, "chargeStoredPaymentMethod")
        self._json(200, response)

    def _payment_details(self) -> None:
        if not self._authorized():
            self._request_record(401, "paymentDetails")
            self._json(401, api_error(401, "000", "Invalid API key"))
            return
        body = self._read_body(MAX_API_BODY)
        if body is None:
            return
        fault = self._apply_fault("details")
        if fault is not None:
            self._request_record(0, "paymentDetails")
            return
        try:
            payload = json.loads(body)
        except json.JSONDecodeError as error:
            self._request_record(400, "paymentDetails")
            self._json(400, api_error(400, "000", f"Malformed JSON: {error.msg}"))
            return
        payment_data = payload.get("paymentData") or payload.get("details", {}).get("paymentData")
        with self.simulator.lock:
            pending = copy.deepcopy(
                self.simulator.idempotency.get("pending3ds", {}).get(payment_data)
            )
        if pending is None:
            self._request_record(422, "paymentDetails")
            self._json(422, api_error(422, "000", "unknown paymentData"))
            return
        result = self.simulator.config.get("details_result", "Authorised")
        if result not in ("Authorised", "Refused"):
            result = "Authorised"
        payment = self.simulator.create_payment_record(
            pending["reference"],
            pending["amount"]["value"],
            pending["amount"]["currency"],
            result,
            pending["storedPaymentMethodId"],
            pending["shopperReference"],
            psp=pending.get("pspReference"),
        )
        event = self.simulator.record_event(
            "AUTHORISATION",
            payment["pspReference"],
            "",
            pending["reference"],
            pending["amount"]["value"],
            pending["amount"]["currency"],
            result == "Authorised",
            pending["storedPaymentMethodId"],
            deliver=True,
        )
        payment["events"].append(event["id"])
        with self.simulator.lock:
            self.simulator.idempotency.get("pending3ds", {}).pop(payment_data, None)
            self.simulator.save()
        self._request_record(200, "paymentDetails")
        self._json(
            200,
            {
                "pspReference": payment["pspReference"],
                "resultCode": result,
                "merchantReference": pending["reference"],
            },
        )

    def _refund(self, psp: str) -> None:
        if not self._authorized():
            self._request_record(401, "refund")
            self._json(401, api_error(401, "000", "Invalid API key"))
            return
        body = self._read_body(MAX_API_BODY)
        if body is None:
            return
        fault = self._apply_fault("refunds")
        if fault is not None:
            self._request_record(0, "refund")
            return
        try:
            payload = json.loads(body)
        except json.JSONDecodeError as error:
            self._request_record(400, "refund")
            self._json(400, api_error(400, "000", f"Malformed JSON: {error.msg}"))
            return
        key = self.headers.get("Idempotency-Key")
        fingerprint = f"{psp}:{canonical(payload)}"
        status, record = self.simulator.idempotency_record("refunds", key, fingerprint)
        if status == "conflict":
            self._request_record(422, "refund")
            self._json(422, api_error(422, "702", "Idempotency key already used"))
            return
        if status == "replay":
            self._request_record(201, "refund")
            self._json(201, record["response"], {"X-Sim-Replayed": "true"})
            return
        amount = payload.get("amount")
        amount_value = amount.get("value") if isinstance(amount, dict) else None
        outcome = self.simulator.refund_payment(
            psp, amount_value, payload.get("reference", f"refund:{psp}"), deliver=True
        )
        if "error" in outcome:
            self._request_record(outcome.get("status", 422), "refund")
            self._json(
                outcome.get("status", 422),
                api_error(
                    outcome.get("status", 422),
                    "000",
                    outcome["error"],
                ),
            )
            return
        response = outcome["response"]
        self.simulator.store_idempotency("refunds", key, fingerprint, response)
        self._request_record(201, "refund")
        self._json(201, response)

    # -- control plane ---------------------------------------------------

    def _state_view(self) -> dict[str, Any]:
        with self.simulator.lock:
            links = [copy.deepcopy(link) for link in self.simulator.links.values()]
            payments = [copy.deepcopy(payment) for payment in self.simulator.payments.values()]
            events = [
                {
                    "id": event["id"],
                    "eventCode": event["eventCode"],
                    "pspReference": event["pspReference"],
                    "originalReference": event["originalReference"],
                    "merchantReference": event["merchantReference"],
                    "amount": event["amount"],
                    "success": event["success"],
                    "storedPaymentMethodId": event.get("storedPaymentMethodId", ""),
                    "deliveries": len(event["deliveries"]),
                    "lastDelivery": event["deliveries"][-1] if event["deliveries"] else None,
                }
                for event in self.simulator.events.values()
            ]
            return {
                "config": {
                    "merchantAccount": self.simulator.config["merchant_account"],
                    "webhookUrl": self.simulator.config["webhook_url"],
                    "chargeResult": self.simulator.config["charge_result"],
                    "refundResult": self.simulator.config["refund_result"],
                    "hmacKeyConfigured": bool(self.simulator.hmac_key),
                },
                "counters": copy.deepcopy(self.simulator.counters),
                "faults": copy.deepcopy(self.simulator.faults),
                "links": links,
                "payments": payments,
                "events": events,
                "requests": copy.deepcopy(self.simulator.requests[-100:]),
            }

    def _handle_control(self, path: str) -> None:
        if not self._control_authorized():
            self._json(401, api_error(401, "000", "control token required"))
            return
        body = self._read_body(MAX_CONTROL_BODY)
        if body is None:
            return
        payload: dict[str, Any] = {}
        if body:
            try:
                payload = json.loads(body)
            except json.JSONDecodeError as error:
                self._json(400, api_error(400, "000", f"Malformed control JSON: {error.msg}"))
                return
        if path == "/_control/reset":
            with self.simulator.lock:
                self.simulator.links.clear()
                self.simulator.payments.clear()
                self.simulator.events.clear()
                self.simulator.faults.clear()
                self.simulator.requests.clear()
                self.simulator.idempotency = {
                    "paymentLinks": {},
                    "payments": {},
                    "refunds": {},
                }
                for name in self.simulator.counters:
                    self.simulator.counters[name] = 0
                self.simulator.save()
            self._json(200, {"reset": True})
            return
        if path == "/_control/config":
            with self.simulator.lock:
                for name in (
                    "charge_result",
                    "refund_result",
                    "link_default_result",
                    "details_result",
                    "auto_webhooks",
                    "webhook_url",
                ):
                    if name in payload:
                        self.simulator.config[name] = payload[name]
                self.simulator.save()
            self._json(200, {"config": self.simulator.config})
            return
        if path == "/_control/faults":
            faults = payload.get("faults")
            if faults is None:
                fault = dict(payload)
                fault.setdefault("endpoint", "*")
                if "remaining" not in fault and "count" in fault:
                    fault["remaining"] = int(fault.pop("count"))
                fault.setdefault("remaining", 1)
                faults = [fault]
            with self.simulator.lock:
                for fault in faults:
                    fault = dict(fault)
                    fault.setdefault("endpoint", "*")
                    if "remaining" not in fault and "count" in fault:
                        fault["remaining"] = int(fault.pop("count"))
                    fault.setdefault("remaining", 1)
                    fault.setdefault("mode", "status")
                    self.simulator.faults.append(fault)
                self.simulator.save()
            self._json(200, {"faults": self.simulator.faults})
            return
        if path == "/_control/faults/clear":
            with self.simulator.lock:
                self.simulator.faults.clear()
                self.simulator.save()
            self._json(200, {"faults": []})
            return
        if path == "/_control/events/replay":
            event_code = payload.get("eventCode")
            psp = payload.get("pspReference")
            count = int(payload.get("count", 1))
            with self.simulator.lock:
                matches = [
                    event["id"]
                    for event in self.simulator.events.values()
                    if (event_code is None or event["eventCode"] == event_code)
                    and (psp is None or event["pspReference"] == psp)
                ]
            results = [
                self.simulator.deliver_event(
                    event_id,
                    count=count,
                    corrupt_hmac=bool(payload.get("corrupt_hmac", False)),
                    mutate_amount=payload.get("mutate_amount"),
                )
                for event_id in matches
            ]
            self._json(200, {"replayed": results})
            return
        if path == "/_control/sign":
            # Re-sign an arbitrary item for harness convenience; the eight
            # escaped fields are exactly the documented scheme.
            item = payload.get("item") or payload
            signature = sign_item(self.simulator.hmac_key, item)
            self._json(200, {"hmacSignature": signature, "signedMessage": signed_message(item)})
            return
        deliver = CONTROL_EVENT_DELIVER.match(path)
        if deliver:
            event_id = deliver.group(1)
            with self.simulator.lock:
                exists = event_id in self.simulator.events
            if not exists:
                self._json(404, api_error(404, "000", "unknown event"))
                return
            result = self.simulator.deliver_event(
                event_id,
                count=int(payload.get("count", 1)),
                corrupt_hmac=bool(payload.get("corrupt_hmac", False)),
                mutate_amount=payload.get("mutate_amount"),
                delay_ms=int(payload.get("delay_ms", 0)),
            )
            self._json(200, result)
            return
        complete = CONTROL_LINK_COMPLETE.match(path)
        if complete:
            link_id = complete.group(1)
            result = str(payload.get("result", self.simulator.config["link_default_result"]))
            outcome = self.simulator.complete_link(
                link_id, result, deliver=bool(payload.get("deliver", True))
            )
            if "error" in outcome:
                self._json(outcome.get("status", 400), api_error(outcome.get("status", 400), "000", outcome["error"]))
                return
            self._json(200, outcome)
            return
        action = CONTROL_PAY_ACTION.match(path)
        if action:
            psp, kind = action.groups()
            outcome = self.simulator.transition(
                psp,
                kind,
                amount_value=payload.get("amount"),
                deliver=bool(payload.get("deliver", True)),
            )
            if "error" in outcome:
                self._json(outcome.get("status", 400), api_error(outcome.get("status", 400), "000", outcome["error"]))
                return
            self._json(200, outcome)
            return
        if path == "/_control/deliver":
            body_payload = payload.get("body")
            if not isinstance(body_payload, dict):
                self._json(400, api_error(400, "000", "body must be an envelope object"))
                return
            self._json(
                200,
                self.simulator.deliver_raw(
                    body_payload,
                    count=int(payload.get("count", 1)),
                    corrupt_hmac=bool(payload.get("corrupt_hmac", False)),
                ),
            )
            return
        self._json(404, api_error(404, "000", "unknown control endpoint"))


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def default_hmac_hex() -> str:
    return b"adyen_sim_hmac_key".hex()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Stateful Adyen Checkout simulator")
    parser.add_argument("--host", default=os.environ.get("ADYEN_SIM_HOST", "127.0.0.1"))
    parser.add_argument("--port", type=int, default=int(os.environ.get("ADYEN_SIM_PORT", "8799")))
    parser.add_argument(
        "--api-key",
        default=os.environ.get("APEX_ADYEN_API_KEY", "adyen_sim_api_key"),
    )
    parser.add_argument(
        "--merchant-account",
        default=os.environ.get("APEX_ADYEN_MERCHANT_ACCOUNT", "ApexMerchant"),
    )
    parser.add_argument(
        "--hmac-key-hex",
        default=os.environ.get("APEX_ADYEN_HMAC_KEY", default_hmac_hex()),
    )
    parser.add_argument(
        "--webhook-url",
        default=os.environ.get("APEX_ADYEN_WEBHOOK_URL"),
        help="product webhook endpoint; empty disables automatic delivery",
    )
    parser.add_argument(
        "--public-base",
        default=os.environ.get("ADYEN_SIM_PUBLIC_BASE"),
        help="base URL written into payment-link URLs (default: request Host)",
    )
    parser.add_argument(
        "--charge-result",
        default=os.environ.get("ADYEN_SIM_CHARGE_RESULT", "Authorised"),
    )
    parser.add_argument(
        "--link-default-result",
        default=os.environ.get("ADYEN_SIM_LINK_RESULT", "Authorised"),
    )
    parser.add_argument("--details-result", default="Authorised")
    parser.add_argument("--auto-webhooks", dest="auto_webhooks", action="store_true", default=True)
    parser.add_argument("--no-auto-webhooks", dest="auto_webhooks", action="store_false")
    parser.add_argument("--state-file", default=os.environ.get("ADYEN_SIM_STATE_FILE"))
    parser.add_argument("--control-token", default=os.environ.get("ADYEN_SIM_CONTROL_TOKEN"))
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        bytes.fromhex(args.hmac_key_hex)
    except ValueError as error:
        print(f"--hmac-key-hex is not hex: {error}", file=sys.stderr)
        return 2
    state = SimulatorState(
        {
            "api_key": args.api_key,
            "merchant_account": args.merchant_account,
            "hmac_key_hex": args.hmac_key_hex,
            "webhook_url": args.webhook_url,
            "public_base": args.public_base,
            "auto_webhooks": args.auto_webhooks,
            "link_default_result": args.link_default_result,
            "charge_result": args.charge_result,
            "control_token": args.control_token,
        },
        args.state_file,
    )
    Handler.simulator = state
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.daemon_threads = True
    print(
        json.dumps(
            {
                "event": "adyen-sim-listening",
                "url": f"http://{args.host}:{args.port}",
                "merchantAccount": args.merchant_account,
                "webhookUrl": args.webhook_url,
                "apiKey": "[configured]",
                "hmacKey": "[configured]",
            }
        ),
        flush=True,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
