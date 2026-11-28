#!/usr/bin/env python3
"""Exercise the production hosted executable or native image through HTTPS/OAuth.

All identities, keys, TLS certificates, storage and issuer/collector endpoints are
disposable fixtures. Certificate verification stays enabled. Requires the pinned
Moto environment used by qualify-hosted-data-plane.py. Docker mode is Linux-only
and tests the exact image with its default non-root user and a read-only root.
"""
from __future__ import annotations

import argparse
import base64
import contextlib
from concurrent.futures import ThreadPoolExecutor
import datetime
import hashlib
import http.client
import http.server
import io
import ipaddress
import json
import logging
import os
from pathlib import Path
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import uuid
import zipfile
import zlib

import boto3
from botocore.exceptions import ClientError
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding, rsa
from cryptography.x509.oid import NameOID
from moto.server import DomainDispatcherApplication, MotoRequestHandler, create_backend_app
from werkzeug.serving import make_server


ROOT = Path(__file__).resolve().parent.parent
SCOPES = "infimount:read infimount:create infimount:download"
ALICE = "/Artifacts/tenants/alpha/users/alice"
BOB = "/Artifacts/tenants/beta/users/bob"
ISSUER_SECRET = "synthetic-issuer-credential-never-log"
OTLP_SECRET = "synthetic-collector-credential-never-log"
S3_SECRET = "synthetic-s3-credential-never-log"
HOP_HEADERS = {"connection", "transfer-encoding", "content-length", "server", "date"}


def encoded(value: bytes) -> str:
    return base64.urlsafe_b64encode(value).rstrip(b"=").decode()


def compact(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":")).encode()


def sha(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def bounded_fixture_call(action, description, timeout=10):
    # Moto's start/event wait and stop/join have no built-in deadline. A failed
    # bind must fail this qualification instead of hanging the CI worker.
    failures = []

    def run():
        try:
            action()
        except BaseException as error:
            failures.append(error)

    worker = threading.Thread(target=run, daemon=True)
    worker.start()
    worker.join(timeout)
    assert not worker.is_alive(), f"{description} exceeded its fixture deadline"
    if failures:
        raise failures[0]


def receive_headers(connection):
    data = bytearray()
    while b"\r\n\r\n" not in data:
        chunk = connection.recv(4096)
        assert chunk, "direct listener closed before sending HTTP headers"
        data.extend(chunk)
        assert len(data) <= 16 * 1024, "direct listener sent excessive HTTP headers"
    head, prefix = bytes(data).split(b"\r\n\r\n", 1)
    lines = head.decode("iso-8859-1").split("\r\n")
    version, status, *_ = lines[0].split(" ")
    assert version == "HTTP/1.1"
    headers = dict(line.split(":", 1) for line in lines[1:])
    return int(status), {name.lower(): value.strip() for name, value in headers.items()}, prefix


def prove_staging_quota_recovered(fixture, token, timeout=10, grant_expiry=None):
    # All 24 MiB must be simultaneously available again, before the 30-second
    # grant deadline could hide a dropped-connection cleanup leak. Abort every
    # partial attempt so probing itself cannot exhaust staging capacity.
    deadline = time.monotonic() + timeout
    if grant_expiry is not None:
        deadline = min(deadline, time.monotonic() + grant_expiry - time.time() - 1)
    while True:
        reservations = []
        available = True
        try:
            for size in (16 * 1024 * 1024, 8 * 1024 * 1024):
                response = fixture.rpc(token, "tools/call", {"name": "create_upload", "arguments": {
                    "path": ALICE + "/quota-probe-" + uuid.uuid4().hex, "size": size, "sha256": sha(b"x")}})
                assert "error" not in response, "quota probe returned an unexpected JSON-RPC error"
                result = response["result"]
                if result.get("isError", False):
                    assert result["structuredContent"] == {"error": "limit_exceeded"}, "quota probe failed for another reason"
                    available = False
                    break
                reservations.append(result["structuredContent"])
            if available:
                denied = fixture.tool(token, "create_upload", {
                    "path": ALICE + "/quota-probe-overflow", "size": 1, "sha256": sha(b"x")}, error=True)
                assert denied == {"error": "limit_exceeded"}, "24 MiB did not exhaust the principal staging quota"
                assert time.monotonic() < deadline, "staging quota recovered after its cancellation deadline"
                assert grant_expiry is None or time.time() < grant_expiry, "grant expiry masked disconnect cleanup"
                return
        finally:
            for reservation in reservations:
                fixture.tool(token, "abort_upload", {"id": reservation["upload_id"]})
        assert time.monotonic() < deadline, "staging quota did not recover after disconnect"
        time.sleep(0.1)


class S3Fixture:
    def __init__(self):
        self.host, self.port = "127.0.0.1", free_port()
        self.app = DomainDispatcherApplication(create_backend_app)
        self.server = self.thread = None
        self.publications = []

    def dispatch(self, environ, start_response):
        query = urllib.parse.parse_qs(environ.get("QUERY_STRING", ""), keep_blank_values=True)
        method = environ["REQUEST_METHOD"]
        upload_id = query.get("uploadId", [None])[0]
        operation = ("complete_multipart_upload" if method == "POST" and upload_id
                     else "put_object" if method == "PUT" and not upload_id else None)
        if operation is None:
            return self.app(environ, start_response)
        # Capture only the conditional-publication contract, never authorization
        # headers, request bodies or credentials from signed requests.
        publication = {"operation": operation, "method": method, "key": environ["PATH_INFO"],
                       "upload_id": upload_id, "if_none_match": environ.get("HTTP_IF_NONE_MATCH"), "status": None}
        self.publications.append(publication)

        def capture(status, headers, exc_info=None):
            publication["status"] = int(status.split(" ", 1)[0])
            return start_response(status, headers, exc_info)

        return self.app(environ, capture)

    def start(self):
        assert self.server is None, "S3 fixture is already running"
        # Moto 5.2.6 is not concurrency-safe: its multipart existence check and
        # publication race in separate server threads. Serialize emulator HTTP
        # operations to model S3's atomic conditional-write primitive. Infimount
        # requests still compete, and controls below prove this fixture permits
        # overwrites unless the client actually sends the conditional header.
        self.server = make_server(self.host, self.port, self.dispatch, threaded=False,
                                  request_handler=MotoRequestHandler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def stop(self):
        if self.server is not None:
            self.server.shutdown()
            self.server.server_close()
            self.thread.join(timeout=5)
            assert not self.thread.is_alive(), "S3 fixture server thread did not stop"
            self.server = self.thread = None

    def get_host_and_port(self):
        return self.host, self.port


def qualify_s3_conditions(client, bucket):
    # Negative control: serving requests serially must not itself force a single
    # winner. Two unconditional multipart completions can overwrite each other.
    for conditional in (False, True):
        key = "fixture-conditional-control" if conditional else "fixture-unconditional-control"
        uploads = []
        payloads = (b"first-fixture-control", b"second-fixture-control")
        for payload in payloads:
            upload_id = client.create_multipart_upload(Bucket=bucket, Key=key)["UploadId"]
            part = client.upload_part(Bucket=bucket, Key=key, UploadId=upload_id, PartNumber=1, Body=payload)
            uploads.append({"Bucket": bucket, "Key": key, "UploadId": upload_id,
                            "MultipartUpload": {"Parts": [{"PartNumber": 1, "ETag": part["ETag"]}]}})
        first = client.complete_multipart_upload(**uploads[0], **({"IfNoneMatch": "*"} if conditional else {}))
        assert first["ResponseMetadata"]["HTTPStatusCode"] == 200
        if conditional:
            try:
                client.complete_multipart_upload(**uploads[1], IfNoneMatch="*")
            except ClientError as error:
                assert error.response["ResponseMetadata"]["HTTPStatusCode"] == 412
                assert error.response["Error"]["Code"] == "PreconditionFailed"
            else:
                raise AssertionError("S3 fixture ignored a multipart publication precondition")
            finally:
                client.abort_multipart_upload(Bucket=bucket, Key=key, UploadId=uploads[1]["UploadId"])
        else:
            second = client.complete_multipart_upload(**uploads[1])
            assert second["ResponseMetadata"]["HTTPStatusCode"] == 200, "S3 fixture incorrectly forbids unconditional overwrite"
        response = client.get_object(Bucket=bucket, Key=key)
        try:
            assert response["Body"].read() == payloads[0 if conditional else 1]
        finally:
            response["Body"].close()
        client.delete_object(Bucket=bucket, Key=key)
    assert not client.list_multipart_uploads(Bucket=bucket).get("Uploads"), "S3 fixture controls leaked multipart uploads"


def make_tls(directory: Path) -> tuple[Path, Path, Path]:
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Infimount disposable qualification CA")])
    now = datetime.datetime.now(datetime.timezone.utc)
    ca = (x509.CertificateBuilder().subject_name(name).issuer_name(name)
          .public_key(key.public_key()).serial_number(x509.random_serial_number())
          .not_valid_before(now - datetime.timedelta(minutes=5))
          .not_valid_after(now + datetime.timedelta(days=1))
          .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
          .add_extension(x509.KeyUsage(False, False, False, False, False, True, True, False, False), critical=True)
          .sign(key, hashes.SHA256()))
    leaf_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    leaf = (x509.CertificateBuilder()
            .subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "localhost")]))
            .issuer_name(name).public_key(leaf_key.public_key())
            .serial_number(x509.random_serial_number())
            .not_valid_before(now - datetime.timedelta(minutes=5))
            .not_valid_after(now + datetime.timedelta(days=1))
            .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
            .add_extension(x509.SubjectAlternativeName([
                x509.DNSName("localhost"), x509.IPAddress(ipaddress.ip_address("127.0.0.1"))]), critical=False)
            .add_extension(x509.ExtendedKeyUsage([x509.oid.ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
            .sign(key, hashes.SHA256()))
    ca_path, cert_path, key_path = (directory / name for name in ("ca.pem", "server.pem", "server.key"))
    ca_path.write_bytes(ca.public_bytes(serialization.Encoding.PEM))
    cert_path.write_bytes(leaf.public_bytes(serialization.Encoding.PEM))
    key_path.write_bytes(leaf_key.private_bytes(serialization.Encoding.PEM,
                         serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
    key_path.chmod(0o600)
    return ca_path, cert_path, key_path


class Fixture:
    def __init__(self, directory: Path):
        self.directory = directory
        self.ca, cert, key = make_tls(directory)
        self.keys = {name: rsa.generate_private_key(public_exponent=65537, key_size=2048)
                     for name in ("first", "rotated")}
        self.active_keys = ["first"]
        self.backend_port = free_port()
        self.issuer_mode = "ok"
        self.issuer_requests: list[dict] = []
        self.platform_tokens: dict[str, str] = {}
        self.metrics: list[dict] = []
        self.tokens: list[str] = []
        self.errors: list[str] = []
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def respond(self, status, body, headers=None):
                self.send_response(status)
                for name, value in (headers or {}).items():
                    if name.lower() not in HOP_HEADERS:
                        self.send_header(name, value)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                if self.command != "HEAD":
                    with contextlib.suppress(BrokenPipeError, ConnectionResetError):
                        self.wfile.write(body)

            def handle_request(self):
                body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                if self.path == "/jwks":
                    keys = []
                    for kid in fixture.active_keys:
                        numbers = fixture.keys[kid].public_key().public_numbers()
                        keys.append({"kty": "RSA", "kid": kid, "use": "sig", "alg": "RS256",
                                     "n": encoded(numbers.n.to_bytes((numbers.n.bit_length() + 7) // 8, "big")),
                                     "e": encoded(numbers.e.to_bytes((numbers.e.bit_length() + 7) // 8, "big"))})
                    self.respond(200, compact({"keys": keys}), {"Content-Type": "application/json"})
                    return
                if self.path == "/issue":
                    if self.headers.get("Authorization") != f"Bearer {ISSUER_SECRET}":
                        fixture.errors.append("issuer received incorrect service credential")
                        self.respond(401, b"{}")
                        return
                    request = json.loads(body)
                    fixture.issuer_requests.append(request)
                    expected = {"version", "grant_id", "redeem_url", "expires_at_unix_seconds",
                                "requires_authorization", "single_use"}
                    if set(request) != expected or request["requires_authorization"] is not True or request["single_use"] is not True:
                        fixture.errors.append("issuer request violated the allowlisted authority contract")
                    response = {"version": 1, "grant_id": request["grant_id"],
                                "url": f"{fixture.platform_origin}/platform/{request['grant_id']}",
                                "expires_at_unix_seconds": request["expires_at_unix_seconds"]}
                    if fixture.issuer_mode == "wrong_origin":
                        response["url"] = "https://untrusted.invalid/download"
                    self.respond(503 if fixture.issuer_mode == "failure" else 200,
                                 compact(response), {"Content-Type": "application/json"})
                    return
                if self.path == "/v1/metrics":
                    if self.headers.get("Authorization") != f"Bearer {OTLP_SECRET}":
                        fixture.errors.append("collector received incorrect service credential")
                    fixture.metrics.append(json.loads(body))
                    self.respond(200, b"{}", {"Content-Type": "application/json"})
                    return
                if self.path.startswith("/platform/"):
                    # Model the platform's own login session and server-side
                    # OAuth relay. The browser never supplies an Infimount token
                    # to this separate presentation origin.
                    if self.headers.get("Authorization"):
                        fixture.errors.append("Infimount OAuth token was sent to the platform origin")
                    token = fixture.platform_tokens.get(self.headers.get("Cookie", ""))
                    if token is None:
                        self.respond(401, b"{}")
                        return
                    grant_id = self.path.removeprefix("/platform/")
                    issued = next((row for row in fixture.issuer_requests if row["grant_id"] == grant_id), None)
                    if issued is None:
                        self.respond(404, b"{}")
                        return
                    status, headers, content = fixture.request("GET", issued["redeem_url"], token)
                    self.respond(status, content, headers)
                    return
                connection = http.client.HTTPConnection("127.0.0.1", fixture.backend_port, timeout=15)
                try:
                    connection.request(self.command, self.path, body=body,
                                       headers={k: v for k, v in self.headers.items()
                                                if k.lower() not in HOP_HEADERS})
                    response = connection.getresponse()
                    self.respond(response.status, response.read(), dict(response.getheaders()))
                except (OSError, http.client.HTTPException):
                    self.respond(502, b"{}")
                finally:
                    connection.close()

            do_GET = do_POST = do_PUT = do_HEAD = do_DELETE = handle_request

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cert, key)
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        self.origin = f"https://localhost:{self.server.server_port}"
        self.platform = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.platform.socket = context.wrap_socket(self.platform.socket, server_side=True)
        self.platform_origin = f"https://localhost:{self.platform.server_port}"
        self.context = ssl.create_default_context(cafile=str(self.ca))
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.platform_thread = threading.Thread(target=self.platform.serve_forever, daemon=True)
        self.platform_thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
        self.platform.shutdown()
        self.platform.server_close()
        self.platform_thread.join(timeout=5)

    def token(self, subject="alice", kid="first", **changes):
        now = int(time.time())
        claims = {"iss": self.origin + "/identity", "aud": self.origin + "/mcp", "sub": subject,
                  "tenant_id": "alpha" if subject == "alice" else "beta", "client_id": "artifact-agent",
                  "jti": uuid.uuid4().hex, "iat": now, "nbf": now, "exp": now + 300, "scope": SCOPES}
        claims.update(changes)
        message = (encoded(compact({"alg": "RS256", "typ": "at+jwt", "kid": kid}))
                   + "." + encoded(compact(claims))).encode()
        signature = self.keys[kid].sign(message, padding.PKCS1v15(), hashes.SHA256())
        token = message.decode() + "." + encoded(signature)
        self.tokens.append(token)
        return token

    def request(self, method, path, token=None, body=b"", headers=None):
        parsed = urllib.parse.urlsplit(path)
        port = self.server.server_port
        if parsed.netloc:
            origin = f"{parsed.scheme}://{parsed.netloc}"
            assert origin in (self.origin, self.platform_origin), "unexpected authority in artifact URL"
            if origin == self.platform_origin:
                assert token is None, "must use the platform's own login at its presentation origin"
            port = parsed.port
            path = parsed.path + ("?" + parsed.query if parsed.query else "")
        supplied = {"Accept": "application/json, text/event-stream", "MCP-Protocol-Version": "2025-06-18"}
        if token is not None:
            supplied["Authorization"] = "Bearer " + token
        supplied.update(headers or {})
        connection = http.client.HTTPSConnection("localhost", port, context=self.context, timeout=20)
        try:
            connection.request(method, path, body=body, headers=supplied)
            response = connection.getresponse()
            return response.status, {k.lower(): v for k, v in response.getheaders()}, response.read()
        finally:
            connection.close()

    def rpc(self, token, method, params):
        status, _, body = self.request("POST", "/mcp", token,
                                      compact({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
                                      {"Content-Type": "application/json"})
        assert status == 200, f"{method}: unexpected HTTP {status}"
        return json.loads(body)

    def tool(self, token, name, args, error=False):
        response = self.rpc(token, "tools/call", {"name": name, "arguments": args})
        failed = "error" in response or response.get("result", {}).get("isError", False)
        assert bool(failed) == error, f"{name}: expected {'denial' if error else 'success'}, got {str(response)[:500]}"
        return response.get("result", {}).get("structuredContent", response.get("error"))


class Service:
    def __init__(self, args, fixture, config, data):
        self.args, self.fixture, self.config, self.data = args, fixture, config, data
        self.path = fixture.directory / "config.json"
        self.process = None
        self.log = fixture.directory / "service.log"
        self.container = "infimount-qualification-" + uuid.uuid4().hex
        self.output = None

    def start(self):
        self.path.write_bytes(compact(self.config))
        environment = os.environ.copy()
        environment.update({"SSL_CERT_FILE": str(self.fixture.ca), "AWS_EC2_METADATA_DISABLED": "true",
                            "INFIMOUNT_QUALIFY_ACCESS_KEY": "synthetic-access-key",
                            "INFIMOUNT_QUALIFY_SECRET_KEY": S3_SECRET,
                            "INFIMOUNT_QUALIFY_ISSUER": ISSUER_SECRET, "INFIMOUNT_QUALIFY_OTLP": OTLP_SECRET,
                            "OTEL_EXPORTER_OTLP_ENDPOINT": self.fixture.origin + "/v1/metrics"})
        if self.args.binary:
            command = [str(Path(self.args.binary).resolve()), "serve", "--config", str(self.path)]
        else:
            command = ["docker", "run", "--rm", "--name", self.container, "--network=host",
                       "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges",
                       "--tmpfs", "/tmp:rw,nosuid,nodev,size=256m",
                       "--mount", f"type=bind,src={self.path},dst=/run/hosted.json,readonly",
                       "--mount", f"type=bind,src={self.fixture.ca},dst=/run/qualification-ca.pem,readonly",
                       "--mount", f"type=bind,src={self.data},dst=/data/artifacts",
                       "--env", "SSL_CERT_FILE=/run/qualification-ca.pem", "--env", "AWS_EC2_METADATA_DISABLED=true"]
            for name in ("INFIMOUNT_QUALIFY_ACCESS_KEY", "INFIMOUNT_QUALIFY_SECRET_KEY",
                         "INFIMOUNT_QUALIFY_ISSUER", "INFIMOUNT_QUALIFY_OTLP", "OTEL_EXPORTER_OTLP_ENDPOINT"):
                command.extend(["--env", name])
            command.extend([self.args.image, "serve", "--config", "/run/hosted.json"])
        self.output = self.log.open("ab")
        self.process = subprocess.Popen(command, env=environment, stdout=self.output, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 25
        while time.monotonic() < deadline:
            assert self.process.poll() is None, "production service exited during startup (see sanitized log below)"
            if self.fixture.request("GET", "/health/ready")[0] == 200:
                return
            time.sleep(0.1)
        raise AssertionError("production service did not become ready")

    def stop(self):
        if self.process is not None:
            if self.args.image:
                stopped = False
                try:
                    stopped = subprocess.run(["docker", "stop", "--time", "5", self.container],
                                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                             timeout=15, check=False).returncode == 0
                except subprocess.TimeoutExpired:
                    pass
                if not stopped:
                    subprocess.run(["docker", "rm", "--force", self.container],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15, check=False)
                    exists = subprocess.run(["docker", "container", "inspect", self.container],
                                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                            timeout=10, check=False).returncode == 0
                    assert not exists, "disposable qualification container could not be stopped"
            elif self.process.poll() is None:
                self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
            self.process = None
        if self.output:
            self.output.close()
            self.output = None

    def cleanup_image_staging(self):
        if self.args.image:
            # The production image intentionally owns a mode-0700 private
            # directory. Remove only that disposable fixture directory using
            # the same non-root UID before the host TemporaryDirectory cleanup.
            subprocess.run([
                "docker", "run", "--rm", "--network=none", "--read-only", "--cap-drop=ALL",
                "--security-opt=no-new-privileges", "--entrypoint", "/bin/sh",
                "--mount", f"type=bind,src={self.data},dst=/data/artifacts", self.args.image,
                "-c", "rm -rf -- /data/artifacts/.infimount-hosted-staging /data/artifacts/.infimount-hosted-stage",
            ], check=True, timeout=20)


def artifacts():
    # Real signatures/containers, including non-UTF-8 bytes. No fixture depends on a desktop app.
    def png_chunk(kind, data):
        return struct.pack("!I", len(data)) + kind + data + struct.pack("!I", zlib.crc32(kind + data) & 0xffffffff)

    png = (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack("!2I5B", 1, 1, 8, 6, 0, 0, 0))
           + png_chunk(b"IDAT", zlib.compress(b"\x00\xff\x00\x00\xff")) + png_chunk(b"IEND", b""))
    yield "chart.png", "image/png", png
    yield "chart.svg", "image/svg+xml", b'<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>'
    yield "report.csv", "text/csv", "item,value\n\u03b1,42\n".encode()
    pdf = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>", b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
               b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << >> /Contents 4 0 R >>",
               b"<< /Length 0 >>\nstream\n\nendstream"]
    for number, value in enumerate(objects, start=1):
        offsets.append(len(pdf))
        pdf.extend(f"{number} 0 obj\n".encode() + value + b"\nendobj\n")
    startxref = len(pdf)
    pdf.extend(b"xref\n0 5\n0000000000 65535 f \n")
    for offset in offsets[1:]:
        pdf.extend(f"{offset:010} 00000 n \n".encode())
    pdf.extend(f"trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{startxref}\n%%EOF\n".encode())
    yield "report.pdf", "application/pdf", bytes(pdf)
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w") as archive:
        archive.writestr("[Content_Types].xml", '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>')
        archive.writestr("_rels/.rels", '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>')
        archive.writestr("xl/workbook.xml", '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Report" sheetId="1" r:id="rId1"/></sheets></workbook>')
        archive.writestr("xl/_rels/workbook.xml.rels", '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>')
        archive.writestr("xl/worksheets/sheet1.xml", '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>42</v></c></row></sheetData></worksheet>')
    yield "report.xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", stream.getvalue()
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w", compression=zipfile.ZIP_STORED) as archive:
        archive.writestr("payload.bin", bytes(range(256)) * (9 * 1024 * 4))
    yield "large.zip", "application/zip", stream.getvalue()


def exercise(args, backend, report):
    with tempfile.TemporaryDirectory(prefix="infimount-process-") as temporary:
        directory = Path(temporary)
        directory.chmod(0o755)
        data = directory / "data"
        data.mkdir(mode=0o777)
        data.chmod(0o777)
        for prefix in (ALICE, BOB):
            path = data / prefix.removeprefix("/Artifacts/")
            path.mkdir(parents=True)
            for parent in [path, *path.parents]:
                if parent == directory:
                    break
                parent.chmod(0o777)
        fixture = Fixture(directory)
        service = None
        moto = None
        try:
            config = json.loads((ROOT / "deploy/hosted/example-local.json").read_text())
            config.update(public_url=fixture.origin, listen=f"127.0.0.1:{fixture.backend_port}", allow_non_loopback=False)
            config["auth"].update(issuer=fixture.origin + "/identity", audience=fixture.origin + "/mcp",
                                  jwks_url=fixture.origin + "/jwks", jwks_cache_seconds=30, jwks_refresh_min_seconds=5)
            for grant in config["grants"]:
                grant["issuer"] = fixture.origin + "/identity"
            record = config["storages"][0]["record"]
            record["config"]["root"] = "/data/artifacts" if args.image else str(data)
            config["limits"].update(inline_bytes=1024 * 1024, json_bytes=2 * 1024 * 1024,
                                      transfer_bytes=16 * 1024 * 1024, principal_staging_bytes=24 * 1024 * 1024,
                                      staging_bytes=32 * 1024 * 1024, max_grants_per_principal=4, grant_ttl_seconds=30)
            if backend == "s3":
                moto = S3Fixture()
                bounded_fixture_call(moto.start, "S3 fixture startup")
                host, port = moto.get_host_and_port()
                endpoint = f"http://{host}:{port}"
                bucket = "process-" + uuid.uuid4().hex
                s3 = boto3.client("s3", endpoint_url=endpoint, region_name="us-east-1",
                                  aws_access_key_id="synthetic-access-key", aws_secret_access_key=S3_SECRET)
                s3.create_bucket(Bucket=bucket)
                qualify_s3_conditions(s3, bucket)
                record.update(backend="s3", config={"bucket": bucket, "region": "us-east-1", "endpoint": endpoint})
                config["storages"][0].update(dedicated_local=False, credentials={
                    "accessKeyId": {"kind": "env", "name": "INFIMOUNT_QUALIFY_ACCESS_KEY"},
                    "secretAccessKey": {"kind": "env", "name": "INFIMOUNT_QUALIFY_SECRET_KEY"}})
                config["telemetry"] = {"mode": "otlp_http", "endpoint": fixture.origin + "/v1/metrics",
                                       "bearer_token": {"kind": "env", "name": "INFIMOUNT_QUALIFY_OTLP"},
                                       "interval_seconds": 5, "timeout_milliseconds": 1000}
            service = Service(args, fixture, config, data)
            service.start()
            alice, bob = fixture.token(), fixture.token("bob")
            fixture.platform_tokens = {"platform_session=alice": alice, "platform_session=bob": bob}
            initialized = fixture.rpc(alice, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                                           "clientInfo": {"name": "acceptance", "version": "1"}})
            assert initialized["result"]["serverInfo"]["name"] == "infimount-hosted"
            tools = fixture.rpc(alice, "tools/list", {})["result"]["tools"]
            assert {tool["name"] for tool in tools} == {"list_dir", "stat_path", "read_file", "write_file",
                                                       "create_download", "create_upload", "complete_upload", "abort_upload"}
            assert fixture.request("GET", "/health/live")[0] == 200
            metadata = json.loads(fixture.request("GET", "/.well-known/oauth-protected-resource/mcp")[2])
            assert metadata["resource"] == config["auth"]["audience"]
            for token in (None, "forged", fixture.token(aud="https://wrong.invalid/mcp"),
                          fixture.token(iss="https://wrong.invalid"), fixture.token(iat=int(time.time()) - 60,
                          nbf=int(time.time()) - 60, exp=int(time.time()) - 1)):
                for method, path in (("POST", "/mcp"), ("GET", "/downloads/fake"), ("PUT", "/uploads/fake")):
                    assert fixture.request(method, path, token)[0] == 401, "authentication did not cover every route"
            for headers in ({"Host": "attacker.invalid"}, {"Origin": "https://attacker.invalid"},
                            {"Mcp-Session-Id": "untrusted-session"}, {"Content-Encoding": "gzip"}):
                assert fixture.request("POST", "/mcp", alice, b"{}", headers)[0] >= 400
            fixture.tool(fixture.token(scope="infimount:read"), "write_file",
                         {"path": ALICE + "/scope-denied", "content": "x"}, error=True)
            report.append(f"{backend}: production startup, TLS, OAuth claims, metadata and exact tool allowlist")

            for name, mime, content in artifacts():
                path = ALICE + "/" + name
                if len(content) <= config["limits"]["inline_bytes"]:
                    arguments = {"path": path, "content": base64.b64encode(content).decode(), "encoding": "base64"}
                    if backend == "s3":
                        arguments["content_type"] = mime
                    fixture.tool(alice, "write_file", arguments)
                    if backend == "s3":
                        assert s3.head_object(Bucket=bucket, Key=path.removeprefix("/Artifacts/"))["ContentType"] == mime
                    read = fixture.tool(alice, "read_file", {"path": path})
                    assert base64.b64decode(read["content"], validate=True) == content
                    fixture.tool(alice, "write_file", arguments, error=True)
                else:
                    upload = fixture.tool(alice, "create_upload", {"path": path, "size": len(content), "sha256": sha(content)})
                    assert fixture.request("PUT", upload["url"], bob, b"x")[0] == 404
                    assert fixture.request("PUT", upload["url"], alice, content)[0] == 200
                    fixture.tool(bob, "complete_upload", {"id": upload["upload_id"]}, error=True)
                    fixture.tool(alice, "complete_upload", {"id": upload["upload_id"]})
                    fixture.tool(alice, "complete_upload", {"id": upload["upload_id"]}, error=True)
                    read = fixture.tool(alice, "read_file", {"path": path, "offset": 5000, "max_bytes": 8192})
                    assert base64.b64decode(read["content"], validate=True) == content[5000:13192]
                fixture.tool(alice, "stat_path", {"path": path})
                link = fixture.tool(alice, "create_download", {"path": path})
                assert link["delivery"] == "proxy" and link["single_use"] and link["requires_authorization"]
                assert link["sha256"] == sha(content)
                assert fixture.request("GET", link["url"], bob)[0] == 404
                status, headers, downloaded = fixture.request("GET", link["url"], alice)
                assert status == 200 and downloaded == content, "artifact download changed bytes"
                assert headers["content-type"] == "application/octet-stream" and headers["x-content-type-options"] == "nosniff"
                assert headers["cache-control"] == "private, no-store" and headers["content-disposition"].startswith("attachment;")
                assert "location" not in headers and fixture.request("GET", link["url"], alice)[0] == 404
            report.append(f"{backend}: PNG, SVG, CSV, PDF, XLSX and 9 MiB ZIP save/read/download, MIME and replay")

            fixture.tool(bob, "write_file", {"path": BOB + "/own.bin", "content": "bob-only"})
            for actor, path in ((bob, ALICE + "/chart.png"), (alice, BOB + "/own.bin"),
                                (fixture.token(tenant_id="beta"), ALICE + "/chart.png"),
                                (fixture.token(client_id="other-agent"), ALICE + "/chart.png")):
                for name in ("read_file", "stat_path", "create_download"):
                    fixture.tool(actor, name, {"path": path}, error=True)
                fixture.tool(actor, "list_dir", {"path": path.rsplit("/", 1)[0]}, error=True)
                fixture.tool(actor, "write_file", {"path": path + ".new", "content": "denied"}, error=True)
                fixture.tool(actor, "create_upload", {"path": path + ".new", "size": 1, "sha256": sha(b"x")}, error=True)
            page = fixture.tool(alice, "list_dir", {"path": ALICE, "limit": 1})
            cursor = page["next_cursor"]
            assert cursor
            fixture.tool(bob, "list_dir", {"path": BOB, "limit": 1, "cursor": cursor}, error=True)
            fixture.tool(alice, "list_dir", {"path": ALICE, "limit": 1, "cursor": cursor})
            fixture.tool(alice, "list_dir", {"path": ALICE, "limit": 1, "cursor": cursor}, error=True)
            for suffix in ("/../bob/own.bin", "/%2e%2e/secret", "/.infimount-hosted-staging/x", "/a\\b"):
                fixture.tool(alice, "write_file", {"path": ALICE + suffix, "content": "denied"}, error=True)
            fixture.tool(alice, "write_file", {"path": ALICE + "/bad-base64", "content": "!!!!", "encoding": "base64"}, error=True)
            fixture.tool(alice, "write_file", {"path": ALICE + "/unknown", "content": "denied", "overwrite": True}, error=True)
            for name in ("delete_file", "move_file", "copy_file", "add_storage"):
                fixture.tool(alice, name, {"path": ALICE + "/chart.png"}, error=True)
            # Send only the oversized length to the real upstream listener.
            # A rejection before any body arrives proves the pre-parser bound
            # without making this buffering TLS fixture race an early close.
            connection = http.client.HTTPConnection("127.0.0.1", fixture.backend_port, timeout=3)
            try:
                connection.putrequest("POST", "/mcp", skip_host=True)
                connection.putheader("Host", urllib.parse.urlsplit(fixture.origin).netloc)
                connection.putheader("Authorization", "Bearer " + alice)
                connection.putheader("Content-Type", "application/json")
                connection.putheader("Content-Length", str(config["limits"]["json_bytes"] + 1))
                connection.endheaders()
                assert connection.getresponse().status == 413
            finally:
                connection.close()
            fixture.tool(alice, "write_file", {"path": ALICE + "/oversize", "content": "x" * (1024 * 1024 + 1)}, error=True)
            fixture.tool(alice, "stat_path", {"path": ALICE + "/oversize"}, error=True)
            if backend == "local":
                outside = data / "outside-secret"
                outside.write_text("must-never-escape")
                outside.chmod(0o644)
                link_path = data / ALICE.removeprefix("/Artifacts/") / "link"
                link_path.symlink_to(os.path.relpath(outside, link_path.parent))
                fixture.tool(alice, "read_file", {"path": ALICE + "/link"}, error=True)
                fixture.tool(alice, "write_file", {"path": ALICE + "/mime-unsupported", "content": "x", "content_type": "text/plain"}, error=True)
                fixture.tool(alice, "stat_path", {"path": ALICE + "/mime-unsupported"}, error=True)
            report.append(f"{backend}: two-tenant isolation across tools/cursors/grants, confinement and fail-closed limits")

            upload = fixture.tool(alice, "create_upload", {"path": ALICE + "/bad-checksum", "size": 4, "sha256": sha(b"good")})
            assert fixture.request("PUT", upload["url"], alice, b"evil")[0] >= 400
            fixture.tool(alice, "complete_upload", {"id": upload["upload_id"]}, error=True)
            fixture.tool(alice, "stat_path", {"path": ALICE + "/bad-checksum"}, error=True)
            upload = fixture.tool(alice, "create_upload", {"path": ALICE + "/aborted", "size": 1, "sha256": sha(b"x")})
            fixture.tool(bob, "abort_upload", {"id": upload["upload_id"]}, error=True)
            fixture.tool(alice, "abort_upload", {"id": upload["upload_id"]})
            assert fixture.request("PUT", upload["url"], alice, b"x")[0] == 404
            fixture.tool(alice, "stat_path", {"path": ALICE + "/aborted"}, error=True)
            race_payloads = (b"first-writer-exact-bytes", b"second-writer-exact-bytes")
            race_path = ALICE + "/concurrent.bin"
            race_uploads = [fixture.tool(alice, "create_upload", {"path": race_path, "size": len(payload), "sha256": sha(payload)})
                            for payload in race_payloads]
            for upload, payload in zip(race_uploads, race_payloads):
                assert fixture.request("PUT", upload["url"], alice, payload)[0] == 200
            barrier = threading.Barrier(2, timeout=5)

            def finish(upload):
                barrier.wait()
                return fixture.rpc(alice, "tools/call", {"name": "complete_upload", "arguments": {"id": upload["upload_id"]}})

            with ThreadPoolExecutor(max_workers=2) as executor:
                completed = list(executor.map(finish, race_uploads))
            winners = [index for index, response in enumerate(completed)
                       if "error" not in response and not response.get("result", {}).get("isError", False)]
            assert len(winners) == 1, "two concurrent create-only uploads must have exactly one winner"
            read = fixture.tool(alice, "read_file", {"path": race_path})
            assert base64.b64decode(read["content"], validate=True) == race_payloads[winners[0]]
            prove_staging_quota_recovered(fixture, alice)
            link = fixture.tool(alice, "create_download", {"path": ALICE + "/chart.png", "ttl_seconds": 1})
            time.sleep(1.1)
            assert fixture.request("GET", link["url"], alice)[0] == 404
            link = fixture.tool(alice, "create_download", {"path": ALICE + "/chart.png"})
            status, headers, body = fixture.request("GET", link["url"], alice, headers={"Range": "bytes=1-7"})
            assert status == 206 and len(body) == 7 and headers["content-range"].startswith("bytes 1-7/")
            report.append(f"{backend}: checksum failure, abort, concurrent create-only race, quota recovery, expiry and ranges")

            large_payloads = (bytes(range(256)) * (9 * 1024 * 4) + b"first-large-writer",
                              bytes(reversed(range(256))) * (9 * 1024 * 4) + b"second-large-writer")
            large_path = ALICE + "/concurrent-large.bin"
            large_uploads = [fixture.tool(alice, "create_upload", {
                "path": large_path, "size": len(payload), "sha256": sha(payload)}) for payload in large_payloads]
            for upload, payload in zip(large_uploads, large_payloads):
                assert len(payload) > 9 * 1024 * 1024
                assert fixture.request("PUT", upload["url"], alice, payload)[0] == 200
            barrier = threading.Barrier(2, timeout=5)
            with ThreadPoolExecutor(max_workers=2) as executor:
                completed = list(executor.map(finish, large_uploads))
            winners = [index for index, response in enumerate(completed)
                       if "error" not in response and not response.get("result", {}).get("isError", False)]
            assert len(winners) == 1, "concurrent large create-only uploads must have exactly one winner"
            loser = completed[1 - winners[0]]
            assert loser.get("result", {}).get("structuredContent") == {"error": "conflict"}, "large race lost for an unrelated reason"
            winner = large_payloads[winners[0]]
            link = fixture.tool(alice, "create_download", {"path": large_path})
            assert link["sha256"] == sha(winner)
            status, _, downloaded = fixture.request("GET", link["url"], alice)
            assert status == 200 and downloaded == winner, "large upload race changed the winning object's bytes"
            if backend == "s3":
                completions = [item for item in moto.publications if item["operation"] == "complete_multipart_upload"
                               and item["key"] == f"/{bucket}/{large_path.removeprefix('/Artifacts/')}"]
                assert len(completions) == 2 and len({item["upload_id"] for item in completions}) == 2, "both S3 uploads did not reach multipart completion"
                assert all(item["if_none_match"] == "*" for item in completions), "production multipart completion omitted If-None-Match:*"
                assert sorted(item["status"] for item in completions) == [200, 412], "S3 conditional multipart completion did not reject the loser"
                etag = s3.head_object(Bucket=bucket, Key=large_path.removeprefix("/Artifacts/"))["ETag"].strip('"')
                assert "-" in etag and int(etag.rsplit("-", 1)[1]) >= 2, "large S3 race did not exercise multipart completion"
                assert not s3.list_multipart_uploads(Bucket=bucket).get("Uploads"), "large S3 race leaked an unfinished multipart upload"
                report.append("s3: competing uploads above 9 MiB, recorded conditional headers and 412, exact winner bytes, no unfinished multipart uploads and fixture overwrite controls")
            else:
                report.append("local: competing uploads above 9 MiB, one conflict and one exact-byte winner")

            # The TLS test proxy deliberately buffers, so cancellation must hit
            # the real listener. Host and OAuth validation remain unchanged.
            authority = urllib.parse.urlsplit(fixture.origin).netloc
            partial_path = ALICE + "/disconnected-upload.bin"
            upload = fixture.tool(alice, "create_upload", {
                "path": partial_path, "size": len(winner), "sha256": sha(winner)})
            with socket.create_connection(("127.0.0.1", fixture.backend_port), timeout=5) as connection:
                request = (f"PUT {urllib.parse.urlsplit(upload['url']).path} HTTP/1.1\r\n"
                           f"Host: {authority}\r\nAuthorization: Bearer {alice}\r\n"
                           f"Content-Length: {len(winner)}\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n")
                connection.sendall(request.encode("ascii"))
                status, _, prefix = receive_headers(connection)
                assert status == 100 and not prefix, "partial PUT was not accepted before disconnect"
                connection.sendall(winner[:64 * 1024])
                denied = fixture.tool(alice, "create_upload", {
                    "path": ALICE + "/held-upload-quota", "size": 16 * 1024 * 1024, "sha256": sha(b"x")}, error=True)
                assert denied == {"error": "limit_exceeded"}, "partial PUT did not retain its staging reservation"
                assert time.time() + 2 < upload["expires_at_unix_seconds"], "partial PUT grant expired before disconnect"
            prove_staging_quota_recovered(fixture, alice, grant_expiry=upload["expires_at_unix_seconds"])
            missing = fixture.tool(alice, "stat_path", {"path": partial_path}, error=True)
            assert missing == {"error": "not_found"}, "disconnected upload published a destination"
            fixture.tool(alice, "complete_upload", {"id": upload["upload_id"]}, error=True)
            assert fixture.request("PUT", upload["url"], alice, b"x")[0] == 404
            assert time.time() < upload["expires_at_unix_seconds"], "expiry masked the disconnected PUT replay denial"

            link = fixture.tool(alice, "create_download", {"path": large_path})
            with socket.socket() as connection:
                # Keep the receive window small enough that the full >9 MiB
                # response cannot drain into kernel buffers before we close it.
                connection.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4096)
                connection.settimeout(5)
                connection.connect(("127.0.0.1", fixture.backend_port))
                request = (f"GET {urllib.parse.urlsplit(link['url']).path} HTTP/1.1\r\n"
                           f"Host: {authority}\r\nAuthorization: Bearer {alice}\r\nConnection: close\r\n\r\n")
                connection.sendall(request.encode("ascii"))
                status, headers, prefix = receive_headers(connection)
                assert status == 200 and int(headers["content-length"]) == len(winner)
                while len(prefix) < 8192:
                    chunk = connection.recv(8192 - len(prefix))
                    assert chunk, "large download ended before its prefix"
                    prefix += chunk
                assert prefix == winner[:len(prefix)] and len(prefix) < len(winner)
                denied = fixture.tool(alice, "create_upload", {
                    "path": ALICE + "/held-download-quota", "size": 16 * 1024 * 1024, "sha256": sha(b"x")}, error=True)
                assert denied == {"error": "limit_exceeded"}, "large GET completed before the intended disconnect"
                assert time.time() + 2 < link["expires_at_unix_seconds"], "large GET grant expired before disconnect"
            prove_staging_quota_recovered(fixture, alice, grant_expiry=link["expires_at_unix_seconds"])
            assert fixture.request("GET", link["url"], alice)[0] == 404
            assert time.time() < link["expires_at_unix_seconds"], "expiry masked the disconnected GET replay denial"
            report.append(f"{backend}: direct-listener partial PUT/GET disconnects, full quota recovery, no partial destination or grant replay")

            if backend == "s3":
                process = service.process
                original_endpoint = moto.get_host_and_port()

                def await_readiness(expected):
                    deadline = time.monotonic() + 15
                    while True:
                        assert service.process is process and process.poll() is None, "service restarted or exited during S3 outage"
                        assert fixture.request("GET", "/health/live")[0] == 200, "storage outage changed process liveness"
                        # Keep the independent JWKS cache live so this check
                        # isolates the stopped S3 dependency, not token refresh.
                        fixture.rpc(alice, "tools/list", {})
                        actual = fixture.request("GET", "/health/ready")[0]
                        assert actual in (200, 503), f"unexpected readiness HTTP {actual}"
                        assert time.monotonic() < deadline, f"readiness did not become {expected} within its bound"
                        if actual == expected:
                            return
                        time.sleep(0.2)

                await_readiness(200)
                try:
                    bounded_fixture_call(moto.stop, "S3 fixture outage")
                    await_readiness(503)
                finally:
                    bounded_fixture_call(moto.start, "S3 fixture recovery")
                assert moto.get_host_and_port() == original_endpoint, "S3 fixture changed its configured endpoint"
                await_readiness(200)
                read = fixture.tool(alice, "read_file", {"path": ALICE + "/chart.png"})
                assert base64.b64decode(read["content"], validate=True) == next(artifacts())[2], "S3 recovery lost the stored artifact"
                report.append("s3: same-process backend outage, bounded readiness 503/200 recovery, live 200 and preserved artifact bytes")

            fixture.active_keys.append("rotated")
            rotated = fixture.token(kid="rotated")
            deadline = time.monotonic() + 8
            while fixture.request("POST", "/mcp", rotated, b"{}", {"Content-Type": "application/json"})[0] == 401:
                assert time.monotonic() < deadline, "rotated JWKS key was not accepted after refresh interval"
                time.sleep(0.2)
            fixture.tool(rotated, "read_file", {"path": ALICE + "/chart.png"})
            fixture.active_keys = ["rotated"]
            deadline = time.monotonic() + 35
            while fixture.request("POST", "/mcp", alice, b"{}", {"Content-Type": "application/json"})[0] != 401:
                assert time.monotonic() < deadline, "retired signing key remained valid after JWKS refresh"
                time.sleep(0.2)
            alice, bob = fixture.token(kid="rotated"), fixture.token("bob", kid="rotated")
            fixture.platform_tokens = {"platform_session=alice": alice, "platform_session=bob": bob}
            fixture.tool(alice, "read_file", {"path": ALICE + "/chart.png"})
            if backend == "s3":
                deadline = time.monotonic() + 7
                while not fixture.metrics:
                    assert time.monotonic() < deadline, "explicit OTLP exporter did not emit metrics"
                    time.sleep(0.1)
                payload = json.dumps(fixture.metrics)
                assert "infimount.hosted" in payload
                for secret in (alice, bob, ISSUER_SECRET, OTLP_SECRET, S3_SECRET, "chart.png", "tenants/alpha"):
                    assert secret not in payload, "OTLP included unauthorized high-cardinality data"
            else:
                assert fixture.metrics == [], "disabled telemetry followed ambient OTEL configuration"
            link = fixture.tool(alice, "create_download", {"path": ALICE + "/chart.png"})
            pending = fixture.tool(alice, "create_upload", {"path": ALICE + "/restart", "size": 1, "sha256": sha(b"x")})
            service.stop()
            service.start()
            assert fixture.request("GET", link["url"], alice)[0] == 404
            assert fixture.request("PUT", pending["url"], alice, b"x")[0] == 404
            fixture.tool(alice, "read_file", {"path": ALICE + "/chart.png"})
            service.stop()
            record["revision"] += 1
            record["mcp_policy"]["denied_paths"] = [ALICE.removeprefix("/Artifacts/")]
            service.start()
            fixture.tool(alice, "read_file", {"path": ALICE + "/chart.png"}, error=True)
            fixture.tool(bob, "read_file", {"path": BOB + "/own.bin"})
            report.append(f"{backend}: JWKS addition and retirement, explicit telemetry, restart invalidation and policy revocation")

            service.stop()
            record["mcp_policy"]["denied_paths"] = []
            config["delivery"] = {"mode": "delegated", "issuer_url": fixture.origin + "/issue",
                                  "issuer_token": {"kind": "env", "name": "INFIMOUNT_QUALIFY_ISSUER"},
                                  "returned_url_origin": fixture.platform_origin, "timeout_milliseconds": 1000}
            service.start()
            for mode in ("failure", "wrong_origin"):
                fixture.issuer_mode = mode
                fixture.tool(alice, "create_download", {"path": ALICE + "/chart.png"}, error=True)
                last = fixture.issuer_requests[-1]
                assert fixture.request("GET", last["redeem_url"], alice)[0] == 404, "failed issuer published a live grant"
            fixture.issuer_mode = "ok"
            delegated = fixture.tool(alice, "create_download", {"path": ALICE + "/chart.png"})
            assert delegated["delivery"] == "delegated" and "/platform/" in delegated["url"]
            assert fixture.request("GET", delegated["redeem_url"])[0] == 401
            assert fixture.request("GET", delegated["redeem_url"], bob)[0] == 404
            assert fixture.request("GET", delegated["url"])[0] == 401
            assert fixture.request("GET", delegated["url"], headers={"Cookie": "platform_session=bob"})[0] == 404
            status, _, downloaded = fixture.request("GET", delegated["url"], headers={"Cookie": "platform_session=alice"})
            assert status == 200 and downloaded == next(artifacts())[2]
            assert fixture.request("GET", delegated["redeem_url"], alice)[0] == 404
            assert not fixture.errors, "; ".join(fixture.errors)
            service.stop()
            logs = service.log.read_text(errors="replace")
            for secret in (*fixture.tokens, ISSUER_SECRET, OTLP_SECRET, S3_SECRET, "chart.png", "tenants/alpha", "bob-only"):
                assert secret not in logs, "service log disclosed a token, credential, raw path or artifact content"
            audit = [json.loads(row["fields"]["event"]) for line in logs.splitlines()
                     if (row := json.loads(line)).get("target") == "infimount_hosted::audit"]
            assert audit and {row["decision"] for row in audit} == {"allowed", "denied"}
            assert all(set(row) == {"event_id", "timestamp_unix_seconds", "operation", "issuer", "subject",
                                    "tenant", "client_id", "decision", "error_code", "path_sha256", "bytes"}
                       for row in audit), "production audit schema contained unexpected data"
            report.append(f"{backend}: delegated issuer contract, failure cleanup, owner-only redemption and audit redaction")
        except BaseException:
            if service:
                if service.log.exists():
                    logs = service.log.read_text(errors="replace")[-5000:]
                    for secret in (*fixture.tokens, ISSUER_SECRET, OTLP_SECRET, S3_SECRET):
                        logs = logs.replace(secret, "[redacted]")
                    print(logs, flush=True)
            raise
        finally:
            cleanup = [service.stop, service.cleanup_image_staging] if service else []
            cleanup.append(fixture.close)
            if moto:
                cleanup.append(lambda: bounded_fixture_call(moto.stop, "S3 fixture cleanup"))
            failures = []
            for action in cleanup:
                try:
                    action()
                except Exception as error:
                    failures.append(error)
            if failures:
                if original := sys.exception():
                    original.add_note("Fixture cleanup also failed: " + ", ".join(type(error).__name__ for error in failures))
                else:
                    raise ExceptionGroup("Disposable fixture cleanup failed", failures)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument("--binary", help="production infimount-hosted executable")
    target.add_argument("--image", help="already-built native image, no rebuild or publication")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    logging.getLogger("werkzeug").setLevel(logging.ERROR)
    report = []
    for backend in ("local", "s3"):
        first = len(report)
        exercise(args, backend, report)
        for line in report[first:]:
            print("PASS " + line, flush=True)
    evidence = {"target": "native-image" if args.image else "production-binary", "passed": report,
                "fixture_scope": "disposable localhost TLS/OAuth, confined local filesystem and Moto S3 with serialized emulator HTTP operations because Moto is not concurrency-safe, recorded production preconditions and overwrite controls, no provider qualification"}
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(evidence, indent=2) + "\n")
    print(f"Production process acceptance passed: {len(report)} scenario groups across two backends.", flush=True)


if __name__ == "__main__":
    main()
