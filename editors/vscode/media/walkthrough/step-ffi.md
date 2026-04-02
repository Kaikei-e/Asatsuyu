# Python FFI

Import Python modules with `from python import`:

```asatsuyu
from python import pathlib
from python import requests
```

Asatsuyu classifies Python modules by trust level:

- **Verified** (pathlib, os, sys) — full type information, used as normal types
- **Checked** (requests, json) — runtime-validated at the boundary
- **Unsafe** — isolated as opaque types

Use `try` to safely convert Python exceptions to `Result`:

```asatsuyu
let response = try requests.get(url)
let path = pathlib.Path("output.txt")
let _ = try path.write_text(response.text)
```

Exceptions never leak into Asatsuyu's pure domain.
