"""Fraud inference service with authenticated scoring and RED metrics."""

import base64
import hashlib
import hmac
import importlib
import json
import logging
import os
import sys
import time
import warnings
from collections import defaultdict, deque
from contextlib import asynccontextmanager
from pathlib import Path
from threading import Lock
from typing import Any, Dict, List, Optional

import joblib
import numpy as np
import onnxruntime as ort


def _ensure_python_multipart() -> None:
    try:  # pragma: no cover
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

from fastapi import Depends, FastAPI, HTTPException, Request, Response
from prometheus_client import CONTENT_TYPE_LATEST, Counter, Gauge, Histogram, generate_latest
from pydantic import BaseModel, ConfigDict, Field

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
)
logger = logging.getLogger(__name__)

# Prometheus metrics (RED: Rate, Errors, Duration)
REQUEST_COUNT = Counter(
    "fraud_inference_requests_total",
    "Total fraud inference requests",
    ["endpoint", "status", "tenant"],
)
REQUEST_DURATION = Histogram(
    "fraud_inference_duration_seconds",
    "Fraud inference request duration",
    ["endpoint", "tenant"],
    buckets=[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0],
)
ERROR_COUNT = Counter(
    "fraud_inference_errors_total",
    "Total fraud inference errors",
    ["endpoint", "error_type", "tenant"],
)

FRAUD_SCORE_HISTOGRAM = Histogram(
    "fraud_score_histogram",
    "Distribution of fraud scores per tenant",
    ["tenant", "model_version"],
    buckets=[0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0],
)
FRAUD_DRIFT_GAUGE = Gauge(
    "fraud_drift_js_divergence",
    "Jensen-Shannon divergence between tenant scores and global baseline",
    ["tenant"],
)

# Bias/Fairness Metrics
FRAUD_SCORE_BY_CONSENT_HISTOGRAM = Histogram(
    "fraud_score_by_consent",
    "Distribution of fraud scores segmented by consent status",
    ["tenant", "consent_type", "consent_value"],
    buckets=[0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0],
)

BIAS_DISPARITY_GAUGE = Gauge(
    "fraud_bias_disparity",
    "Demographic parity disparity between consent groups (abs diff in mean scores)",
    ["tenant", "consent_type"],
)

FAIRNESS_RATIO_GAUGE = Gauge(
    "fraud_fairness_ratio",
    "Fairness ratio (min/max mean score) across consent groups - closer to 1.0 is better",
    ["tenant", "consent_type"],
)

SCORE_WINDOWS: Dict[str, deque] = defaultdict(lambda: deque(maxlen=400))
GLOBAL_SCORE_WINDOW: deque = deque(maxlen=4000)

# Bias tracking windows (consent_type -> consent_value -> scores)
CONSENT_SCORE_WINDOWS: Dict[str, Dict[str, deque]] = defaultdict(
    lambda: defaultdict(lambda: deque(maxlen=200))
)


class FraudFeatures(BaseModel):
    """Input features for fraud detection."""
    feature_1: float = Field(..., description="Network entropy")
    feature_2: float = Field(..., description="Click-to-install time")
    feature_3: float = Field(..., description="IP reputation score")
    # Add more features as needed


class FraudDetectionRequest(FraudFeatures):
    """Fraud detection request enriched with consent state."""

    consent_gdpr: Optional[bool] = Field(
        default=None,
        description="End-user GDPR consent flag",
    )
    consent_tcf: Optional[str] = Field(
        default=None,
        description="Encoded IAB TCF string (redacted at rest)",
    )
    us_privacy: Optional[str] = Field(
        default=None,
        description="US Privacy / CCPA string (redacted at rest)",
    )
    coppa: Optional[bool] = Field(
        default=None,
        description="Child-directed treatment flag",
    )


class FraudScore(BaseModel):
    """Fraud detection response."""
    fraud_score: float = Field(..., ge=0.0, le=1.0, description="Fraud probability")
    is_fraud: bool = Field(..., description="Binary fraud decision")
    threshold: float = Field(..., description="Decision threshold used")
    latency_ms: float = Field(..., description="Inference latency in milliseconds")
    mode: str = Field(..., description="Fraud decision mode for this tenant")
    shadow_decision: bool = Field(..., description="Raw fraud decision prior to mode gating")


class HealthResponse(BaseModel):
    model_config = ConfigDict(protected_namespaces=())

    """Health check response."""
    status: str
    model_loaded: bool
    model_version: Optional[str] = None
    uptime_seconds: float


class ModelState:
    """Global model state."""
    model: Optional[ort.InferenceSession] = None
    threshold: float = 0.5
    feature_names: List[str] = []
    cat_cardinalities: List[int] = []
    model_version: str = "unknown"
    start_time: float = time.time()
    scaler: Optional[Any] = None


state = ModelState()

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
RATE_LIMIT_MAX_REQUESTS = int(os.getenv("RATE_LIMIT_MAX_REQUESTS", "300"))
RATE_LIMIT_WINDOW_SECONDS = int(os.getenv("RATE_LIMIT_WINDOW_SECONDS", "60"))
DEFAULT_FRAUD_MODE = os.getenv("FRAUD_MODE", "shadow").strip().lower() or "shadow"
if DEFAULT_FRAUD_MODE not in {"shadow", "block"}:
    logger.warning("Unsupported FRAUD_MODE provided; defaulting to shadow", extra={"mode": DEFAULT_FRAUD_MODE})
    DEFAULT_FRAUD_MODE = "shadow"

_mode_overrides_raw = os.getenv("FRAUD_MODE_OVERRIDES_JSON", "{}")
try:
    TENANT_MODE_OVERRIDES = json.loads(_mode_overrides_raw)
    if not isinstance(TENANT_MODE_OVERRIDES, dict):
        logger.warning("FRAUD_MODE_OVERRIDES_JSON must be a JSON object")
        TENANT_MODE_OVERRIDES = {}
except json.JSONDecodeError:
    logger.warning("Invalid FRAUD_MODE_OVERRIDES_JSON; ignoring overrides")
    TENANT_MODE_OVERRIDES = {}


class RateLimiter:
    """Simple in-memory sliding-window rate limiter keyed by tenant."""

    def __init__(self, max_requests: int, window_seconds: int) -> None:
        self.max_requests = max_requests
        self.window_seconds = window_seconds
        self._hits: Dict[str, deque] = defaultdict(deque)
        self._lock = Lock()

    def hit(self, key: str) -> None:
        if self.max_requests <= 0:
            return

        now = time.time()
        window_floor = now - self.window_seconds

        with self._lock:
            bucket = self._hits[key]
            while bucket and bucket[0] < window_floor:
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


def mode_for(tenant: str) -> str:
    override = TENANT_MODE_OVERRIDES.get(tenant)
    if isinstance(override, str):
        candidate = override.strip().lower()
        if candidate in {"shadow", "block"}:
            return candidate
    return DEFAULT_FRAUD_MODE


class RedactedRequestBuffer:
    """Thread-safe redaction buffer for debugging without storing PII."""

    def __init__(self, maxlen: int = 200) -> None:
        self._entries: deque = deque(maxlen=maxlen)
        self._lock = Lock()

    def record(self, tenant: str, payload: "FraudDetectionRequest") -> None:
        redacted = {
            "tenant": tenant,
            "timestamp": time.time(),
            "features": {
                "feature_1": round(float(payload.feature_1), 6),
                "feature_2": round(float(payload.feature_2), 6),
                "feature_3": round(float(payload.feature_3), 6),
            },
            "consent": {
                "gdpr": payload.consent_gdpr,
                "coppa": payload.coppa,
                "tcf_hash": _hash_text(payload.consent_tcf),
                "us_privacy_hash": _hash_text(payload.us_privacy),
            },
        }
        with self._lock:
            self._entries.append(redacted)

    def snapshot(self) -> List[Dict[str, Any]]:
        with self._lock:
            return list(self._entries)


def _hash_text(value: Optional[str]) -> Optional[str]:
    if value is None:
        return None
    digest = hashlib.sha256(value.encode("utf-8")).hexdigest()
    return digest[:12]


DEBUG_BUFFER = RedactedRequestBuffer()


def _base64url_decode(segment: str) -> bytes:
    padding = "=" * (-len(segment) % 4)
    return base64.urlsafe_b64decode(segment + padding)


def decode_jwt_token(token: str) -> Dict[str, Any]:
    if not JWT_SECRET:
        logger.error("JWT_SECRET not configured for fraud inference service")
        raise HTTPException(status_code=500, detail="Authentication configuration error")

    parts = token.split(".")
    if len(parts) != 3:
        raise HTTPException(status_code=401, detail="Invalid token format")

    header_segment, payload_segment, signature_segment = parts

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
        audience_claim = payload.get("aud")
        audiences = {audience_claim} if isinstance(audience_claim, str) else set(audience_claim or [])
        if JWT_AUDIENCE not in audiences:
            raise HTTPException(status_code=403, detail="Invalid token audience")

    return payload


def _require_tenant(request: Request) -> str:
    tenant = request.headers.get(TENANT_HEADER)
    if not tenant:
        raise HTTPException(status_code=400, detail=f"Missing {TENANT_HEADER}")
    request.state.tenant = tenant
    return tenant


def authorize_request(request: Request) -> AuthContext:
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

    tenant_hint = _require_tenant(request)
    if tenant_hint != publisher_id:
        raise HTTPException(status_code=403, detail="Tenant mismatch")

    context = AuthContext(user_id=user_id, publisher_id=publisher_id, email=email, role=role)
    request.state.auth_context = context

    key = f"{publisher_id}:{request.url.path}"
    RATE_LIMITER.hit(key)

    return context


def resolve_model_path(env_var: str, default: str) -> Path:
    raw_path = os.getenv(env_var, default)
    candidate = Path(raw_path).expanduser().resolve()
    if not candidate.exists():
        raise FileNotFoundError(f"Required model artifact missing at {candidate}")
    if candidate.is_dir():
        raise IsADirectoryError(f"Expected file for {env_var}, found directory at {candidate}")
    return candidate


def load_model():
    """Load ONNX model for CPU inference."""
    try:
        model_path = resolve_model_path("MODEL_PATH", "models/fraud/latest/fraud_model.onnx")
    except (FileNotFoundError, IsADirectoryError) as exc:
        if not ALLOW_MODEL_MOCKS:
            raise
        logger.warning(
            "Model artifact missing but mock mode enabled",
            extra={"error": str(exc)},
        )
        state.model = None
        state.threshold = 0.5
        state.feature_names = []
        state.model_version = "mock"
        return

    try:
        # CPU-optimized session with quantization support
        session_options = ort.SessionOptions()
        session_options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
        session_options.intra_op_num_threads = int(os.getenv("OMP_NUM_THREADS", "4"))

        state.model = ort.InferenceSession(
            str(model_path),
            sess_options=session_options,
            providers=["CPUExecutionProvider"],
        )

        # Load metadata if present
        meta_path_default = "models/fraud/latest/trained_fraud_model.json"
        meta_path_env = os.getenv("MODEL_META_PATH", meta_path_default)
        meta_path = Path(meta_path_env).expanduser().resolve()
        if meta_path.exists():
            import json

            with open(meta_path, "r", encoding="utf-8") as f:
                meta = json.load(f)
                state.threshold = float(meta.get("threshold", 0.5))
                state.feature_names = meta.get("features", [])
                state.cat_cardinalities = meta.get("cat_cardinalities", [3, 100, 100, 3])
                state.model_version = meta.get("version", meta_path.parent.name)
        else:
            logger.warning("Model metadata not found; using defaults", extra={"meta_path": str(meta_path)})
            state.cat_cardinalities = [3, 100, 100, 3]

        scaler_path_env = os.getenv("MODEL_SCALER_PATH")
        scaler_candidate = Path(scaler_path_env).expanduser().resolve() if scaler_path_env else meta_path.with_name("scaler.joblib")
        if scaler_candidate.exists():
            state.scaler = joblib.load(scaler_candidate)
            logger.info("Loaded scaler", extra={"scaler_path": str(scaler_candidate)})

        logger.info(
            "Model loaded",
            extra={
                "model_path": str(model_path),
                "version": state.model_version,
                "threshold": state.threshold,
                "features": state.feature_names or "fallback",
            },
        )

    except Exception as e:
        logger.error("Failed to load model", extra={"error": str(e)})
        raise


def _encode_bool_index(flag: Optional[bool]) -> int:
    if flag is True:
        return 2
    if flag is False:
        return 1
    return 0


def _encode_string_index(value: Optional[str], cardinality: int) -> int:
    if not value:
        return 0
    digest = hashlib.sha256(value.encode("utf-8")).digest()
    # Use modulo to map to [0, cardinality)
    return int.from_bytes(digest[:4], "big") % cardinality


def transform_features(sample: FraudFeatures) -> Dict[str, np.ndarray]:
    """Build model-ready feature arrays (numerical and categorical)."""

    # Numerical features
    x_num_list = [
        float(sample.feature_1),
        float(sample.feature_2),
        float(sample.feature_3),
    ]
    x_num = np.array([x_num_list], dtype=np.float32)

    if state.scaler is not None:
        try:
            x_num = state.scaler.transform(x_num)
        except Exception as exc:
            logger.warning("Failed to transform features with scaler", extra={"error": str(exc)})

    # Categorical features
    # Order: consent_gdpr, consent_tcf, us_privacy, coppa
    # Defaults if state.cat_cardinalities is not set (should be set by load_model)
    cards = state.cat_cardinalities or [3, 100, 100, 3]
    
    # Ensure we have enough cardinalities, else pad with defaults
    if len(cards) < 4:
        cards = cards + [100] * (4 - len(cards))

    consent_gdpr = None
    consent_tcf = None
    us_privacy = None
    coppa = None

    if isinstance(sample, FraudDetectionRequest):
        consent_gdpr = sample.consent_gdpr
        consent_tcf = sample.consent_tcf
        us_privacy = sample.us_privacy
        coppa = sample.coppa

    x_cat_list = [
        _encode_bool_index(consent_gdpr),
        _encode_string_index(consent_tcf, cards[1]),
        _encode_string_index(us_privacy, cards[2]),
        _encode_bool_index(coppa),
    ]
    x_cat = np.array([x_cat_list], dtype=np.int64)

    return {"x_num": x_num.astype(np.float32), "x_cat": x_cat}


def _record_score_metrics(tenant: str, score: float, payload: Optional["FraudDetectionRequest"] = None) -> None:
    FRAUD_SCORE_HISTOGRAM.labels(tenant=tenant, model_version=state.model_version).observe(score)
    tenant_window = SCORE_WINDOWS[tenant]
    tenant_window.append(score)
    GLOBAL_SCORE_WINDOW.append(score)
    _update_drift_metric(tenant)
    
    # Record bias metrics if payload available
    if payload is not None:
        _record_bias_metrics(tenant, score, payload)


def _update_drift_metric(tenant: str) -> None:
    tenant_window = SCORE_WINDOWS.get(tenant)
    if not tenant_window or len(tenant_window) < 5 or len(GLOBAL_SCORE_WINDOW) < 20:
        return

    tenant_dist = _distribution_from_window(tenant_window)
    global_dist = _distribution_from_window(GLOBAL_SCORE_WINDOW)
    drift = _js_divergence(tenant_dist, global_dist)
    FRAUD_DRIFT_GAUGE.labels(tenant=tenant).set(float(drift))


def _distribution_from_window(values: deque) -> np.ndarray:
    hist, _ = np.histogram(list(values), bins=10, range=(0.0, 1.0))
    total = hist.sum()
    if total == 0:
        return np.full(10, 0.1)
    return hist / total


def _js_divergence(p: np.ndarray, q: np.ndarray) -> float:
    m = 0.5 * (p + q)

    def _kl(a: np.ndarray, b: np.ndarray) -> float:
        mask = (a > 0) & (b > 0)
        if not np.any(mask):
            return 0.0
        return float(np.sum(a[mask] * np.log2(a[mask] / b[mask])))

    return 0.5 * _kl(p, m) + 0.5 * _kl(q, m)


def _record_bias_metrics(tenant: str, score: float, payload: "FraudDetectionRequest") -> None:
    """Record bias/fairness metrics segmented by consent attributes."""
    # Track scores by GDPR consent
    gdpr_value = "true" if payload.consent_gdpr is True else ("false" if payload.consent_gdpr is False else "unknown")
    FRAUD_SCORE_BY_CONSENT_HISTOGRAM.labels(
        tenant=tenant, consent_type="gdpr", consent_value=gdpr_value
    ).observe(score)
    CONSENT_SCORE_WINDOWS[f"{tenant}:gdpr"][gdpr_value].append(score)
    
    # Track scores by COPPA
    coppa_value = "true" if payload.coppa is True else ("false" if payload.coppa is False else "unknown")
    FRAUD_SCORE_BY_CONSENT_HISTOGRAM.labels(
        tenant=tenant, consent_type="coppa", consent_value=coppa_value
    ).observe(score)
    CONSENT_SCORE_WINDOWS[f"{tenant}:coppa"][coppa_value].append(score)
    
    # Track scores by TCF presence
    tcf_value = "present" if payload.consent_tcf else "absent"
    FRAUD_SCORE_BY_CONSENT_HISTOGRAM.labels(
        tenant=tenant, consent_type="tcf", consent_value=tcf_value
    ).observe(score)
    CONSENT_SCORE_WINDOWS[f"{tenant}:tcf"][tcf_value].append(score)
    
    # Track scores by US Privacy presence
    us_privacy_value = "present" if payload.us_privacy else "absent"
    FRAUD_SCORE_BY_CONSENT_HISTOGRAM.labels(
        tenant=tenant, consent_type="us_privacy", consent_value=us_privacy_value
    ).observe(score)
    CONSENT_SCORE_WINDOWS[f"{tenant}:us_privacy"][us_privacy_value].append(score)
    
    # Update bias disparity metrics periodically
    _update_bias_metrics(tenant)


def _update_bias_metrics(tenant: str) -> None:
    """Calculate and update demographic parity metrics."""
    for consent_type in ["gdpr", "coppa", "tcf", "us_privacy"]:
        key = f"{tenant}:{consent_type}"
        windows = CONSENT_SCORE_WINDOWS.get(key, {})
        
        # Need at least 2 groups with enough samples
        group_means = {}
        for group_value, scores in windows.items():
            if len(scores) >= 10:
                group_means[group_value] = np.mean(list(scores))
        
        if len(group_means) < 2:
            continue
        
        # Calculate demographic parity disparity (max - min mean)
        mean_values = list(group_means.values())
        disparity = max(mean_values) - min(mean_values)
        BIAS_DISPARITY_GAUGE.labels(tenant=tenant, consent_type=consent_type).set(disparity)
        
        # Calculate fairness ratio (min/max - closer to 1.0 is better)
        if max(mean_values) > 0:
            ratio = min(mean_values) / max(mean_values)
            FAIRNESS_RATIO_GAUGE.labels(tenant=tenant, consent_type=consent_type).set(ratio)


@asynccontextmanager
async def lifespan(app: FastAPI):
    """Application lifespan manager."""
    logger.info("Starting fraud inference service...")
    load_model()
    yield
    logger.info("Shutting down fraud inference service...")


app = FastAPI(
    title="Fraud Detection Inference Service",
    description="CPU-optimized fraud scoring with RED metrics",
    version="1.0.0",
    lifespan=lifespan,
)


@app.middleware("http")
async def metrics_middleware(request: Request, call_next):
    """Record RED metrics for all requests."""
    start_time = time.time()
    tenant = getattr(request.state, "tenant", request.headers.get(TENANT_HEADER, "unknown"))

    try:
        response = await call_next(request)
        duration = time.time() - start_time
        
        endpoint = request.url.path
        tenant = getattr(request.state, "tenant", tenant)
        REQUEST_COUNT.labels(endpoint=endpoint, status=response.status_code, tenant=tenant).inc()
        REQUEST_DURATION.labels(endpoint=endpoint, tenant=tenant).observe(duration)
        
        return response
    
    except Exception as e:
        duration = time.time() - start_time
        endpoint = request.url.path
        tenant = getattr(request.state, "tenant", tenant)
        
        ERROR_COUNT.labels(endpoint=endpoint, error_type=type(e).__name__, tenant=tenant).inc()
        REQUEST_DURATION.labels(endpoint=endpoint, tenant=tenant).observe(duration)
        
        raise


@app.get("/health/live", response_model=HealthResponse, tags=["Health"])
async def liveness():
    """
    Liveness probe: service is running.
    Returns 200 if process is alive.
    """
    return HealthResponse(
        status="alive",
        model_loaded=state.model is not None,
        model_version=state.model_version,
        uptime_seconds=time.time() - state.start_time,
    )


@app.get("/health/ready", response_model=HealthResponse, tags=["Health"])
async def readiness():
    """
    Readiness probe: service is ready to accept traffic.
    Returns 200 if model is loaded, 503 otherwise.
    """
    if state.model is None:
        raise HTTPException(status_code=503, detail="Model not loaded")
    
    return HealthResponse(
        status="ready",
        model_loaded=True,
        model_version=state.model_version,
        uptime_seconds=time.time() - state.start_time,
    )


@app.get("/metrics", tags=["Observability"])
async def metrics():
    """Prometheus metrics endpoint."""
    return Response(content=generate_latest(), media_type=CONTENT_TYPE_LATEST)


class BiasReportGroup(BaseModel):
    """Bias statistics for a single group."""
    group_value: str
    sample_count: int
    mean_score: float
    std_score: float
    min_score: float
    max_score: float


class BiasReportDimension(BaseModel):
    """Bias report for a single consent dimension."""
    consent_type: str
    groups: List[BiasReportGroup]
    disparity: Optional[float] = None
    fairness_ratio: Optional[float] = None
    bias_alert: bool = False


class BiasReport(BaseModel):
    """Full bias/fairness report for a tenant."""
    tenant: str
    timestamp: float
    total_samples: int
    dimensions: List[BiasReportDimension]
    overall_bias_alert: bool = False


@app.get("/v1/bias/report", response_model=BiasReport, tags=["Fairness"])
async def get_bias_report(
    request: Request,
    auth: AuthContext = Depends(authorize_request),
):
    """
    Get bias/fairness report for the authenticated tenant.
    
    Reports demographic parity metrics across consent dimensions:
    - GDPR consent (true/false/unknown)
    - COPPA (true/false/unknown)
    - TCF presence (present/absent)
    - US Privacy presence (present/absent)
    
    Alerts if disparity > 0.1 or fairness_ratio < 0.8
    """
    _ = auth
    tenant = getattr(request.state, "tenant", _require_tenant(request))
    
    dimensions: List[BiasReportDimension] = []
    overall_alert = False
    total_samples = 0
    
    for consent_type in ["gdpr", "coppa", "tcf", "us_privacy"]:
        key = f"{tenant}:{consent_type}"
        windows = CONSENT_SCORE_WINDOWS.get(key, {})
        
        groups: List[BiasReportGroup] = []
        for group_value, scores in windows.items():
            score_list = list(scores)
            if not score_list:
                continue
            
            total_samples += len(score_list)
            groups.append(BiasReportGroup(
                group_value=group_value,
                sample_count=len(score_list),
                mean_score=float(np.mean(score_list)),
                std_score=float(np.std(score_list)) if len(score_list) > 1 else 0.0,
                min_score=float(np.min(score_list)),
                max_score=float(np.max(score_list)),
            ))
        
        # Calculate metrics if we have enough data
        disparity = None
        fairness_ratio = None
        bias_alert = False
        
        if len(groups) >= 2:
            means = [g.mean_score for g in groups if g.sample_count >= 10]
            if len(means) >= 2:
                disparity = max(means) - min(means)
                fairness_ratio = min(means) / max(means) if max(means) > 0 else 1.0
                
                # Alert thresholds
                if disparity > 0.1 or fairness_ratio < 0.8:
                    bias_alert = True
                    overall_alert = True
        
        dimensions.append(BiasReportDimension(
            consent_type=consent_type,
            groups=groups,
            disparity=disparity,
            fairness_ratio=fairness_ratio,
            bias_alert=bias_alert,
        ))
    
    return BiasReport(
        tenant=tenant,
        timestamp=time.time(),
        total_samples=total_samples // 4,  # Approximate unique samples
        dimensions=dimensions,
        overall_bias_alert=overall_alert,
    )


@app.post("/v1/score", response_model=FraudScore, tags=["Inference"])
async def score_fraud(
    payload: FraudDetectionRequest,
    request: Request,
    auth: AuthContext = Depends(authorize_request),
):
    """
    Score fraud probability for given features.
    
    Returns fraud score (0-1) and binary decision based on threshold.
    """
    _ = auth  # enforce dependency evaluation
    tenant = getattr(request.state, "tenant", _require_tenant(request))
    DEBUG_BUFFER.record(tenant, payload)
    if state.model is None:
        ERROR_COUNT.labels(endpoint="/v1/score", error_type="ModelNotLoaded", tenant=tenant).inc()
        raise HTTPException(status_code=503, detail="Model not loaded")
    
    start_time = time.time()
    
    try:
        # Prepare input features
        features = transform_features(payload)
        
        # Run inference
        # We expect inputs: x_num, x_cat
        outputs = state.model.run(None, features)
        
        # Extract fraud probability
        if len(outputs[0].shape) > 1 and outputs[0].shape[1] > 1:
            fraud_score = float(outputs[0][0][1])  # Binary classifier
        else:
            fraud_score = float(outputs[0][0])
        
        latency_ms = (time.time() - start_time) * 1000
        _record_score_metrics(tenant, fraud_score, payload)
        mode = mode_for(tenant)
        shadow_decision = fraud_score >= state.threshold
        is_fraud = shadow_decision and mode == "block"

        return FraudScore(
            fraud_score=fraud_score,
            is_fraud=is_fraud,
            threshold=state.threshold,
            latency_ms=latency_ms,
            mode=mode,
            shadow_decision=shadow_decision,
        )
    
    except Exception as e:
        logger.error(f"Inference error: {e}")
        ERROR_COUNT.labels(endpoint="/v1/score", error_type=type(e).__name__, tenant=tenant).inc()
        raise HTTPException(status_code=500, detail=f"Inference failed: {str(e)}")


@app.post("/v1/score/batch", response_model=List[FraudScore], tags=["Inference"])
async def score_fraud_batch(
    features_list: List[FraudDetectionRequest],
    request: Request,
    auth: AuthContext = Depends(authorize_request),
):
    """
    Batch fraud scoring for multiple samples.
    More efficient than individual requests for large batches.
    """
    _ = auth
    tenant = getattr(request.state, "tenant", _require_tenant(request))

    if state.model is None:
        ERROR_COUNT.labels(endpoint="/v1/score/batch", error_type="ModelNotLoaded", tenant=tenant).inc()
        raise HTTPException(status_code=503, detail="Model not loaded")
    
    if len(features_list) > 1000:
        raise HTTPException(status_code=400, detail="Batch size limited to 1000")
    
    start_time = time.time()
    
    try:
        # Prepare batch input
        for item in features_list:
            DEBUG_BUFFER.record(tenant, item)

        transformed_list = [transform_features(f) for f in features_list]
        x_num_batch = np.vstack([t["x_num"] for t in transformed_list])
        x_cat_batch = np.vstack([t["x_cat"] for t in transformed_list])
        
        # Batch inference
        outputs = state.model.run(None, {"x_num": x_num_batch, "x_cat": x_cat_batch})
        
        # Extract scores
        if len(outputs[0].shape) > 1 and outputs[0].shape[1] > 1:
            fraud_scores = outputs[0][:, 1].tolist()
        else:
            fraud_scores = outputs[0].flatten().tolist()
        
        total_latency_ms = (time.time() - start_time) * 1000
        per_sample_latency = total_latency_ms / len(features_list)
        
        mode = mode_for(tenant)
        shadow_flags = [score >= state.threshold for score in fraud_scores]

        results = [
            FraudScore(
                fraud_score=score,
                is_fraud=(flag and mode == "block"),
                threshold=state.threshold,
                latency_ms=per_sample_latency,
                mode=mode,
                shadow_decision=flag,
            )
            for score, flag in zip(fraud_scores, shadow_flags)
        ]

        for score, item in zip(fraud_scores, features_list):
            _record_score_metrics(tenant, float(score), item)

        return results
    
    except Exception as e:
        logger.error(f"Batch inference error: {e}")
        ERROR_COUNT.labels(endpoint="/v1/score/batch", error_type=type(e).__name__, tenant=tenant).inc()
        raise HTTPException(status_code=500, detail=f"Batch inference failed: {str(e)}")


if __name__ == "__main__":
    import uvicorn
    
    uvicorn.run(
        app,
        host="0.0.0.0",
        port=int(os.getenv("PORT", "8080")),
        log_level="info",
    )
