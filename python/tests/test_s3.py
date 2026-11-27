"""Actual OpenDAL S3 HTTP operations through the installed native extension.

Run with python/qualify_s3.py. Ordinary platform wheel tests skip this suite when
the isolated emulator fixture has not been configured.
"""
from __future__ import annotations

import copy
import ipaddress
import json
import os
from pathlib import Path
import tempfile
import unittest
from urllib.parse import parse_qs, urlsplit
from urllib.request import Request, urlopen
from unittest.mock import patch
import uuid

from infimount import (
    AlreadyExists, Client, ConfirmationRequired, InvalidRequest,
    LimitExceeded, NotFound, PermissionDenied,
)


@unittest.skipUnless(os.environ.get("INFIMOUNT_PYTHON_TEST_S3_ENDPOINT"), "run python/qualify_s3.py for isolated S3 qualification")
class NativeS3Tests(unittest.TestCase):
    def setUp(self) -> None:
        endpoint = os.environ["INFIMOUNT_PYTHON_TEST_S3_ENDPOINT"]
        parsed = urlsplit(endpoint)
        if parsed.scheme != "http" or not ipaddress.ip_address(parsed.hostname or "").is_loopback:
            raise AssertionError("Qualification must use the disposable loopback server")
        self.bucket = os.environ["INFIMOUNT_PYTHON_TEST_S3_BUCKET"]
        if not self.bucket.startswith("infimount-python-qualification-"):
            raise AssertionError("Qualification requires its own disposable bucket")
        self.prefix = f"allowed-{uuid.uuid4().hex}"
        self.path = f"/Artifacts/{self.prefix}"
        self.config = {
            "storage": {
                "backend": "s3", "name": "Artifacts", "bucket": self.bucket,
                "region": "us-east-1", "endpoint": endpoint,
                "access_key_id": {"kind": "env", "name": "INFIMOUNT_PYTHON_TEST_ACCESS"},
                "secret_access_key": {"kind": "env", "name": "INFIMOUNT_PYTHON_TEST_SECRET"},
            },
            "scopes": ["infimount:read", "infimount:create", "infimount:delete", "infimount:download"],
            "policy": {
                "version": 2, "default_access": "none",
                "rules": [{
                    "id": "qualification", "prefix": self.prefix, "access": "read_write",
                    "source": {"kind": "manual"},
                    "confirmation_rules": {
                        "require_for_write": False, "require_for_delete": False,
                        "require_for_presign": False,
                    },
                }],
                "denied_paths": [f"{self.prefix}/private"],
            },
        }
        import boto3
        self.provider = boto3.client(
            "s3", endpoint_url=endpoint, region_name="us-east-1",
            aws_access_key_id=os.environ["INFIMOUNT_PYTHON_TEST_ACCESS"],
            aws_secret_access_key=os.environ["INFIMOUNT_PYTHON_TEST_SECRET"],
        )
        self.clients: list[Client] = []
        self.client = self.make_client()

    def tearDown(self) -> None:
        for client in self.clients:
            client.close()

    def make_client(self, config: dict | None = None) -> Client:
        client = Client.from_config(config or self.config)
        self.clients.append(client)
        return client

    def test_binary_ranges_conditional_collision_and_native_presigned_get(self) -> None:
        payload = b"\x89PNG\r\n\x1a\n\x00\xff" + bytes(range(256)) * 9000
        path = f"{self.path}/image.png"
        self.assertEqual(self.client.write(path, payload), len(payload))
        with self.assertRaises(LimitExceeded):
            self.client.read(path)
        self.assertEqual(self.client.read(path, offset=37, max_bytes=4096), payload[37:4133])
        self.assertEqual(self.client.read(path, offset=len(payload)), b"")
        self.assertEqual(self.client.stat(path)["size"], len(payload))
        with self.assertRaises(AlreadyExists):
            self.client.write(path, b"must not replace")
        observed = self.provider.get_object(Bucket=self.bucket, Key=f"{self.prefix}/image.png")["Body"].read()
        self.assertEqual(observed, payload)
        link = self.client.download_link(path, expires_seconds=60)
        self.assertEqual(link["method"], "GET")
        self.assertEqual(link["expires_seconds"], 60)
        request = Request(link["url"], method=link["method"], headers=link["headers"])
        with urlopen(request, timeout=10) as response:
            self.assertEqual(response.read(4 * 1024 * 1024 + 1), payload)
        for ttl in [0, 59, 901]:
            with self.subTest(ttl=ttl), self.assertRaises(LimitExceeded):
                self.client.download_link(path, expires_seconds=ttl)
        events = json.dumps(self.client.drain_audit_events())
        for sensitive in [link["url"], "image.png", os.environ["INFIMOUNT_PYTHON_TEST_SECRET"]]:
            self.assertNotIn(sensitive, events)

    def test_filtered_pagination_and_file_only_deletion(self) -> None:
        expected = {f"item-{number:03}.bin" for number in range(31)}
        for name in expected:
            self.provider.put_object(Bucket=self.bucket, Key=f"{self.prefix}/{name}", Body=b"fixture")
        sentinel = f"{self.prefix}/private/sentinel.bin"
        self.provider.put_object(Bucket=self.bucket, Key=sentinel, Body=b"denied sentinel")
        observed = set()
        cursor = None
        first_cursor = None
        for _ in range(20):
            page = self.client.list(self.path, limit=4, cursor=cursor)
            observed.update(entry["name"] for entry in page["entries"])
            cursor = page["next_cursor"]
            first_cursor = first_cursor or cursor
            if cursor is None:
                break
        else:
            self.fail("S3 listing did not terminate")
        self.assertEqual(observed, expected)
        self.assertIsNotNone(first_cursor)
        with self.assertRaises(InvalidRequest):
            self.make_client().list(self.path, limit=4, cursor=first_cursor)
        with self.assertRaises(PermissionDenied):
            self.client.delete(f"{self.path}/private/sentinel.bin")
        # S3 has no materialized directory at this bare prefix. Stat must fail
        # before single-object delete, without trying recursive prefix removal.
        with self.assertRaises(NotFound):
            self.client.delete(self.path)
        retained = self.provider.list_objects_v2(Bucket=self.bucket, Prefix=f"{self.prefix}/")
        self.assertEqual(
            {entry["Key"] for entry in retained["Contents"]},
            {f"{self.prefix}/{name}" for name in expected} | {sentinel},
        )
        target = f"{self.path}/item-000.bin"
        self.client.delete(target)
        with self.assertRaises(NotFound):
            self.client.stat(target)
        self.assertEqual(self.provider.get_object(Bucket=self.bucket, Key=sentinel)["Body"].read(), b"denied sentinel")

    def test_scopes_confirmation_and_read_only_configuration_remain_enforced(self) -> None:
        self.client.write(f"{self.path}/original.bin", b"unchanged")
        config = copy.deepcopy(self.config)
        config["scopes"] = ["infimount:read"]
        reader = self.make_client(config)
        with self.assertRaises(PermissionDenied):
            reader.write(f"{self.path}/new.bin", b"denied")
        with self.assertRaises(PermissionDenied):
            reader.download_link(f"{self.path}/original.bin")
        config = copy.deepcopy(self.config)
        config["policy"]["rules"][0].pop("confirmation_rules")
        guarded = self.make_client(config)
        for operation in [
            lambda: guarded.write(f"{self.path}/new.bin", b"denied"),
            lambda: guarded.delete(f"{self.path}/original.bin"),
            lambda: guarded.download_link(f"{self.path}/original.bin"),
        ]:
            with self.assertRaises(ConfirmationRequired):
                operation()
        config = copy.deepcopy(self.config)
        config["storage"]["read_only"] = True
        with self.assertRaises(PermissionDenied):
            self.make_client(config).write(f"{self.path}/new.bin", b"denied")
        self.assertEqual(self.client.read(f"{self.path}/original.bin"), b"unchanged")
        with self.assertRaises(NotFound):
            self.client.stat(f"{self.path}/new.bin")

    def test_explicit_file_credentials_with_ambient_values_ignored(self) -> None:
        with tempfile.TemporaryDirectory(prefix="infimount-python-credentials-") as directory:
            root = Path(directory).resolve()
            access = root / "access-key"
            secret = root / "secret-key"
            access.write_text(os.environ["INFIMOUNT_PYTHON_TEST_ACCESS"] + "\n", encoding="utf-8")
            secret.write_text(os.environ["INFIMOUNT_PYTHON_TEST_SECRET"] + "\n", encoding="utf-8")
            config = copy.deepcopy(self.config)
            config["storage"]["access_key_id"] = {"kind": "file", "path": str(access)}
            config["storage"]["secret_access_key"] = {"kind": "file", "path": str(secret)}
            with patch.dict(os.environ, {"AWS_ACCESS_KEY_ID": "ambient-must-be-ignored", "AWS_SECRET_ACCESS_KEY": "ambient-must-be-ignored"}):
                client = self.make_client(config)
            client.write(f"{self.path}/file-credentials.bin", b"explicit")
            self.assertEqual(client.read(f"{self.path}/file-credentials.bin"), b"explicit")
            link = client.download_link(f"{self.path}/file-credentials.bin")
            credential = parse_qs(urlsplit(link["url"]).query)["X-Amz-Credential"][0]
            self.assertEqual(credential.split("/", 1)[0], os.environ["INFIMOUNT_PYTHON_TEST_ACCESS"])
            report = repr(client) + json.dumps(client.drain_audit_events())
            for sensitive in [str(access), str(secret), os.environ["INFIMOUNT_PYTHON_TEST_SECRET"]]:
                self.assertNotIn(sensitive, report)


if __name__ == "__main__":
    unittest.main()
