#!/usr/bin/env python3
"""Reference implementation of the v1 detector HTTP contract.

This intentionally has no model artifact dependency. It lets the local release
demo exercise the same request validation, health, and prediction-response
boundaries that a production detector must implement.
"""

import json
import math
import os
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


CONTRACT_VERSION = "v1"
MODEL_ID = "reference-market-regime-detector"
BEHAVIOR = os.environ.get("MODEL_BEHAVIOR", "valid")
REQUIRED_BAR_FIELDS = ("timestamp", "symbol", "open", "high", "low", "close", "volume")
INVALID_RESPONSE_PATH = "/app/invalid-response-v1.json"


def is_finite_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def validate_request(payload):
    if not isinstance(payload, dict):
        return "request body must be a JSON object"
    if payload.get("contract_version") != CONTRACT_VERSION:
        return "contract_version must be 'v1'"
    if not isinstance(payload.get("request_id"), str) or not payload["request_id"]:
        return "request_id must be a non-empty string"

    bars = payload.get("market_bars")
    if not isinstance(bars, list) or not bars:
        return "market_bars must be a non-empty array"

    for index, bar in enumerate(bars):
        if not isinstance(bar, dict):
            return f"market_bars[{index}] must be an object"
        missing = [field for field in REQUIRED_BAR_FIELDS if field not in bar]
        if missing:
            return f"market_bars[{index}] is missing {', '.join(missing)}"
        if not isinstance(bar["timestamp"], str) or not bar["timestamp"]:
            return f"market_bars[{index}].timestamp must be a non-empty string"
        if not isinstance(bar["symbol"], str) or not bar["symbol"]:
            return f"market_bars[{index}].symbol must be a non-empty string"
        for field in ("open", "high", "low", "close", "volume"):
            if not is_finite_number(bar[field]):
                return f"market_bars[{index}].{field} must be a finite number"
        if bar["high"] < max(bar["open"], bar["close"], bar["low"]):
            return f"market_bars[{index}].high is inconsistent with OHLC values"
        if bar["low"] > min(bar["open"], bar["close"], bar["high"]):
            return f"market_bars[{index}].low is inconsistent with OHLC values"
        if bar["volume"] < 0:
            return f"market_bars[{index}].volume must not be negative"
    return None


def predictions_for(payload):
    """Return a stable numeric prediction derived from each supplied market bar."""
    predictions = []
    for bar in payload["market_bars"]:
        # A deterministic proxy score, deliberately not a market-regime algorithm.
        prediction = round((bar["close"] - bar["open"]) / bar["open"], 8)
        predictions.append(
            {
                "timestamp": bar["timestamp"],
                "symbol": bar["symbol"],
                "prediction": prediction,
            }
        )
    return predictions


class DetectorHandler(BaseHTTPRequestHandler):
    def _send_json(self, status, payload):
        encoded = json.dumps(payload, allow_nan=False, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def do_GET(self):
        if self.path != "/health":
            self._send_json(HTTPStatus.NOT_FOUND, {"error": "not found"})
            return
        if BEHAVIOR == "unhealthy":
            self._send_json(
                HTTPStatus.SERVICE_UNAVAILABLE,
                {
                    "contract_version": CONTRACT_VERSION,
                    "status": "not_ready",
                    "model_id": MODEL_ID,
                },
            )
            return
        self._send_json(
            HTTPStatus.OK,
            {
                "contract_version": CONTRACT_VERSION,
                "status": "ready",
                "model_id": MODEL_ID,
            },
        )

    def do_POST(self):
        if self.path != "/infer":
            self._send_json(HTTPStatus.NOT_FOUND, {"error": "not found"})
            return
        if self.headers.get_content_type() != "application/json":
            self._send_json(HTTPStatus.UNSUPPORTED_MEDIA_TYPE, {"error": "content type must be application/json"})
            return
        try:
            content_length = int(self.headers.get("Content-Length", ""))
        except ValueError:
            self._send_json(HTTPStatus.BAD_REQUEST, {"error": "invalid Content-Length"})
            return
        if content_length < 0:
            self._send_json(HTTPStatus.BAD_REQUEST, {"error": "invalid Content-Length"})
            return
        try:
            payload = json.loads(self.rfile.read(content_length).decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            self._send_json(HTTPStatus.BAD_REQUEST, {"error": "request body must be valid JSON"})
            return

        request_error = validate_request(payload)
        if request_error:
            self._send_json(HTTPStatus.UNPROCESSABLE_ENTITY, {"error": request_error})
            return
        if BEHAVIOR == "unhealthy":
            self._send_json(HTTPStatus.SERVICE_UNAVAILABLE, {"error": "detector is not ready"})
            return
        if BEHAVIOR == "invalid":
            # Deliberately violates the contract while remaining syntactically valid JSON.
            with open(INVALID_RESPONSE_PATH, encoding="utf-8") as fixture:
                self._send_json(HTTPStatus.OK, json.load(fixture))
            return

        self._send_json(
            HTTPStatus.OK,
            {
                "contract_version": CONTRACT_VERSION,
                "request_id": payload["request_id"],
                "model_id": MODEL_ID,
                "predictions": predictions_for(payload),
            },
        )

    def log_message(self, _format, *_args):
        """Keep demo output focused on the platform's release records."""


if __name__ == "__main__":
    if BEHAVIOR not in {"valid", "invalid", "unhealthy"}:
        raise SystemExit("MODEL_BEHAVIOR must be valid, invalid, or unhealthy")
    ThreadingHTTPServer(("0.0.0.0", 8080), DetectorHandler).serve_forever()
