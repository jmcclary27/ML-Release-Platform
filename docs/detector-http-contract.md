# Detector HTTP Contract v1

The local release adapter treats a detector as an external HTTP service. The
platform owns container lifecycle and release decisions; detector owners retain
ownership of feature engineering, artifact loading, model selection, and the
meaning of predictions.

The contract version is the JSON string `"v1"`. A later incompatible contract
uses a new version rather than silently changing fields.

## Readiness

`GET /health` returns `200 OK` only after the detector is ready to serve its
configured model:

```json
{"contract_version":"v1","status":"ready","model_id":"detector-model:v1"}
```

`model_id` is a non-empty detector-defined immutable model identifier. A
non-2xx response, unavailable endpoint, timeout, or malformed/non-ready 2xx
response is unhealthy. The local adapter polls readiness for 30 seconds by
default (`ML_RELEASE_STARTUP_TIMEOUT_SECONDS`).

## Inference

`POST /infer` requires `Content-Type: application/json` and this request form:

```json
{
  "contract_version": "v1",
  "request_id": "caller-generated-correlation-id",
  "market_bars": [
    {
      "timestamp": "2026-09-18T20:00:00Z",
      "symbol": "SPY",
      "open": 654.36,
      "high": 655.74,
      "low": 653.41,
      "close": 655.12,
      "volume": 52361000
    }
  ]
}
```

`market_bars` is a non-empty evaluation window, chronological for each symbol. Every bar has a
UTC RFC 3339 timestamp, non-empty symbol, and finite positive OHLCV values
with `high >= max(open, close, low)` and `low <= min(open, close, high)`.
The caller supplies a representative multi-symbol window; detector-specific
feature construction stays inside the detector.

On success, return `200 OK` with:

```json
{
  "contract_version": "v1",
  "request_id": "caller-generated-correlation-id",
  "model_id": "detector-model:v1",
  "predictions": [
    {"timestamp":"2026-09-18T20:00:00Z","symbol":"SPY","prediction":0.00098}
  ]
}
```

`request_id` must echo the request, `model_id` must be non-empty, and
`predictions` must be non-empty. Every prediction has non-empty timestamp and
symbol plus a finite numeric scalar. Its business meaning is detector-defined;
the Market Regime Detector will use next-period log-return predictions. The
release platform validates this transport schema only, not forecast quality.

## Errors and timeouts

Use JSON error payloads such as `{"error":{"code":"...","message":"..."}}`.
Return `400` for malformed JSON or unsupported contract versions, `415` for a
non-JSON media type, `422` for a well-formed but invalid or insufficient bar
window, `500` for an inference failure, and `503` when no model is ready.
Unknown paths return `404`.

The release platform submits the checked-in fixture
[`demo/fixtures/market-data-inference-v1.json`](../demo/fixtures/market-data-inference-v1.json)
with a 10-second default inference deadline
(`ML_RELEASE_VERIFICATION_TIMEOUT_SECONDS`). Non-2xx, timeout/transport, or a
schema-invalid 2xx response is recorded as failed runtime verification and
causes rollback of an otherwise deployed candidate.
