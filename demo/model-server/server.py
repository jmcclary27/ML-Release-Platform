#!/usr/bin/env python3
"""Deterministic HTTP model stub used by the local Docker release demo."""

import json
import os
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


BEHAVIOR = os.environ.get("MODEL_BEHAVIOR", "good")


class ModelHandler(BaseHTTPRequestHandler):
    def _send_json(self, status, payload):
        encoded = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def do_GET(self):
        if self.path == "/healthz":
            self._send_json(HTTPStatus.OK, {"status": "ok"})
            return
        self._send_json(HTTPStatus.NOT_FOUND, {"error": "not found"})

    def do_POST(self):
        if self.path != "/infer":
            self._send_json(HTTPStatus.NOT_FOUND, {"error": "not found"})
            return
        if BEHAVIOR == "bad":
            self._send_json(HTTPStatus.SERVICE_UNAVAILABLE, {"error": "simulated inference failure"})
            return
        self._send_json(HTTPStatus.OK, {"prediction": "approved", "model_behavior": BEHAVIOR})

    def log_message(self, _format, *_args):
        """Keep demo output focused on the platform's release records."""


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", 8080), ModelHandler).serve_forever()
