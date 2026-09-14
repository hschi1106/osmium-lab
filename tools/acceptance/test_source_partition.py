#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from source_partition import PartitionVerificationError, load_published_partition


@unittest.skipUnless(shutil.which("zstd"), "zstd command is not installed")
class PublishedSourcePartitionTests(unittest.TestCase):
    def make_partition(self, root: Path) -> Path:
        partition = root / "provider-a" / "partition-a"
        revision = "a" * 64
        revision_root = partition / "revisions" / revision
        pages = revision_root / "objects"
        pages.mkdir(parents=True)
        (partition / "current.yaml").write_text(revision, encoding="utf-8")
        items = [{"provider_owned_field": "opaque-to-storage-verifier"}]
        uncompressed = json.dumps(
            {"items": items, "next_cursor": None}, separators=(",", ":")
        ).encode("utf-8")
        compressed = subprocess.run(
            ["zstd", "-q", "-c"],
            input=uncompressed,
            check=True,
            stdout=subprocess.PIPE,
        ).stdout
        relative_path = "objects/00000000.json.zst"
        (revision_root / relative_path).write_bytes(compressed)
        manifest = {
            "manifest_version": 1,
            "revision_identity": revision,
            "query_identity": "b" * 64,
            "terminal_cursor_reached": True,
            "tick_record_count": len(items),
            "pages": [
                {
                    "kind": "TickPage",
                    "ordinal": 0,
                    "relative_path": relative_path,
                    "record_count": len(items),
                    "uncompressed_bytes": len(uncompressed),
                    "compressed_bytes": len(compressed),
                    "uncompressed_sha256": hashlib.sha256(uncompressed).hexdigest(),
                    "compressed_sha256": hashlib.sha256(compressed).hexdigest(),
                }
            ],
        }
        (revision_root / "manifest.yaml").write_text(
            json.dumps(manifest), encoding="utf-8"
        )
        return partition

    def test_verifies_storage_without_interpreting_provider_fields(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            report = load_published_partition(self.make_partition(Path(temp_dir)))
            self.assertEqual(report.tick_record_count, 1)
            self.assertEqual(
                report.records[0]["provider_owned_field"],
                "opaque-to-storage-verifier",
            )

    def test_rejects_tampered_compressed_object(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            partition = self.make_partition(Path(temp_dir))
            revision = (partition / "current.yaml").read_text(encoding="utf-8")
            page = partition / "revisions" / revision / "objects/00000000.json.zst"
            page.write_bytes(page.read_bytes() + b"tampered")
            with self.assertRaises(PartitionVerificationError):
                load_published_partition(partition)

    def test_rejects_malformed_metadata_without_provider_parser(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            partition = self.make_partition(Path(temp_dir))
            revision = (partition / "current.yaml").read_text(encoding="utf-8")
            manifest_path = partition / "revisions" / revision / "manifest.yaml"
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            manifest["pages"] = [None]
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaises(PartitionVerificationError):
                load_published_partition(partition)


if __name__ == "__main__":
    unittest.main()
