#!/usr/bin/env python3
"""Qualify against a disposable S3 emulator. No cloud credentials or requests."""
import os
from pathlib import Path
import subprocess
import uuid

import boto3
from botocore.exceptions import ClientError
from moto.server import ThreadedMotoServer


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    server = ThreadedMotoServer(ip_address="127.0.0.1", port=0, verbose=False)
    server.start()
    host, port = server.get_host_and_port()
    assert host == "127.0.0.1"
    endpoint = f"http://{host}:{port}"
    bucket = f"infimount-test-{uuid.uuid4().hex}"
    try:
        client = boto3.client(
            "s3", endpoint_url=endpoint, region_name="us-east-1",
            aws_access_key_id="synthetic-test-key",
            aws_secret_access_key="synthetic-test-secret",
        )
        client.create_bucket(Bucket=bucket)
        client.put_object(Bucket=bucket, Key="conditional-preflight", Body=b"sentinel", IfNoneMatch="*")
        try:
            client.put_object(Bucket=bucket, Key="conditional-preflight", Body=b"wrong", IfNoneMatch="*")
        except ClientError as error:
            assert error.response["ResponseMetadata"]["HTTPStatusCode"] == 412
        else:
            raise AssertionError("emulator did not enforce conditional create")
        assert client.get_object(Bucket=bucket, Key="conditional-preflight")["Body"].read() == b"sentinel"
        environment = os.environ.copy()
        environment["INFIMOUNT_HOSTED_TEST_S3_ENDPOINT"] = endpoint
        environment["INFIMOUNT_HOSTED_TEST_S3_BUCKET"] = bucket
        environment["AWS_EC2_METADATA_DISABLED"] = "true"
        return subprocess.call(
            ["cargo", "+1.94.0", "test", "-p", "infimount_hosted", "--locked",
             "--test", "s3_qualification", "--", "--ignored", "--nocapture"],
            cwd=root, env=environment,
        )
    finally:
        server.stop()


if __name__ == "__main__":
    raise SystemExit(main())
