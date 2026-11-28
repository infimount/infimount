"""Exercise the installed native wheel, without mocking policy or storage I/O."""
from __future__ import annotations

import base64
import copy
import json
import os
from pathlib import Path
import pickle
import select
import signal
import sys
import tempfile
import unittest
import unicodedata
import warnings
from unittest.mock import patch

from infimount import (
    AlreadyExists, Client, ConfigurationError, ConfirmationRequired,
    InfimountError, InvalidRequest, LimitExceeded, NotFound, PermissionDenied,
    UnsupportedOperation,
)


def local_config(root: Path, *, scopes: list[str] | None = None) -> dict:
    return {
        "storage": {"backend": "local", "name": "Artifacts", "root": str(root.resolve())},
        "scopes": scopes if scopes is not None else ["infimount:read", "infimount:create"],
        "policy": {
            "version": 2,
            "default_access": "none",
            "rules": [{
                "id": "application-artifacts", "prefix": "allowed", "access": "read_write",
                "source": {"kind": "manual"},
                "confirmation_rules": {"require_for_write": False},
            }],
            "denied_paths": ["allowed/private"],
        },
    }


class NativeWheelTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="infimount-python-")
        self.root = Path(self.temp.name).resolve()
        (self.root / "allowed").mkdir()
        (self.root / "allowed/private").mkdir()
        (self.root / "allowed/private/sentinel.bin").write_bytes(b"private sentinel")
        (self.root / "outside").mkdir()
        (self.root / "outside/sentinel.bin").write_bytes(b"outside sentinel")
        self.config = local_config(self.root)
        self.clients: list[Client] = []
        self.client = self.make_client()

    def tearDown(self) -> None:
        for client in self.clients:
            client.close()
        self.clients.clear()
        self.temp.cleanup()

    def make_client(self, config: dict | None = None) -> Client:
        client = Client.from_config(config or self.config)
        self.clients.append(client)
        return client

    def test_binary_round_trip_and_create_only_collision(self) -> None:
        payload = b"\x89PNG\r\n\x1a\n" + bytes(range(256)) + b"\xff\x00\x80"
        self.assertEqual(self.client.write("/Artifacts/allowed/image.png", payload), len(payload))
        self.assertEqual(self.client.read("/Artifacts/allowed/image.png"), payload)
        self.assertEqual((self.root / "allowed/image.png").read_bytes(), payload)
        self.assertEqual(self.client.stat("/Artifacts/allowed/image.png")["size"], len(payload))
        with self.assertRaises(AlreadyExists):
            self.client.write("/Artifacts/allowed/image.png", b"replacement")
        self.assertEqual((self.root / "allowed/image.png").read_bytes(), payload)
        with self.assertRaises(TypeError):
            self.client.write("/Artifacts/allowed/text.txt", "not bytes")
        self.assertFalse((self.root / "allowed/text.txt").exists())

    def test_scopes_and_policy_are_independent_and_immutable(self) -> None:
        config = local_config(self.root, scopes=["infimount:read"])
        read_only = self.make_client(config)
        config["scopes"].append("infimount:create")
        with self.assertRaises(PermissionDenied):
            read_only.write("/Artifacts/allowed/new.bin", b"denied")
        for path in ["/Artifacts/outside/sentinel.bin", "/Artifacts/allowed/private/sentinel.bin", "/Other/allowed/file"]:
            with self.subTest(path=path), self.assertRaises(PermissionDenied):
                self.client.read(path)
        self.assertFalse((self.root / "allowed/new.bin").exists())
        denied = self.make_client(local_config(self.root, scopes=[]))
        with self.assertRaises(PermissionDenied):
            denied.list("/Artifacts/allowed")

    def test_confirmation_cannot_be_self_issued(self) -> None:
        config = copy.deepcopy(self.config)
        config["policy"]["rules"][0].pop("confirmation_rules")
        client = self.make_client(config)
        with self.assertRaises(ConfirmationRequired):
            client.write("/Artifacts/allowed/needs-review.bin", b"denied")
        with self.assertRaises(TypeError):
            client.write("/Artifacts/allowed/needs-review.bin", b"denied", confirmation_id="self-approved")
        self.assertFalse((self.root / "allowed/needs-review.bin").exists())

    def test_limits_and_ranges_never_silently_truncate_whole_reads(self) -> None:
        config = copy.deepcopy(self.config)
        config["limits"] = {"read_bytes": 16, "write_bytes": 32}
        client = self.make_client(config)
        payload = bytes(range(32))
        client.write("/Artifacts/allowed/ranges.bin", payload)
        with self.assertRaises(LimitExceeded):
            client.read("/Artifacts/allowed/ranges.bin")
        self.assertEqual(client.read("/Artifacts/allowed/ranges.bin", offset=8, max_bytes=16), payload[8:24])
        self.assertEqual(client.read("/Artifacts/allowed/ranges.bin", offset=32), b"")
        for options in [{"max_bytes": 17}, {"max_bytes": 0}]:
            with self.subTest(options=options), self.assertRaises(LimitExceeded):
                client.read("/Artifacts/allowed/ranges.bin", **options)
        with self.assertRaises(InvalidRequest):
            client.read("/Artifacts/allowed/ranges.bin", offset=33)
        with self.assertRaises(LimitExceeded):
            client.write("/Artifacts/allowed/too-large.bin", b"x" * 33)
        self.assertFalse((self.root / "allowed/too-large.bin").exists())

    def test_listing_filters_denied_children_and_binds_cursors(self) -> None:
        expected = {f"item-{number:03}.bin" for number in range(35)}
        for name in expected:
            (self.root / "allowed" / name).write_bytes(b"fixture")
        observed = set()
        cursor = None
        first_cursor = None
        for _ in range(20):
            page = self.client.list("/Artifacts/allowed", limit=5, cursor=cursor)
            observed.update(entry["name"] for entry in page["entries"])
            self.assertNotIn("private", observed)
            cursor = page["next_cursor"]
            first_cursor = first_cursor or cursor
            if cursor is None:
                break
        else:
            self.fail("listing did not terminate")
        self.assertEqual(observed, expected)
        self.assertIsNotNone(first_cursor)
        other = self.make_client()
        with self.assertRaises(InvalidRequest):
            other.list("/Artifacts/allowed", limit=5, cursor=first_cursor)
        with self.assertRaises(InvalidRequest):
            self.client.list("/Artifacts/allowed", cursor="forged")
        with self.assertRaises(LimitExceeded):
            self.client.list("/Artifacts/allowed", limit=201)

    def test_listing_cursors_do_not_disclose_filtered_filenames_when_decoded(self) -> None:
        names = {f"confidential-cursor-fixture-{number}.bin" for number in range(4)}
        for name in names:
            (self.root / "allowed" / name).write_bytes(b"fixture")
        # Exercise each exclusion so this test does not assume filesystem order.
        # At least one exclusion lies between allowed results in a paged scan.
        for denied in names:
            config = copy.deepcopy(self.config)
            config["policy"]["denied_paths"].append(f"allowed/{denied}")
            client = self.make_client(config)
            cursor = None
            observed = set()
            for _ in range(10):
                page = client.list("/Artifacts/allowed", limit=1, cursor=cursor)
                observed.update(entry["name"] for entry in page["entries"])
                cursor = page["next_cursor"]
                if cursor is None:
                    break
                self.assertNotIn(denied, cursor)
                for component in cursor.split("."):
                    try:
                        decoded = base64.urlsafe_b64decode(component + "=" * (-len(component) % 4))
                    except ValueError:
                        continue
                    self.assertNotIn(denied.encode(), decoded)
            else:
                self.fail("filtered pagination did not terminate")
            self.assertEqual(observed, names - {denied})

    @unittest.skipUnless(sys.platform.startswith("linux"), "regression exercises Linux long paths")
    def test_generated_long_path_cursor_can_be_used_for_every_page(self) -> None:
        directory = self.root / "allowed"
        for number in range(30):
            directory /= f"{number:03}" + "x" * 97
            directory.mkdir()
        names = {f"file-{number}-" + "y" * 93 for number in range(3)}
        for name in names:
            (directory / name).write_bytes(b"fixture")
        path = "/Artifacts/" + directory.relative_to(self.root).as_posix()
        self.assertLess(len(path), 4096)
        observed = set()
        cursor = None
        maximum_cursor_bytes = 0
        for _ in range(10):
            page = self.client.list(path, limit=1, cursor=cursor)
            observed.update(entry["name"] for entry in page["entries"])
            cursor = page["next_cursor"]
            if cursor is None:
                break
            maximum_cursor_bytes = max(maximum_cursor_bytes, len(cursor))
        else:
            self.fail("long-path pagination did not terminate")
        self.assertGreater(maximum_cursor_bytes, 4096, "fixture must exercise the old Python cursor ceiling")
        self.assertEqual(observed, names)

    def test_traversal_aliases_and_missing_parents_fail_before_creation(self) -> None:
        for path in ["/Artifacts/allowed/../outside/new.bin", "/Artifacts/allowed/%2e%2e/outside/new.bin", "/Artifacts/allowed\\..\\outside/new.bin", "/Artifacts/allowed/new.bin:stream"]:
            with self.subTest(path=path), self.assertRaises(InfimountError):
                self.client.write(path, b"denied")
        with self.assertRaises(NotFound):
            self.client.write("/Artifacts/allowed/missing-parent/new.bin", b"denied")
        self.assertFalse((self.root / "outside/new.bin").exists())
        self.assertFalse((self.root / "allowed/missing-parent").exists())

    def test_symlink_escape_is_denied_for_read_write_list_and_stat(self) -> None:
        link = self.root / "allowed/link"
        try:
            link.symlink_to(self.root / "outside", target_is_directory=True)
        except OSError as error:
            if os.name == "nt" and getattr(error, "winerror", None) == 1314:
                self.skipTest("Windows account lacks symlink creation privilege")
            raise
        for call in [
            lambda: self.client.read("/Artifacts/allowed/link/sentinel.bin"),
            lambda: self.client.stat("/Artifacts/allowed/link/sentinel.bin"),
            lambda: self.client.list("/Artifacts/allowed/link"),
            lambda: self.client.write("/Artifacts/allowed/link/new.bin", b"denied"),
        ]:
            with self.assertRaises(InfimountError):
                call()
        self.assertFalse((self.root / "outside/new.bin").exists())
        self.assertNotIn("link", {entry["name"] for entry in self.client.list("/Artifacts/allowed")["entries"]})

    def test_case_and_unicode_aliases_cannot_bypass_exclusions(self) -> None:
        config = copy.deepcopy(self.config)
        config["policy"]["denied_paths"].extend(["allowed/future", "allowed/caf\u00e9"])
        client = self.make_client(config)
        # The excluded path need not exist yet. The guard must also prevent a
        # create under a name that aliases that exclusion on another volume.
        aliases = ["PRIVATE", "FUTURE", unicodedata.normalize("NFD", "caf\u00e9")]
        for name in aliases:
            path = f"/Artifacts/allowed/{name}/new.bin"
            with self.subTest(path=path), self.assertRaises(PermissionDenied):
                client.write(path, b"must not escape the policy")
        (self.root / "allowed/ExactCase.bin").write_bytes(b"stored spelling")
        with self.assertRaises((PermissionDenied, NotFound)):
            client.read("/Artifacts/allowed/exactcase.bin")
        self.assertEqual(client.read("/Artifacts/allowed/ExactCase.bin"), b"stored spelling")
        for name in ["filename.", "filename "]:
            with self.subTest(name=name), self.assertRaises(InvalidRequest):
                client.write(f"/Artifacts/allowed/{name}", b"ambiguous")
        self.assertFalse((self.root / "allowed/future").exists())

    def test_private_execution_evidence_is_denied_under_a_broad_local_grant(self) -> None:
        config_directory = "infimount" if os.name == "nt" else ".infimount"
        protected = self.root / config_directory / "agent-task-executions"
        protected.mkdir(parents=True)
        sentinel = protected / "task.json"
        sentinel.write_bytes(b"private execution evidence")
        sibling = protected.parent / "visible.txt"
        sibling.write_bytes(b"ordinary sibling")
        config = copy.deepcopy(self.config)
        config["scopes"].append("infimount:delete")
        config["policy"] = {
            "version": 2, "default_access": "read_write", "rules": [],
            "denied_paths": [],
            "confirmation_rules": {"require_for_write": False, "require_for_delete": False},
        }
        ambient_root = str(self.root)
        # Rust canonicalizes the configured storage root to an extended-length
        # path on Windows. APPDATA normally retains the ordinary spelling.
        if os.name == "nt" and ambient_root.startswith("\\\\?\\"):
            ambient_root = (
                "\\\\" + ambient_root[8:]
                if ambient_root[4:8].lower() == "unc\\"
                else ambient_root[4:]
            )
        environment = {"APPDATA" if os.name == "nt" else "HOME": ambient_root}
        prefix = f"/Artifacts/{config_directory}"
        evidence = f"{prefix}/agent-task-executions"
        with patch.dict(os.environ, environment):
            client = self.make_client(config)
            self.assertEqual(client.read(f"{prefix}/visible.txt"), b"ordinary sibling")
            self.assertNotIn(
                "agent-task-executions",
                {entry["name"] for entry in client.list(prefix)["entries"]},
            )
            for operation, call in [
                ("read", lambda: client.read(f"{evidence}/task.json")),
                ("stat", lambda: client.stat(f"{evidence}/task.json")),
                ("list", lambda: client.list(evidence)),
                ("write", lambda: client.write(f"{evidence}/new.bin", b"denied")),
                ("delete", lambda: client.delete(f"{evidence}/task.json")),
                ("delete ancestor", lambda: client.delete(prefix)),
            ]:
                with self.subTest(operation=operation), self.assertRaises(PermissionDenied):
                    call()
        self.assertEqual(sentinel.read_bytes(), b"private execution evidence")
        self.assertFalse((protected / "new.bin").exists())

    def test_hosted_staging_names_are_denied_under_a_broad_local_grant(self) -> None:
        config = copy.deepcopy(self.config)
        config["policy"] = {
            "version": 2, "default_access": "read_write", "rules": [],
            "denied_paths": [], "confirmation_rules": {"require_for_write": False},
        }
        client = self.make_client(config)
        reserved = [".infimount-hosted-stage", ".infimount-hosted-staging"]
        for name in reserved:
            protected = self.root / name
            protected.mkdir()
            sentinel = protected / "private.bin"
            sentinel.write_bytes(b"unpublished private staging")
            path = f"/Artifacts/{name}"
            for operation, call in [
                ("read", lambda: client.read(f"{path}/private.bin")),
                ("stat", lambda: client.stat(f"{path}/private.bin")),
                ("list", lambda: client.list(path)),
                ("write", lambda: client.write(f"{path}/new.bin", b"denied")),
            ]:
                with self.subTest(name=name, operation=operation), self.assertRaises(InvalidRequest):
                    call()
            self.assertEqual(sentinel.read_bytes(), b"unpublished private staging")
            self.assertFalse((protected / "new.bin").exists())
        names = {entry["name"] for entry in client.list("/Artifacts")["entries"]}
        self.assertTrue(names.isdisjoint(reserved))

    def test_deletion_requires_scope_policy_and_file_target(self) -> None:
        (self.root / "allowed/remove.bin").write_bytes(b"remove")
        with self.assertRaises(PermissionDenied):
            self.client.delete("/Artifacts/allowed/remove.bin")
        config = copy.deepcopy(self.config)
        config["scopes"].append("infimount:delete")
        confirmation = self.make_client(config)
        with self.assertRaises(ConfirmationRequired):
            confirmation.delete("/Artifacts/allowed/remove.bin")
        config["policy"]["rules"][0]["confirmation_rules"]["require_for_delete"] = False
        deleter = self.make_client(config)
        with self.assertRaises(UnsupportedOperation):
            deleter.delete("/Artifacts/allowed")
        with self.assertRaises(PermissionDenied):
            deleter.delete("/Artifacts/allowed/private/sentinel.bin")
        deleter.delete("/Artifacts/allowed/remove.bin")
        self.assertFalse((self.root / "allowed/remove.bin").exists())
        self.assertTrue((self.root / "allowed/private/sentinel.bin").exists())

    def test_download_links_require_scope_confirmation_and_backend_capability(self) -> None:
        (self.root / "allowed/file.bin").write_bytes(b"link")
        with self.assertRaises(PermissionDenied):
            self.client.download_link("/Artifacts/allowed/file.bin")
        config = copy.deepcopy(self.config)
        config["scopes"].append("infimount:download")
        with self.assertRaises(ConfirmationRequired):
            self.make_client(config).download_link("/Artifacts/allowed/file.bin")
        config["policy"]["rules"][0]["confirmation_rules"]["require_for_presign"] = False
        with self.assertRaises(UnsupportedOperation):
            self.make_client(config).download_link("/Artifacts/allowed/file.bin")

    def test_audit_and_repr_do_not_contain_paths_payloads_or_ambient_identity(self) -> None:
        secret_name = "private-business-filename.bin"
        path = f"/Artifacts/allowed/{secret_name}"
        payload = b"content-must-not-enter-audit"
        with patch.dict(os.environ, {"INFIMOUNT_ACTIVATION_PROBE_ID": "untrusted-ambient-identity"}):
            self.client.write(path, payload)
            self.client.read(path)
        report = self.client.drain_audit_events()
        text = json.dumps(report) + repr(self.client)
        for value in [secret_name, str(self.root), payload.decode(), "untrusted-ambient-identity"]:
            self.assertNotIn(value, text)
        self.assertEqual(report["dropped"], 0)
        self.assertEqual(len(report["events"]), 2)
        self.assertTrue(all(event["path"].startswith("sha256:") for event in report["events"]))
        self.assertEqual(self.client.drain_audit_events()["events"], [])

    def test_audit_buffer_reports_dropped_events_without_unbounded_growth(self) -> None:
        for _ in range(260):
            self.client.stat("/Artifacts/allowed")
        report = self.client.drain_audit_events()
        self.assertEqual(len(report["events"]), 256)
        self.assertEqual(report["dropped"], 4)
        self.assertEqual(self.client.drain_audit_events(), {"events": [], "dropped": 0})

    @unittest.skipUnless(hasattr(os, "fork"), "fork does not exist on this platform")
    def test_inherited_client_fails_before_touching_runtime_or_locks(self) -> None:
        reader, writer = os.pipe()
        with warnings.catch_warnings():
            warnings.filterwarnings("ignore", category=DeprecationWarning)
            pid = os.fork()
        if pid == 0:
            os.close(reader)
            try:
                self.client.stat("/Artifacts/allowed")
            except InfimountError as error:
                if str(error) == "client_cannot_be_used_after_fork":
                    os.write(writer, b"denied")
            finally:
                os._exit(0)
        os.close(writer)
        try:
            ready, _, _ = select.select([reader], [], [], 5)
            self.assertTrue(ready, "inherited client blocked instead of rejecting fork reuse")
            self.assertEqual(os.read(reader, 32), b"denied")
        finally:
            os.close(reader)
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            os.waitpid(pid, 0)

    def test_credential_configuration_is_explicit_and_errors_are_redacted(self) -> None:
        config = copy.deepcopy(self.config)
        config["storage"] = {
            "backend": "s3", "name": "Artifacts", "bucket": "synthetic-artifacts",
            "region": "us-east-1", "endpoint": "https://storage.example.test",
            "access_key_id": {"kind": "env", "name": "INFIMOUNT_TEST_ACCESS"},
            "secret_access_key": {"kind": "env", "name": "INFIMOUNT_TEST_SECRET"},
        }
        with patch.dict(os.environ, {"INFIMOUNT_TEST_ACCESS": "synthetic-access", "INFIMOUNT_TEST_SECRET": "private-fixture-secret", "AWS_ACCESS_KEY_ID": "ambient-ignored", "AWS_SECRET_ACCESS_KEY": "ambient-ignored"}):
            client = self.make_client(config)
            self.assertNotIn("private-fixture-secret", repr(client))
            with self.assertRaises(PermissionDenied) as failure:
                client.read("/Artifacts/outside/private-fixture-secret")
            text = str(failure.exception) + json.dumps(client.drain_audit_events())
            self.assertNotIn("private-fixture-secret", text)
            malformed = copy.deepcopy(config)
            malformed["storage"]["secret_access_key"] = {"kind": "inline", "value": "private-fixture-secret"}
            with self.assertRaises(ConfigurationError) as failure:
                self.make_client(malformed)
            self.assertNotIn("private-fixture-secret", str(failure.exception))
            del config["storage"]["secret_access_key"]
            with self.assertRaises(ConfigurationError):
                self.make_client(config)

    def test_unknown_config_fields_and_closed_or_pickled_clients_fail(self) -> None:
        for mutate in [
            lambda value: value.update({"desktop_registry": "ambient"}),
            lambda value: value["policy"].update({"approved": True}),
            lambda value: value["policy"]["rules"][0].update({"prefix": "allowed "}),
            lambda value: value["policy"].update({"denied_paths": ["allowed/private "]}),
            lambda value: value["policy"]["rules"][0]["confirmation_rules"].update({"auto_approve": True}),
            lambda value: value.update({"limits": {"write_bytes": 4 * 1024 * 1024 + 1}}),
        ]:
            config = copy.deepcopy(self.config)
            mutate(config)
            with self.assertRaises(ConfigurationError):
                self.make_client(config)
        with self.assertRaises(TypeError):
            pickle.dumps(self.client)
        self.client.close()
        with self.assertRaises(InfimountError):
            self.client.stat("/Artifacts/allowed")


if __name__ == "__main__":
    unittest.main()
