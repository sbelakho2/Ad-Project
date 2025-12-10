"""Download network enrichment data with reproducible manifests."""

from __future__ import annotations

import datetime as _dt
import hashlib
import json
import os
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Callable, Dict, Iterable, List, Mapping, MutableMapping, Optional, Sequence

import requests

ISO_FMT = "%Y-%m-%dT%H:%M:%SZ"
BASE_VERSION = "v1"
DEFAULT_SOURCE_NAMES = ("tor", "cloud", "ripe", "vpn")


@dataclass(slots=True)
class ManifestEntry:
    name: str
    file: str
    url: str
    sha256: str
    license: str
    content_type: str
    fetched_at: str
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(slots=True)
class EnrichmentRunResult:
    source: str
    run_date: str
    manifest_path: Path
    entries: Sequence[ManifestEntry]


@dataclass(slots=True)
class FetchContext:
    force: bool = False
    include_vpn_lists: bool = False
    resources: Optional[Sequence[str]] = None
    extra_metadata: MutableMapping[str, object] = field(default_factory=dict)


@dataclass(slots=True)
class EnrichmentSourceConfig:
    name: str
    description: str
    fetch_fn: Callable[[Path, requests.Session, FetchContext], Sequence[ManifestEntry]]
    optional: bool = False


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()


def _write_manifest(manifest_path: Path, payload: Mapping[str, object]) -> None:
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _read_json(path: Path) -> MutableMapping[str, object]:
    if path.exists():
        return json.loads(path.read_text(encoding="utf-8"))
    return {}


def _choose_date(date_override: Optional[str] = None) -> str:
    if date_override:
        return date_override
    return _dt.datetime.now(_dt.timezone.utc).strftime("%Y-%m-%d")


def _now_iso() -> str:
    return _dt.datetime.now(_dt.timezone.utc).strftime(ISO_FMT)


def _download_if_needed(
    session: requests.Session,
    url: str,
    destination: Path,
    *,
    force: bool = False,
    timeout: int = 60,
) -> Path:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists() and not force:
        return destination

    response = session.get(url, timeout=timeout)
    response.raise_for_status()
    data = response.content
    destination.write_bytes(data)
    return destination


def _tor_fetcher(target_dir: Path, session: requests.Session, ctx: FetchContext) -> Sequence[ManifestEntry]:
    entries: List[ManifestEntry] = []
    fetched_at = _now_iso()

    tor_sources = [
        {
            "name": "torbulkexitlist",
            "url": "https://check.torproject.org/torbulkexitlist",
            "filename": "torbulkexitlist.txt",
            "content_type": "text/plain",
            "license": "Tor Project License (https://check.torproject.org/api/terms)"
        },
        {
            "name": "onionoo-details",
            "url": "https://onionoo.torproject.org/details?type=relay&flag=Exit",
            "filename": "onionoo.json",
            "content_type": "application/json",
            "license": "CC-BY-3.0 (Tor Project Onionoo API terms)"
        },
    ]

    for spec in tor_sources:
        dest = target_dir / spec["filename"]
        path = _download_if_needed(session, spec["url"], dest, force=ctx.force)
        sha = _sha256(path)
        metadata: Dict[str, object] = {}
        if spec["content_type"] == "application/json":
            try:
                payload = json.loads(path.read_text(encoding="utf-8"))
                metadata["relayCount"] = len(payload.get("relays", []))
            except json.JSONDecodeError:
                metadata["relayCount"] = 0
        entries.append(
            ManifestEntry(
                name=spec["name"],
                file=path.name,
                url=spec["url"],
                sha256=sha,
                license=spec["license"],
                content_type=spec["content_type"],
                fetched_at=fetched_at,
                metadata=metadata,
            )
        )

    return entries


def _cloud_fetcher(target_dir: Path, session: requests.Session, ctx: FetchContext) -> Sequence[ManifestEntry]:
    entries: List[ManifestEntry] = []
    fetched_at = _now_iso()
    cloud_specs = [
        {
            "name": "aws-ip-ranges",
            "url": "https://ip-ranges.amazonaws.com/ip-ranges.json",
            "filename": "aws-ip-ranges.json",
            "license": "Amazon IP Ranges Terms",
        },
        {
            "name": "gcp-cloud-ranges",
            "url": "https://www.gstatic.com/ipranges/cloud.json",
            "filename": "gcp-cloud.json",
            "license": "Google Cloud IP Ranges Terms",
        },
        # Azure link is frequently broken/rotating. Disabling for stability.
        # {
        #     "name": "azure-service-tags",
        #     "url": "https://download.microsoft.com/download/2/1/5/2158C934-979C-4B40-8C4D-9BEF7F24863F/ServiceTags_Public.json",
        #     "filename": "azure-service-tags.json",
        #     "license": "Microsoft Azure Service Tags Terms",
        # },
    ]

    for spec in cloud_specs:
        dest = target_dir / spec["filename"]
        path = _download_if_needed(session, spec["url"], dest, force=ctx.force, timeout=120)
        sha = _sha256(path)
        metadata: Dict[str, object] = {}
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
            if "prefixes" in payload:
                metadata["ipv4Prefixes"] = len(payload.get("prefixes", []))
            if "ipv6_prefixes" in payload:
                metadata["ipv6Prefixes"] = len(payload.get("ipv6_prefixes", []))
            if "values" in payload:
                metadata["serviceTagCount"] = len(payload.get("values", []))
        except json.JSONDecodeError:
            pass

        entries.append(
            ManifestEntry(
                name=spec["name"],
                file=path.name,
                url=spec["url"],
                sha256=sha,
                license=spec["license"],
                content_type="application/json",
                fetched_at=fetched_at,
                metadata=metadata,
            )
        )

    return entries


def _ripe_fetcher(target_dir: Path, session: requests.Session, ctx: FetchContext) -> Sequence[ManifestEntry]:
    entries: List[ManifestEntry] = []
    fetched_at = _now_iso()
    resources = ctx.resources or os.getenv("RIPE_RESOURCES", "AS15169,AS16509,AS8075").split(",")
    normalized = [r.strip().upper() for r in resources if r.strip()]

    for resource in normalized:
        url = f"https://stat.ripe.net/data/announced-prefixes/data.json?resource={resource}"
        filename = f"{resource.replace('/', '_').lower()}.json"
        dest = target_dir / filename
        path = _download_if_needed(session, url, dest, force=ctx.force)
        sha = _sha256(path)
        metadata: Dict[str, object] = {"resource": resource}
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
            metadata["prefixCount"] = len(payload.get("data", {}).get("prefixes", []))
        except json.JSONDecodeError:
            metadata["prefixCount"] = 0

        entries.append(
            ManifestEntry(
                name=f"ripe-{resource.lower()}",
                file=path.name,
                url=url,
                sha256=sha,
                license="RIPE NCC Creative Commons Attribution 4.0",
                content_type="application/json",
                fetched_at=fetched_at,
                metadata=metadata,
            )
        )

    return entries


def _vpn_fetcher(target_dir: Path, session: requests.Session, ctx: FetchContext) -> Sequence[ManifestEntry]:
    if not ctx.include_vpn_lists:
        return []

    entries: List[ManifestEntry] = []
    fetched_at = _now_iso()
    vpn_sources = [
        # {
        #     "name": "x4bnet-vpn-hosts",
        #     "url": "https://raw.githubusercontent.com/X4BNet/lists_vpn/main/output/vpn_hosts.txt",
        #     "filename": "x4bnet_vpn_hosts.txt",
        #     "license": "Creative Commons Attribution 4.0 International",
        # },
        {
            "name": "vpn-gate",
            "url": "https://raw.githubusercontent.com/ejrv/VPNs/master/vpn-ipv4.txt",
            "filename": "vpn_gate_ipv4.txt",
            "license": "Creative Commons Attribution 4.0 International",
        },
    ]

    for spec in vpn_sources:
        dest = target_dir / spec["filename"]
        path = _download_if_needed(session, spec["url"], dest, force=ctx.force)
        sha = _sha256(path)
        entries.append(
            ManifestEntry(
                name=spec["name"],
                file=path.name,
                url=spec["url"],
                sha256=sha,
                license=spec["license"],
                content_type="text/plain",
                fetched_at=fetched_at,
                metadata={},
            )
        )

    return entries


def load_available_sources(include_optional: bool = True) -> Dict[str, EnrichmentSourceConfig]:
    configs: Dict[str, EnrichmentSourceConfig] = {
        "tor": EnrichmentSourceConfig("tor", "Tor exit relays", _tor_fetcher),
        "cloud": EnrichmentSourceConfig("cloud", "Cloud provider IPv4/IPv6 ranges", _cloud_fetcher),
        "ripe": EnrichmentSourceConfig("ripe", "RIPEstat announced prefixes", _ripe_fetcher),
        "vpn": EnrichmentSourceConfig("vpn", "Community VPN indicator lists", _vpn_fetcher, optional=True),
    }
    if not include_optional:
        return {k: v for k, v in configs.items() if not v.optional}
    return configs


def fetch_sources(
    base_output: Path,
    *,
    date_override: Optional[str] = None,
    sources: Optional[Sequence[str]] = None,
    include_vpn_lists: Optional[bool] = None,
    force: bool = False,
    session: Optional[requests.Session] = None,
) -> Dict[str, EnrichmentRunResult]:
    """Download the requested enrichment sources.

    Parameters
    ----------
    base_output:
        Root directory for enrichment data (typically data/enrichment/v1).
    date_override:
        Optional YYYY-MM-DD string to control the output folder.
    sources:
        Subset of sources to download. Defaults to all known sources.
    include_vpn_lists:
        Whether to download VPN lists. Overrides environment variable INCLUDE_PERMISSIVE_VPN_LISTS.
    force:
        Force re-download even if files already exist.
    session:
        Optional preconfigured :class:`requests.Session` for testing.
    """

    resolved_date = _choose_date(date_override)
    options = load_available_sources(include_optional=True)
    selected_names = [s.lower() for s in (sources or options.keys())]
    selected: Dict[str, EnrichmentSourceConfig] = {}
    for name in selected_names:
        if name not in options:
            raise ValueError(f"Unknown enrichment source: {name}")
        selected[name] = options[name]

    if include_vpn_lists is None:
        include_vpn_lists = os.getenv("INCLUDE_PERMISSIVE_VPN_LISTS", "false").lower() == "true"

    base_output = base_output / BASE_VERSION
    results: Dict[str, EnrichmentRunResult] = {}
    session = session or requests.Session()
    session.headers.setdefault("User-Agent", "apexml-fetch-enrichment/1.0")

    for name, config in selected.items():
        if name == "vpn" and not include_vpn_lists:
            # Explicit opt-in required so we skip creating manifests or directories when disabled
            continue
        ctx = FetchContext(force=force, include_vpn_lists=include_vpn_lists, resources=None)
        target_dir = base_output / name / resolved_date
        entries = config.fetch_fn(target_dir, session, ctx)
        manifest_payload = {
            "source": name,
            "version": BASE_VERSION,
            "runDate": resolved_date,
            "generatedAt": _now_iso(),
            "entries": [asdict(entry) for entry in entries],
        }
        manifest_path = target_dir / "manifest.json"
        _write_manifest(manifest_path, manifest_payload)
        results[name] = EnrichmentRunResult(
            source=name,
            run_date=resolved_date,
            manifest_path=manifest_path,
            entries=entries,
        )

    _update_root_manifest(base_output, results)
    return results


def _update_root_manifest(base_output: Path, results: Mapping[str, EnrichmentRunResult]) -> None:
    manifest_path = base_output / "manifest.json"
    payload = _read_json(manifest_path)
    payload.setdefault("version", BASE_VERSION)
    payload["updatedAt"] = _now_iso()
    history = payload.setdefault("latestRuns", {})
    for name, result in results.items():
        history[name] = {
            "runDate": result.run_date,
            "manifest": str(result.manifest_path.relative_to(base_output.parent)),
            "entryCount": len(result.entries),
        }
    _write_manifest(manifest_path, payload)


__all__ = [
    "DEFAULT_SOURCE_NAMES",
    "EnrichmentRunResult",
    "EnrichmentSourceConfig",
    "FetchContext",
    "ManifestEntry",
    "fetch_sources",
    "load_available_sources",
]
