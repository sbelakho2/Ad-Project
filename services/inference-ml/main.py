"""ML Inference Service with authenticated model-backed scoring and metrics."""

import base64
import hashlib
import hmac
import importlib
import json
import os
import sys
import time
import warnings
from collections import defaultdict, deque
from dataclasses import dataclass
from pathlib import Path
from threading import Lock
from typing import Any, Dict, List, Optional, Tuple

import numpy as np
import onnxruntime as ort
import structlog


def _ensure_python_multipart() -> None:
    try:  # pragma: no cover - best-effort import
        import python_multipart  # type: ignore  # noqa: F401
    except ImportError:
        try:
            module = importlib.import_module("multipart")
        except ImportError:
            return
        sys.modules.setdefault("python_multipart", module)


_ensure_python_multipart()

warnings.filterwarnings(
    "ignore",
    message="Please use `import python_multipart` instead.",
    category=PendingDeprecationWarning,
    module="multipart.multipart",
)
warnings.filterwarnings(
    "ignore",
    message="Please use `import python_multipart` instead.",
    category=PendingDeprecationWarning,
    module="starlette.formparsers",
)

from fastapi import Depends, FastAPI, HTTPException, Request
from fastapi.responses import PlainTextResponse
from prometheus_client import CONTENT_TYPE_LATEST, Counter, Gauge, Histogram, generate_latest
from pydantic import BaseModel, ConfigDict, Field

logger = structlog.get_logger()

INFERENCE_REQUESTS = Counter(
    "ml_inference_requests_total",
    "Total number of inference requests",
    ["model_name", "status", "tenant"],
)

INFERENCE_LATENCY = Histogram(
    "ml_inference_duration_seconds",
    "Inference request duration in seconds",
    ["model_name", "tenant"],
    buckets=[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0],
)

INFERENCE_ERRORS = Counter(
    "ml_inference_errors_total",
    "Total number of inference errors",
    ["model_name", "error_type", "tenant"],
)

MODEL_LOAD_TIME = Gauge(
    "ml_model_load_time_seconds",
    "Time taken to load model",
    ["model_name"],
)

ACTIVE_MODELS = Gauge("ml_active_models", "Number of currently loaded models")


class FraudDetectionRequest(BaseModel):
    user_id: Optional[str] = None
    ip_address: Optional[str] = None
    user_agent: Optional[str] = None
    # Features required by ONNX model (9 dimensions)
    auction_bid_count: int = Field(0, ge=0)
    ctit_seconds: float = Field(0.0, ge=0.0)
    is_tor: float = Field(0.0, ge=0.0, le=1.0)
    is_cloud: float = Field(0.0, ge=0.0, le=1.0)
    is_vpn: float = Field(0.0, ge=0.0, le=1.0)
    is_bad_asn: float = Field(0.0, ge=0.0, le=1.0)
    is_bogon: float = Field(0.0, ge=0.0, le=1.0)
    omsdk_viewable_ratio: float = Field(0.0, ge=0.0, le=1.0)
    auction_win_ecpm: float = Field(0.0, ge=0.0)
    # Legacy fields
    impressions_24h: Optional[int] = 0
    clicks_24h: Optional[int] = 0
    app_id: Optional[str] = None
    device_info: Optional[Dict[str, Any]] = None


class CTRPredictionRequest(BaseModel):
    placement_id: str
    ad_format: str
    user_segments: List[str] = []
    time_of_day: int = Field(ge=0, le=23)
    day_of_week: int = Field(ge=0, le=6)
    historical_ctr: Optional[float] = Field(None, ge=0.0, le=1.0)


class BidOptimizationRequest(BaseModel):
    placement_id: str
    floor_cpm: float = Field(ge=0)
    predicted_ctr: float = Field(ge=0.0, le=1.0)
    competition_level: str = Field(pattern="^(low|medium|high)$")
    budget_remaining: float = Field(ge=0)


class AdapterSnapshot(BaseModel):
    adapter: str
    bid_cpm: float = Field(ge=0)
    latency_ms: float = Field(ge=0)
    metadata: Dict[str, Any] = Field(default_factory=dict)


class AuctionReplayRequest(BaseModel):
    auction_id: str
    snapshots: List[AdapterSnapshot]
    request_payload: Dict[str, Any] = Field(default_factory=dict)
    response_payload: Dict[str, Any] = Field(default_factory=dict)


class InferenceResponse(BaseModel):
    model_config = ConfigDict(protected_namespaces=())

    model_name: str
    model_version: str
    prediction: Any
    confidence: Optional[float] = None
    latency_ms: float
    transparency_receipt: Optional[Dict[str, Any]] = None


class HealthResponse(BaseModel):
    model_config = ConfigDict(protected_namespaces=())

    status: str
    models_loaded: int
    models_required_ready: bool
    uptime_seconds: float


@dataclass
class ModelHandle:
    name: str
    version: str
    session: Optional[ort.InferenceSession]
    metadata: Dict[str, Any]
    mock_only: bool = False


MODEL_CATALOG = ("fraud_detection", "ctr_prediction", "bid_optimization")
START_TIME = time.time()
MODEL_REGISTRY: Dict[str, ModelHandle] = {}
MODEL_ROOT = Path(os.getenv("MODEL_DIR", "/app/models")).expanduser()
ALLOW_MODEL_MOCKS = os.getenv("ALLOW_MODEL_MOCKS", "false").lower() in {"1", "true", "yes"}
JWT_SECRET = os.getenv("JWT_SECRET")
JWT_ALGORITHM = os.getenv("JWT_ALGORITHM", "HS256")
JWT_AUDIENCE = os.getenv("JWT_AUDIENCE")
TENANT_HEADER = os.getenv("INFERENCE_TENANT_HEADER", "X-Publisher-Id")
ALLOWED_ROLES = {
    role.strip()
    for role in os.getenv("INFERENCE_ALLOWED_ROLES", "admin,publisher,readonly").split(",")
    if role.strip()
}
RATE_LIMIT_MAX_REQUESTS = int(os.getenv("RATE_LIMIT_MAX_REQUESTS", "600"))
RATE_LIMIT_WINDOW_SECONDS = int(os.getenv("RATE_LIMIT_WINDOW_SECONDS", "60"))
TRANSPARENCY_SECRET = os.getenv("TRANSPARENCY_SECRET")

MODEL_ALIAS_MAP = {
    "fraud": "fraud_detection",
    "fraud_detection": "fraud_detection",
    "ctr": "ctr_prediction",
    "ctr_prediction": "ctr_prediction",
    "bid": "bid_optimization",
    "bid_optimization": "bid_optimization",
}

def _resolve_allowed_models() -> Tuple[str, ...]:
    tokens = [token.strip().lower() for token in os.getenv("ALLOWED_MODELS", "fraud,ctr").split(",") if token.strip()]
    resolved = []
    for token in tokens:
        canonical = MODEL_ALIAS_MAP.get(token)
        if canonical and canonical not in resolved:
            resolved.append(canonical)
    if not resolved:
        resolved = ["fraud_detection", "ctr_prediction"]
    return tuple(name for name in MODEL_CATALOG if name in set(resolved))


MODEL_NAMES = _resolve_allowed_models()

RAW_REQUIRED_MODELS = {
    MODEL_ALIAS_MAP.get(model.strip().lower(), model.strip())
    for model in os.getenv("REQUIRED_MODELS", "fraud_detection").split(",")
    if model.strip()
}
REQUIRED_MODELS = {model for model in RAW_REQUIRED_MODELS if model in set(MODEL_NAMES)}


class RateLimiter:
    def __init__(self, max_requests: int, window_seconds: int) -> None:
        self.max_requests = max_requests
        self.window_seconds = window_seconds
        self._hits: Dict[str, deque] = defaultdict(deque)
        self._lock = Lock()

    def hit(self, key: str) -> None:
        if self.max_requests <= 0:
            return

        now = time.time()
        cutoff = now - self.window_seconds

        with self._lock:
            bucket = self._hits[key]
            while bucket and bucket[0] < cutoff:
                bucket.popleft()

            if len(bucket) >= self.max_requests:
                raise HTTPException(
                    status_code=429,
                    detail="Rate limit exceeded",
                    headers={"Retry-After": str(self.window_seconds)},
                )

            bucket.append(now)

    def reset(self) -> None:
        with self._lock:
            self._hits.clear()


RATE_LIMITER = RateLimiter(RATE_LIMIT_MAX_REQUESTS, RATE_LIMIT_WINDOW_SECONDS)


class AuthContext(BaseModel):
    user_id: str
    publisher_id: str
    email: str
    role: Optional[str] = None


def _require_tenant(request: Request) -> str:
    tenant = request.headers.get(TENANT_HEADER)
    if not tenant:
        raise HTTPException(status_code=400, detail=f"Missing {TENANT_HEADER}")
    request.state.tenant_id = tenant
    return tenant


def require_transparency_secret() -> str:
    if not TRANSPARENCY_SECRET:
        raise RuntimeError("TRANSPARENCY_SECRET environment variable must be set")
    return TRANSPARENCY_SECRET


def make_transparency_receipt(tenant: str, req_payload: Dict[str, Any], rsp_payload: Dict[str, Any], secret: str) -> Dict[str, Any]:
    blob = json.dumps(
        {"tenant": tenant, "request": req_payload, "response": rsp_payload},
        separators=(",", ":"),
        sort_keys=True,
    )
    signature = hmac.new(secret.encode("utf-8"), blob.encode("utf-8"), hashlib.sha256).hexdigest()
    return {
        "hash": hashlib.sha256(blob.encode("utf-8")).hexdigest(),
        "sig": signature,
        "algo": "HMAC-SHA256",
    }


def _base64url_decode(value: str) -> bytes:
    padding = "=" * (-len(value) % 4)
    return base64.urlsafe_b64decode(value + padding)


def decode_jwt_token(token: str) -> Dict[str, Any]:
    if not JWT_SECRET:
        logger.error("JWT_SECRET not configured for inference service")
        raise HTTPException(status_code=500, detail="Authentication configuration error")

    segments = token.split(".")
    if len(segments) != 3:
        raise HTTPException(status_code=401, detail="Invalid token format")

    header_segment, payload_segment, signature_segment = segments

    try:
        header = json.loads(_base64url_decode(header_segment))
    except Exception as exc:  # noqa: BLE001
        raise HTTPException(status_code=401, detail="Invalid token header") from exc

    if header.get("alg") != JWT_ALGORITHM:
        raise HTTPException(status_code=401, detail="Unsupported JWT algorithm")

    signing_input = f"{header_segment}.{payload_segment}".encode("ascii")
    expected_signature = hmac.new(
        JWT_SECRET.encode("utf-8"),
        signing_input,
        hashlib.sha256,
    ).digest()

    try:
        provided_signature = _base64url_decode(signature_segment)
    except Exception as exc:  # noqa: BLE001
        raise HTTPException(status_code=401, detail="Invalid token signature") from exc

    if not hmac.compare_digest(expected_signature, provided_signature):
        raise HTTPException(status_code=401, detail="Invalid token signature")

    try:
        payload: Dict[str, Any] = json.loads(_base64url_decode(payload_segment))
    except Exception as exc:  # noqa: BLE001
        raise HTTPException(status_code=401, detail="Invalid token payload") from exc

    expires_at = payload.get("exp")
    if expires_at is not None and time.time() >= float(expires_at):
        raise HTTPException(status_code=401, detail="Token expired")

    if JWT_AUDIENCE:
        aud_claim = payload.get("aud")
        audiences = {aud_claim} if isinstance(aud_claim, str) else set(aud_claim or [])
        if JWT_AUDIENCE not in audiences:
            raise HTTPException(status_code=403, detail="Invalid token audience")

    return payload


def authorize_request(request: Request) -> AuthContext:
    tenant_header = _require_tenant(request)
    auth_header = request.headers.get("authorization")
    if not auth_header or not auth_header.startswith("Bearer "):
        raise HTTPException(status_code=401, detail="Missing bearer token")

    token = auth_header.split(" ", 1)[1]
    payload = decode_jwt_token(token)

    user_id = payload.get("userId")
    publisher_id = payload.get("publisherId")
    email = payload.get("email")
    role = payload.get("role", "publisher")

    if not (user_id and publisher_id and email):
        raise HTTPException(status_code=401, detail="Invalid token claims")

    if ALLOWED_ROLES and role not in ALLOWED_ROLES:
        raise HTTPException(status_code=403, detail="Role not permitted")

    if tenant_header != publisher_id:
        raise HTTPException(status_code=403, detail="Tenant mismatch")

    context = AuthContext(user_id=user_id, publisher_id=publisher_id, email=email, role=role)
    request.state.auth_context = context
    request.state.tenant_id = tenant_header

    key = f"{publisher_id}:{request.url.path}"
    RATE_LIMITER.hit(key)

    return context


def resolve_artifact_paths(model_name: str) -> Tuple[Path, Path, str]:
    prefix = model_name.upper().replace("-", "_")
    explicit_path = os.getenv(f"{prefix}_MODEL_PATH")
    version = os.getenv(f"{prefix}_MODEL_VERSION", "latest")

    if explicit_path:
        model_path = Path(explicit_path).expanduser()
    else:
        model_path = MODEL_ROOT / model_name / version / "model.onnx"

    metadata_env = os.getenv(f"{prefix}_MODEL_METADATA")
    if metadata_env:
        metadata_path = Path(metadata_env).expanduser()
    else:
        metadata_path = model_path.with_name("metadata.json")

    return model_path, metadata_path, version


def load_model_handle(model_name: str) -> ModelHandle:
    model_path, metadata_path, requested_version = resolve_artifact_paths(model_name)
    load_start = time.time()

    if not model_path.exists():
        if model_name in REQUIRED_MODELS and not ALLOW_MODEL_MOCKS:
            raise FileNotFoundError(f"Required model '{model_name}' missing at {model_path}")
        logger.warning(
            "Model artifact missing; falling back to mock inference",
            model=model_name,
            path=str(model_path),
        )
        return ModelHandle(model_name, requested_version, None, {}, mock_only=True)

    session = ort.InferenceSession(
        str(model_path.resolve()),
        providers=["CPUExecutionProvider"],
    )
    MODEL_LOAD_TIME.labels(model_name=model_name).set(time.time() - load_start)

    metadata: Dict[str, Any] = {}
    if metadata_path.exists():
        try:
            metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            logger.warning(
                "Failed to parse model metadata; continuing with defaults",
                model=model_name,
                error=str(exc),
            )

    version = metadata.get("version", requested_version)
    logger.info(
        "Loaded model",
        model=model_name,
        version=version,
        artifact=str(model_path),
    )
    return ModelHandle(name=model_name, version=version, session=session, metadata=metadata)


def refresh_model_registry() -> None:
    MODEL_REGISTRY.clear()
    for name in MODEL_NAMES:
        handle = load_model_handle(name)
        MODEL_REGISTRY[name] = handle
    ACTIVE_MODELS.set(len([h for h in MODEL_REGISTRY.values() if h.session and not h.mock_only]))


def models_ready() -> bool:
    for required in REQUIRED_MODELS:
        handle = MODEL_REGISTRY.get(required)
        if not handle or handle.mock_only or handle.session is None:
            return False
    return True


def require_model(name: str) -> ModelHandle:
    handle = MODEL_REGISTRY.get(name)
    if not handle:
        raise HTTPException(status_code=503, detail=f"Model '{name}' not loaded")
    if handle.mock_only and not ALLOW_MODEL_MOCKS:
        raise HTTPException(status_code=503, detail=f"Model '{name}' unavailable")
    return handle


def record_request(model_name: str, status: str, tenant: str, start_time: float) -> None:
    duration = time.time() - start_time
    INFERENCE_LATENCY.labels(model_name=model_name, tenant=tenant).observe(duration)
    INFERENCE_REQUESTS.labels(model_name=model_name, status=status, tenant=tenant).inc()


def run_session(handle: ModelHandle, features: np.ndarray) -> Optional[float]:
    if handle.session is None:
        return None
    try:
        input_name = handle.session.get_inputs()[0].name
        outputs = handle.session.run(None, {input_name: features.astype(np.float32)})
        flattened = np.ravel(outputs[0])
        if flattened.size == 0:
            return None
        # Assume binary classification; use last column if >1 outputs
        score = flattened[-1] if flattened.size > 1 else flattened[0]
        return float(max(0.0, min(1.0, score)))
    except Exception as exc:
        logger.warning("ONNX execution failed; falling back to heuristics", model=handle.name, error=str(exc))
        return None


def fraud_heuristic_score(req: FraudDetectionRequest) -> float:
    click_ratio = req.clicks_24h / max(1, req.impressions_24h)
    ip_penalty = 0.1 if req.ip_address.startswith(("10.", "192.168.", "172.16.")) else 0.0
    device_penalty = 0.1 if (req.device_info or {}).get("is_rooted") else 0.0
    score = 0.2 + click_ratio * 0.6 + ip_penalty + device_penalty
    return float(min(1.0, max(0.0, score)))


def ctr_heuristic(req: CTRPredictionRequest) -> float:
    base = req.historical_ctr if req.historical_ctr is not None else 0.015
    segment_bonus = 0.002 * len(req.user_segments)
    tod_modifier = abs(12 - req.time_of_day) * 0.0003
    score = base + segment_bonus - tod_modifier
    return float(min(0.5, max(0.001, score)))


def bid_heuristic(req: BidOptimizationRequest) -> float:
    competition_multiplier = {"low": 0.95, "medium": 1.05, "high": 1.2}[req.competition_level]
    ctr_influence = 1 + req.predicted_ctr * 2
    optimized = req.floor_cpm * ctr_influence * competition_multiplier
    return float(min(req.budget_remaining, max(req.floor_cpm, optimized)))


app = FastAPI(
    title="ML Inference Service",
    description="Production ML model serving for Rival Ad Platform",
    version="1.1.0",
)


@app.on_event("startup")
async def startup_event() -> None:
    if not TRANSPARENCY_SECRET:
        raise RuntimeError("TRANSPARENCY_SECRET must be configured")
    logger.info("Starting ML inference service", model_root=str(MODEL_ROOT))
    refresh_model_registry()
    logger.info("Model registry ready", models=list(MODEL_REGISTRY.keys()))


@app.middleware("http")
async def log_requests(request: Request, call_next):
    start = time.time()
    response = await call_next(request)
    duration = time.time() - start
    logger.info(
        "request_complete",
        method=request.method,
        path=request.url.path,
        status=response.status_code,
        duration=duration,
    )
    return response


@app.get("/health", response_model=HealthResponse)
async def health_check():
    return HealthResponse(
        status="ok",
        models_loaded=len(MODEL_REGISTRY),
        models_required_ready=models_ready(),
        uptime_seconds=time.time() - START_TIME,
    )


@app.get("/health/ready", response_model=HealthResponse)
async def readiness_check():
    ready = models_ready()
    if not ready:
        raise HTTPException(status_code=503, detail="Required models are not ready")
    return HealthResponse(
        status="ready",
        models_loaded=len(MODEL_REGISTRY),
        models_required_ready=True,
        uptime_seconds=time.time() - START_TIME,
    )


@app.get("/metrics")
async def metrics():
    return PlainTextResponse(generate_latest(), media_type=CONTENT_TYPE_LATEST)


def inference_wrapper(model_name: str, tenant: str, handler):
    start = time.time()
    try:
        response, status = handler()
        response.latency_ms = (time.time() - start) * 1000
        record_request(model_name, status, tenant, start)
        return response
    except HTTPException:
        record_request(model_name, "error", tenant, start)
        raise
    except Exception as exc:
        INFERENCE_ERRORS.labels(model_name=model_name, error_type="runtime", tenant=tenant).inc()
        record_request(model_name, "error", tenant, start)
        logger.error("Inference failed", model=model_name, error=str(exc))
        raise HTTPException(status_code=500, detail="Inference failed") from exc


@app.post("/predict/fraud", response_model=InferenceResponse)
async def predict_fraud(
    payload: FraudDetectionRequest,
    http_request: Request,
    auth: AuthContext = Depends(authorize_request),
):
    _ = auth
    tenant = getattr(http_request.state, "tenant_id", None) or _require_tenant(http_request)

    def handler():
        handle = require_model("fraud_detection")
        features = np.array([[
            payload.auction_bid_count,
            payload.ctit_seconds,
            payload.is_tor,
            payload.is_cloud,
            payload.is_vpn,
            payload.is_bad_asn,
            payload.is_bogon,
            payload.omsdk_viewable_ratio,
            payload.auction_win_ecpm
        ]], dtype=np.float32)
        score = run_session(handle, features)
        source = "onnx" if score is not None else "heuristic"
        fraud_score = score if score is not None else fraud_heuristic_score(payload)
        confidence = round(1 - min(0.9, fraud_score / 2), 3)
        response = InferenceResponse(
            model_name="fraud_detection",
            model_version=handle.version,
            prediction={"fraud_score": fraud_score, "is_fraud": fraud_score >= 0.5, "source": source},
            confidence=confidence,
            latency_ms=0.0,
        )
        response.transparency_receipt = make_transparency_receipt(
            tenant,
            payload.model_dump(),
            response.prediction,
            require_transparency_secret(),
        )
        status = "mock" if handle.mock_only or source == "heuristic" else "success"
        return response, status

    return inference_wrapper("fraud_detection", tenant, handler)


@app.post("/predict/ctr", response_model=InferenceResponse)
async def predict_ctr(
    payload: CTRPredictionRequest,
    http_request: Request,
    auth: AuthContext = Depends(authorize_request),
):
    _ = auth
    tenant = getattr(http_request.state, "tenant_id", None) or _require_tenant(http_request)

    def handler():
        handle = require_model("ctr_prediction")
        ctr_value = run_session(handle, np.array([[payload.time_of_day, payload.day_of_week]], dtype=np.float32))
        source = "onnx" if ctr_value is not None else "heuristic"
        predicted_ctr = ctr_value if ctr_value is not None else ctr_heuristic(payload)
        response = InferenceResponse(
            model_name="ctr_prediction",
            model_version=handle.version,
            prediction={"predicted_ctr": predicted_ctr, "source": source},
            confidence=min(0.99, predicted_ctr * 10),
            latency_ms=0.0,
        )
        response.transparency_receipt = make_transparency_receipt(
            tenant,
            payload.model_dump(),
            response.prediction,
            require_transparency_secret(),
        )
        status = "mock" if handle.mock_only or source == "heuristic" else "success"
        return response, status

    return inference_wrapper("ctr_prediction", tenant, handler)


@app.post("/predict/bid", response_model=InferenceResponse)
async def optimize_bid(
    payload: BidOptimizationRequest,
    http_request: Request,
    auth: AuthContext = Depends(authorize_request),
):
    _ = auth
    tenant = getattr(http_request.state, "tenant_id", None) or _require_tenant(http_request)

    def handler():
        handle = require_model("bid_optimization")
        features = np.array([[payload.floor_cpm, payload.predicted_ctr]], dtype=np.float32)
        optimized = run_session(handle, features)
        source = "onnx" if optimized is not None else "heuristic"
        optimized_bid = optimized if optimized is not None else bid_heuristic(payload)
        response = InferenceResponse(
            model_name="bid_optimization",
            model_version=handle.version,
            prediction={
                "optimized_cpm": optimized_bid,
                "confidence_interval": [optimized_bid * 0.9, optimized_bid * 1.1],
                "source": source,
            },
            confidence=0.8,
            latency_ms=0.0,
        )
        response.transparency_receipt = make_transparency_receipt(
            tenant,
            payload.model_dump(),
            response.prediction,
            require_transparency_secret(),
        )
        status = "mock" if handle.mock_only or source == "heuristic" else "success"
        return response, status

    return inference_wrapper("bid_optimization", tenant, handler)


@app.post("/v1/replay/auction")
async def replay_auction(
    payload: AuctionReplayRequest,
    http_request: Request,
    auth: AuthContext = Depends(authorize_request),
):
    _ = auth
    tenant = getattr(http_request.state, "tenant_id", None) or _require_tenant(http_request)
    if not payload.snapshots:
        raise HTTPException(status_code=400, detail="At least one snapshot is required")

    sorted_snaps = sorted(
        payload.snapshots,
        key=lambda snap: (-snap.bid_cpm, snap.latency_ms, snap.adapter.lower()),
    )
    landscape = [
        {
            "adapter": snap.adapter,
            "bid_cpm": snap.bid_cpm,
            "latency_ms": snap.latency_ms,
            "metadata": snap.metadata,
        }
        for snap in sorted_snaps
    ]
    winner = landscape[0]

    response_payload = {"winner": winner, "landscape": landscape}
    receipt = make_transparency_receipt(
        tenant,
        {
            "auction_id": payload.auction_id,
            "request_payload": payload.request_payload,
            "snapshots": [snap.model_dump() for snap in payload.snapshots],
        },
        response_payload,
        require_transparency_secret(),
    )

    return {
        "tenant": tenant,
        "auction_id": payload.auction_id,
        "winner": winner,
        "landscape": landscape,
        "transparency_receipt": receipt,
    }


@app.get("/models")
async def list_models():
    return {
        "models": [
            {
                "name": handle.name,
                "version": handle.version,
                "loaded": handle.session is not None,
                "mock_only": handle.mock_only,
            }
            for handle in MODEL_REGISTRY.values()
        ],
        "allowed_models": list(MODEL_NAMES),
    }


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=8000)
