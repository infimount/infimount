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
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding, rsa
from cryptography.x509.oid import NameOID
from moto.server import ThreadedMotoServer


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
                                      transfer_bytes=16 * 1024 * 1024, principal_staging_bytes=16 * 1024 * 1024,
                                      staging_bytes=32 * 1024 * 1024, max_grants_per_principal=4, grant_ttl_seconds=30)
            if backend == "s3":
                moto = ThreadedMotoServer(ip_address="127.0.0.1", port=0, verbose=False)
                moto.start()
                host, port = moto.get_host_and_port()
                endpoint = f"http://{host}:{port}"
                bucket = "process-" + uuid.uuid4().hex
                s3 = boto3.client("s3", endpoint_url=endpoint, region_name="us-east-1",
                                  aws_access_key_id="synthetic-access-key", aws_secret_access_key=S3_SECRET)
                s3.create_bucket(Bucket=bucket)
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
            reserved = fixture.tool(alice, "create_upload", {"path": ALICE + "/reserved", "size": 16 * 1024 * 1024, "sha256": sha(b"x")})
            fixture.tool(alice, "create_upload", {"path": ALICE + "/quota", "size": 1, "sha256": sha(b"x")}, error=True)
            fixture.tool(alice, "abort_upload", {"id": reserved["upload_id"]})
            small = fixture.tool(alice, "create_upload", {"path": ALICE + "/quota-recovered", "size": 1, "sha256": sha(b"x")})
            fixture.tool(alice, "abort_upload", {"id": small["upload_id"]})
            link = fixture.tool(alice, "create_download", {"path": ALICE + "/chart.png", "ttl_seconds": 1})
            time.sleep(1.1)
            assert fixture.request("GET", link["url"], alice)[0] == 404
            link = fixture.tool(alice, "create_download", {"path": ALICE + "/chart.png"})
            status, headers, body = fixture.request("GET", link["url"], alice, headers={"Range": "bytes=1-7"})
            assert status == 206 and len(body) == 7 and headers["content-range"].startswith("bytes 1-7/")
            report.append(f"{backend}: checksum failure, abort, concurrent create-only race, quota recovery, expiry and ranges")

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
                cleanup.append(moto.stop)
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
                "fixture_scope": "disposable localhost TLS/OAuth, confined local filesystem and Moto S3, no provider qualification"}
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(evidence, indent=2) + "\n")
    print(f"Production process acceptance passed: {len(report)} scenario groups across two backends.", flush=True)


if __name__ == "__main__":
    main()
