# Local Docker demo assets

Run the complete local acceptance demo with:

```bash
./demo/run.sh
```

It builds the deterministic images, starts the API, releases good v1, attempts bad v2,
prints the restored active v1 and audit history, then removes every platform-managed
container and its exact local SQLite/log files in a cleanup trap. It never invokes Terraform
or cloud APIs.

To build images without running the API demo:

```bash
./demo/build-images.sh
```

`mlrp-demo-model:good` responds successfully to both `GET /healthz` and `POST /infer`.
`mlrp-demo-model:bad` has the same startup health response but returns HTTP 503 for inference.
This lets a release pass deployment health checks and fail only runtime verification.

After a demo, remove only release-platform-managed containers with:

```bash
./demo/cleanup.sh
```

The API/CLI demo sequence is documented with the MVP interface because it needs the current
release IDs returned by that interface. Neither script provisions cloud infrastructure.
