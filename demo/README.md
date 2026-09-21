# Local Docker demo assets

Run the complete local acceptance demo with:

```bash
./demo/run.sh
```

It builds deterministic reference-detector images, starts the API, promotes valid
v1 from real fixture inference, rolls back a healthy but schema-invalid v2, rejects
an unhealthy v3 during startup, verifies that v1 remains active, then removes every
platform-managed container and its exact local SQLite/log files in a cleanup trap.
It never invokes Terraform or cloud APIs.

The demo sets a three-second startup-health deadline only to keep the deliberate
unhealthy-image case fast; the application default remains 30 seconds.

To build images without running the API demo:

```bash
./demo/build-images.sh
```

`mlrp-demo-detector:valid` is a minimal reference implementation of the
versioned [detector contract](../docs/detector-http-contract.md): it validates
the market-bar request and produces deterministic numeric predictions.
`mlrp-demo-detector:invalid` is healthy but returns the checked-in,
schema-invalid [`invalid-response-v1.json`](model-server/invalid-response-v1.json)
with `200`, proving runtime rollback. `mlrp-demo-detector:unhealthy`
returns `503` from `/health`, proving deployment rejection. The fixture used
by the platform is [`fixtures/market-data-inference-v1.json`](fixtures/market-data-inference-v1.json).

The reference detector is not the Market Regime Detector and contains no
trained artifact. That project will eventually provide its own HTTP adapter
and externally supplied model artifacts behind this same boundary.

After a demo, remove only release-platform-managed containers with:

```bash
./demo/cleanup.sh
```

The API/CLI demo sequence is documented with the MVP interface because it needs the current
release IDs returned by that interface. Neither script provisions cloud infrastructure.
