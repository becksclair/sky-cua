# sky-cua-model-client

Reusable, provider-neutral JSON POST transport. Callers supply the complete
endpoint, headers, and payload and interpret the returned JSON value. Reuse a
client to retain its HTTP connection pool.

```rust,no_run
use std::time::Duration;
use serde_json::json;
use sky_cua_model_client::ModelClient;

let client = ModelClient::new(Duration::from_secs(30), 4 * 1024 * 1024)?;
let response = client.post_json(
    "https://provider.example/infer",
    &[("Authorization", "Bearer caller-supplied-token")],
    &json!({"engine": "caller-selected", "input": "hello"}),
)?;
println!("{}", response.body);
# Ok::<(), sky_cua_model_client::ModelError>(())
```

Requests have a total timeout and bounded response size. Redirects are disabled.
Errors expose a category and HTTP status when available, without URLs, headers,
or request/response bodies. The transport has no provider schema, credential
discovery, retry policy, or model-selection behavior.
