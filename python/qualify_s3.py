#!/usr/bin/env python3
"""Qualify the installed wheel against disposable, explicitly configured S3.

The server runs in this Python process. Successful Rust-backed HTTP operations
therefore also demonstrate that native I/O releases the GIL to the server thread.
No external account, ambient AWS credentials, or running hosted service is used.
"""
from __future__ import annotations

import importlib.util
import logging
import os
from pathlib import Path
import sys
import unittest
import uuid


def main() -> int:
    package_root = Path(__file__).resolve().parent
    # Executing a script in python/ must not import the source package instead
    # of the installed wheel. Only the tests directory is added by discovery.
    sys.path[:] = [
        entry for entry in sys.path
        if Path(entry or os.getcwd()).resolve() != package_root
    ]
    spec = importlib.util.find_spec("infimount")
    if spec is None or spec.origin is None:
        raise RuntimeError("Install the native wheel before running S3 qualification")
    if Path(spec.origin).resolve().is_relative_to(package_root):
        raise RuntimeError("S3 qualification requires the installed wheel")

    import boto3
    from botocore.exceptions import ClientError
    from moto.server import ThreadedMotoServer

    logging.getLogger("werkzeug").setLevel(logging.ERROR)
    server = ThreadedMotoServer(ip_address="127.0.0.1", port=0, verbose=False)
    server.start()
    try:
        host, port = server.get_host_and_port()
        if host != "127.0.0.1":
            raise RuntimeError("Qualification server must bind to loopback")
        endpoint = f"http://{host}:{port}"
        bucket = f"infimount-python-qualification-{uuid.uuid4().hex}"
        access = "synthetic-python-access"
        secret = "synthetic-python-secret"
        client = boto3.client(
            "s3", endpoint_url=endpoint, region_name="us-east-1",
            aws_access_key_id=access, aws_secret_access_key=secret,
        )
        client.create_bucket(Bucket=bucket)
        client.put_object(
            Bucket=bucket, Key="conditional-preflight", Body=b"sentinel", IfNoneMatch="*",
        )
        try:
            client.put_object(
                Bucket=bucket, Key="conditional-preflight", Body=b"wrong", IfNoneMatch="*",
            )
        except ClientError as error:
            if error.response["ResponseMetadata"]["HTTPStatusCode"] != 412:
                raise
        else:
            raise AssertionError("The emulator did not enforce conditional create")
        if client.get_object(Bucket=bucket, Key="conditional-preflight")["Body"].read() != b"sentinel":
            raise AssertionError("Conditional create changed an existing object")

        os.environ.update({
            "INFIMOUNT_PYTHON_TEST_S3_ENDPOINT": endpoint,
            "INFIMOUNT_PYTHON_TEST_S3_BUCKET": bucket,
            "INFIMOUNT_PYTHON_TEST_ACCESS": access,
            "INFIMOUNT_PYTHON_TEST_SECRET": secret,
            "AWS_EC2_METADATA_DISABLED": "true",
        })
        suite = unittest.defaultTestLoader.discover(str(package_root / "tests"), pattern="test_s3.py")
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        if result.skipped:
            raise AssertionError("S3 qualification must execute all configured cases")
        return 0 if result.wasSuccessful() else 1
    finally:
        server.stop()


if __name__ == "__main__":
    raise SystemExit(main())
