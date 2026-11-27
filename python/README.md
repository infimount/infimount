# Infimount native Python bindings

The `infimount` package provides explicitly configured, governed access to local
and S3-compatible storage. The native extension uses shared Infimount policy and
OpenDAL operations. It does not require a desktop session or hosted process.

Configuration is trusted application input. This in-process library is not a
sandbox for hostile Python code. Use the authenticated hosted service or an
independent sandbox for untrusted agent execution.

The API supports byte reads, conditional create-only writes, paginated listings,
metadata, file-only deletion and explicitly authorized native download links.
Policy rules requiring confirmation fail closed. No method can issue its own
human approval, obtain a raw operator, overwrite files or recursively delete.

See [the Python integration guide](https://github.com/infimount/infimount/blob/main/docs/python-bindings.md)
for configuration, limits, audit collection and platform qualification.
