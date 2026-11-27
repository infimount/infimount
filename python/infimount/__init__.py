"""Governed native storage access for trusted Python embedding applications.

Construct clients from trusted application configuration. Passing a caller's
configuration through unchanged would give that caller storage authority.
"""
from __future__ import annotations

import json
from collections.abc import Mapping
from typing import Any

from ._native import (
    AlreadyExists,
    BackendUnavailable,
    ConfigurationError,
    ConfirmationRequired,
    InfimountError,
    InvalidRequest,
    LimitExceeded,
    NativeClient,
    NotFound,
    PermissionDenied,
    UnsupportedOperation,
    __version__,
)

__all__ = [
    "Client", "InfimountError", "ConfigurationError", "InvalidRequest",
    "PermissionDenied", "ConfirmationRequired", "NotFound", "AlreadyExists",
    "LimitExceeded", "UnsupportedOperation", "BackendUnavailable", "__version__",
]


class Client:
    """One immutable storage and policy configuration.

    Calls are synchronous and release the GIL during storage I/O. Async callers
    can use asyncio.to_thread. Clients must be created again after process fork.
    """

    __slots__ = ("_client",)

    def __init__(self, config: Mapping[str, Any]) -> None:
        self._client = NativeClient(json.dumps(config, allow_nan=False, separators=(",", ":")))

    @classmethod
    def from_config(cls, config: Mapping[str, Any]) -> Client:
        """Create from trusted local/S3 configuration, scopes and MCP v2 policy."""
        return cls(config)

    def list(self, path: str, *, limit: int = 200, cursor: str | None = None) -> dict[str, Any]:
        """Return a bounded page with entries, next_cursor and truncated fields."""
        return json.loads(self._client.list_json(path, limit=limit, cursor=cursor))

    def stat(self, path: str) -> dict[str, Any]:
        """Return metadata for an authorized path."""
        return json.loads(self._client.stat_json(path))

    def read(self, path: str, *, offset: int = 0, max_bytes: int | None = None) -> bytes:
        """Read bytes. Omitted max_bytes requires the complete remaining file to fit.

        Explicit max_bytes requests a bounded range. Consult stat for total size
        when consuming a larger file in multiple ranges.
        """
        return self._client.read(path, offset=offset, max_bytes=max_bytes)

    def write(self, path: str, data: bytes) -> int:
        """Create one file without overwrite. Local parent directories must exist."""
        return self._client.write(path, data)

    def delete(self, path: str) -> None:
        """Delete one authorized file. Directory and recursive deletion are unavailable."""
        self._client.delete(path)

    def download_link(self, path: str, *, expires_seconds: int = 300) -> dict[str, Any]:
        """Issue an explicitly policy-authorized native S3 presigned GET request.

        The result is a bearer URL with any required request headers. Local
        storage does not support this operation. The library does not generate
        its own confirmation or substitute an anonymous hosted proxy link.
        """
        return json.loads(self._client.download_link_json(path, expires_seconds=expires_seconds))

    def drain_audit_events(self) -> dict[str, Any]:
        """Drain up to 256 sanitized events and report older overwritten event count."""
        return json.loads(self._client.drain_audit_json())

    def close(self) -> None:
        """Prevent new operations. Already running synchronous calls finish normally."""
        self._client.close()

    def __enter__(self) -> Client:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def __repr__(self) -> str:
        return repr(self._client)

    def __getstate__(self) -> None:
        raise TypeError("Infimount clients cannot be pickled")
