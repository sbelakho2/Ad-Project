#!/usr/bin/env python3
"""Generate schema-valid synthetic SDK analytics events.

Outputs JSON with per-SDK-key batches for:
- /api/v1/analytics/events/impressions
- /api/v1/analytics/events/clicks
- /api/v1/analytics/events/revenue

No external deps.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import sys
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Dict, List, Tuple


def now_iso() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def u() -> str:
    return str(uuid.uuid4())


def choose_weighted(rng: random.Random, items: List[Tuple[str, float]]) -> str:
    total = sum(w for _, w in items)
    x = rng.random() * total
    acc = 0.0
    for value, w in items:
        acc += w
        if x <= acc:
            return value
    return items[-1][0]


def money_usd(rng: random.Random, lo: float, hi: float) -> float:
    return round(rng.uniform(lo, hi), 6)


@dataclass(frozen=True)
class AppContext:
    platform: str
    app_id: str
    sdk_key: str


def env(name: str, default: str = "") -> str:
    return (os.environ.get(name) or default).strip()


def collect_apps() -> List[AppContext]:
    default_key = env("SDK_KEY")

    def app(platform: str, app_var: str, key_var: str) -> AppContext | None:
        app_id = env(app_var)
        if not app_id:
            return None
        sdk_key = env(key_var) or default_key
        if not sdk_key:
            raise SystemExit(f"Missing SDK key for {platform}: set {key_var} or SDK_KEY")
        return AppContext(platform=platform, app_id=app_id, sdk_key=sdk_key)

    apps: List[AppContext] = []
    for platform, app_var, key_var in [
        ("android", "APP_ID_ANDROID", "SDK_KEY_ANDROID"),
        ("android", "APP_ID_ANDROID_2", "SDK_KEY_ANDROID_2"),
        ("ios", "APP_ID_IOS", "SDK_KEY_IOS"),
        ("ios", "APP_ID_IOS_2", "SDK_KEY_IOS_2"),
        ("web", "APP_ID_WEB", "SDK_KEY_WEB"),
        ("unity", "APP_ID_UNITY", "SDK_KEY_UNITY"),
        ("ctv", "APP_ID_CTV", "SDK_KEY_CTV"),
    ]:
        item = app(platform, app_var, key_var)
        if item:
            apps.append(item)

    if not apps:
        raise SystemExit("No apps configured: set APP_ID_ANDROID/IOS/WEB/etc")

    return apps


def build_impression(rng: random.Random, publisher_id: str, app: AppContext) -> Dict[str, Any]:
    networks = ["admob", "unity", "applovin", "ironsource", "vungle"]
    ad_format = choose_weighted(
        rng,
        [
            ("banner", 0.45),
            ("interstitial", 0.18),
            ("rewarded", 0.18),
            ("native", 0.09),
            ("video", 0.10),
        ],
    )
    adapter_name = rng.choice(networks)

    if app.platform == "web":
        os_name = "web"
        device_type = rng.choice(["desktop", "desktop", "desktop", "tablet"])
        os_version = ""
    elif app.platform == "ctv":
        os_name = "tvos"
        device_type = "tv"
        os_version = f"{rng.randint(15,17)}.{rng.randint(0,5)}"
    else:
        os_name = rng.choice(["ios", "android"]) if app.platform == "unity" else app.platform
        device_type = rng.choice(["phone", "phone", "phone", "tablet"])
        if os_name == "ios":
            os_version = f"{rng.randint(15,17)}.{rng.randint(0,6)}"
        else:
            os_version = f"{rng.randint(11,15)}.{rng.randint(0,6)}"

    session_id = u()
    request_id = u()
    event_id = u()

    ecpm_usd = money_usd(rng, 0.05, 4.0)
    bid_price_usd = money_usd(rng, 0.0001, max(0.0002, ecpm_usd / 1000.0 * 1.2))
    latency_ms = rng.randint(40, 650)

    country_code = rng.choice(["US", "GB", "DE", "FR", "CA", "SE", "EE"])

    return {
        "event_id": event_id,
        "timestamp": now_iso(),
        "publisher_id": publisher_id,
        "app_id": app.app_id,
        "placement_id": u(),
        "adapter_id": u(),
        "adapter_name": adapter_name,
        "ad_unit_id": f"{app.app_id}/{ad_format}/{adapter_name}",
        "ad_format": ad_format,
        "country_code": country_code,
        "device_type": device_type,
        "os": os_name,
        "os_version": os_version,
        "app_version": f"{rng.randint(1,3)}.{rng.randint(0,12)}.{rng.randint(0,30)}",
        "sdk_version": f"1.{rng.randint(0,3)}.{rng.randint(0,30)}",
        "session_id": session_id,
        "user_id": f"u_{session_id[:8]}",
        "request_id": request_id,
        "bid_price_usd": bid_price_usd,
        "ecpm_usd": ecpm_usd,
        "latency_ms": latency_ms,
        "is_test_mode": False,
    }


def build_click(rng: random.Random, publisher_id: str, imp: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "event_id": u(),
        "timestamp": now_iso(),
        "impression_id": imp["event_id"],
        "publisher_id": publisher_id,
        "app_id": imp["app_id"],
        "placement_id": imp["placement_id"],
        "adapter_id": imp["adapter_id"],
        "adapter_name": imp["adapter_name"],
        "click_url": f"https://example.com/click?iid={imp['event_id']}",
        "country_code": imp["country_code"],
        "device_type": imp["device_type"],
        "os": imp["os"],
        "session_id": imp["session_id"],
        "user_id": imp["user_id"],
        "request_id": imp["request_id"],
        "time_to_click_ms": rng.randint(120, 4500),
        "is_verified": True,
        "is_test_mode": False,
    }


def build_revenue(rng: random.Random, publisher_id: str, imp: Dict[str, Any]) -> Dict[str, Any]:
    # Revenue per impression loosely derived from eCPM.
    ecpm = float(imp["ecpm_usd"])
    revenue_usd = max(0.0, round(ecpm / 1000.0 * rng.uniform(0.6, 1.2), 6))
    return {
        "event_id": u(),
        "timestamp": now_iso(),
        "publisher_id": publisher_id,
        "app_id": imp["app_id"],
        "placement_id": imp["placement_id"],
        "adapter_id": imp["adapter_id"],
        "adapter_name": imp["adapter_name"],
        "impression_id": imp["event_id"],
        "revenue_type": "impression",
        "revenue_usd": revenue_usd,
        "revenue_currency": "USD",
        "revenue_original": revenue_usd,
        "exchange_rate": 1.0,
        "ecpm_usd": ecpm,
        "country_code": imp["country_code"],
        "ad_format": imp["ad_format"],
        "os": imp["os"],
        "is_test_mode": False,
        "reconciliation_status": "matched",
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=25)
    ap.add_argument("--seed", type=str, default="")
    args = ap.parse_args()

    publisher_id = env("PUBLISHER_ID")
    if not publisher_id:
        print("Missing PUBLISHER_ID", file=sys.stderr)
        return 2

    apps = collect_apps()

    seed = args.seed or env("SLO_SEED") or str(uuid.uuid4())
    rng = random.Random(seed)

    # Guarantee at least one impression per app if count allows.
    count = max(1, int(args.count))
    chosen: List[AppContext] = []
    if count >= len(apps):
        chosen.extend(apps)
        remaining = count - len(apps)
    else:
        remaining = count

    # Weight platform distribution to look like production-ish traffic.
    weights: List[Tuple[AppContext, float]] = []
    for a in apps:
        w = 1.0
        if a.platform == "android":
            w = 2.2
        elif a.platform == "ios":
            w = 2.0
        elif a.platform == "web":
            w = 0.8
        elif a.platform == "unity":
            w = 0.6
        elif a.platform == "ctv":
            w = 0.4
        weights.append((a, w))

    def pick_app() -> AppContext:
        total = sum(w for _, w in weights)
        x = rng.random() * total
        acc = 0.0
        for a, w in weights:
            acc += w
            if x <= acc:
                return a
        return weights[-1][0]

    for _ in range(remaining):
        chosen.append(pick_app())

    impressions_by_key: Dict[str, List[Dict[str, Any]]] = {}
    clicks_by_key: Dict[str, List[Dict[str, Any]]] = {}
    revenue_by_key: Dict[str, List[Dict[str, Any]]] = {}

    for app in chosen:
        imp = build_impression(rng, publisher_id, app)
        impressions_by_key.setdefault(app.sdk_key, []).append(imp)

        # Click rate ~2-6%
        if rng.random() < rng.uniform(0.02, 0.06):
            clicks_by_key.setdefault(app.sdk_key, []).append(build_click(rng, publisher_id, imp))

        # Revenue for most impressions
        if rng.random() < rng.uniform(0.7, 0.95):
            revenue_by_key.setdefault(app.sdk_key, []).append(build_revenue(rng, publisher_id, imp))

    payload = {
        "seed": seed,
        "impressions": [
            {"sdk_key": k, "events": v} for k, v in impressions_by_key.items() if v
        ],
        "clicks": [
            {"sdk_key": k, "events": v} for k, v in clicks_by_key.items() if v
        ],
        "revenue": [
            {"sdk_key": k, "events": v} for k, v in revenue_by_key.items() if v
        ],
    }

    json.dump(payload, sys.stdout, separators=(",", ":"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
