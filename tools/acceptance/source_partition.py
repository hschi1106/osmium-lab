"""Provider-neutral reader for immutable osmium source partitions.

The module verifies the repository-owned storage contract only: current pointer,
manifest invariants, object paths, sizes, checksums, compression, and record
counts. It deliberately does not interpret provider wire fields.
"""

from __future__ import annotations

import hashlib
import json
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from collections.abc import Iterator
from typing import Any


MANIFEST_VERSION = 1
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


class PartitionVerificationError(Exception):
    pass


@dataclass(frozen=True)
class VerifiedSourcePartition:
    root: Path
    revision_identity: str
    page_count: int
    tick_record_count: int
    records: tuple[dict[str, Any], ...]


@dataclass(frozen=True)
class PublishedSourcePartition:
    """Verified manifest metadata whose pages can be consumed one at a time."""

    root: Path
    revision_identity: str
    page_count: int
    tick_record_count: int
    revision_root: Path
    pages: tuple[dict[str, Any], ...]


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _decompress_page(path: Path) -> bytes:
    if shutil.which("zstd") is None:
        raise PartitionVerificationError(
            "zstd command is required to read source pages"
        )
    result = subprocess.run(
        ["zstd", "-q", "-d", "-c", str(path)],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        raise PartitionVerificationError(
            f"cannot decompress source object: {path.name}"
        )
    return result.stdout


def _require_nonnegative_integer(value: Any, field: str) -> int:
    if type(value) is not int or value < 0:
        raise PartitionVerificationError(f"source object {field} is invalid")
    return value


def _validate_object_metadata(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise PartitionVerificationError("source object metadata must be an object")
    _require_nonnegative_integer(value.get("ordinal"), "ordinal")
    _require_nonnegative_integer(value.get("record_count"), "record_count")
    _require_nonnegative_integer(value.get("uncompressed_bytes"), "uncompressed_bytes")
    _require_nonnegative_integer(value.get("compressed_bytes"), "compressed_bytes")
    relative_path = value.get("relative_path")
    if not isinstance(relative_path, str) or not relative_path:
        raise PartitionVerificationError("source object path is missing or not text")
    for field in ("compressed_sha256", "uncompressed_sha256"):
        digest = value.get(field)
        if not isinstance(digest, str) or not SHA256_RE.fullmatch(digest):
            raise PartitionVerificationError(f"source object {field} is invalid")
    return value


def _read_object(
    revision_root: Path,
    metadata: dict[str, Any],
) -> Any:
    relative = Path(metadata["relative_path"])
    if relative.is_absolute() or ".." in relative.parts:
        raise PartitionVerificationError("source object path is not revision-relative")
    try:
        path = (revision_root / relative).resolve(strict=True)
    except OSError as error:
        raise PartitionVerificationError("source object is missing") from error
    if revision_root not in path.parents:
        raise PartitionVerificationError("source object escapes its revision")
    compressed = path.read_bytes()
    if len(compressed) != metadata["compressed_bytes"]:
        raise PartitionVerificationError("compressed source object size mismatch")
    if _sha256(compressed) != metadata["compressed_sha256"]:
        raise PartitionVerificationError("compressed source object checksum mismatch")
    uncompressed = _decompress_page(path)
    if len(uncompressed) != metadata["uncompressed_bytes"]:
        raise PartitionVerificationError("uncompressed source object size mismatch")
    if _sha256(uncompressed) != metadata["uncompressed_sha256"]:
        raise PartitionVerificationError("uncompressed source object checksum mismatch")
    try:
        return json.loads(uncompressed)
    except json.JSONDecodeError as error:
        raise PartitionVerificationError("source object is not valid JSON") from error


def open_published_partition(partition_root: Path) -> PublishedSourcePartition:
    """Verify current pointer and manifest metadata without loading tick pages."""
    partition_root = partition_root.resolve(strict=True)
    try:
        revision = (partition_root / "current.yaml").read_text(encoding="utf-8").strip()
    except OSError as error:
        raise PartitionVerificationError(
            "partition has no readable current.yaml"
        ) from error
    if not SHA256_RE.fullmatch(revision):
        raise PartitionVerificationError(
            "current.yaml does not contain a safe revision identity"
        )

    try:
        revision_root = (partition_root / "revisions" / revision).resolve(strict=True)
    except OSError as error:
        raise PartitionVerificationError("current source revision is missing") from error
    if partition_root not in revision_root.parents:
        raise PartitionVerificationError("current revision escapes its partition")
    try:
        manifest = json.loads(
            (revision_root / "manifest.yaml").read_text(encoding="utf-8")
        )
    except (OSError, json.JSONDecodeError) as error:
        raise PartitionVerificationError(
            "current revision has no valid source manifest"
        ) from error
    if not isinstance(manifest, dict):
        raise PartitionVerificationError("source manifest must be an object")
    if (
        type(manifest.get("manifest_version")) is not int
        or manifest["manifest_version"] != MANIFEST_VERSION
    ):
        raise PartitionVerificationError("unsupported source manifest version")
    if manifest.get("revision_identity") != revision:
        raise PartitionVerificationError(
            "current pointer and manifest revision identities differ"
        )
    if manifest.get("terminal_cursor_reached") is not True:
        raise PartitionVerificationError(
            "source revision did not reach a terminal cursor"
        )

    pages_value = manifest.get("pages")
    if not isinstance(pages_value, list) or not pages_value:
        raise PartitionVerificationError("source manifest has no tick pages")
    pages = [_validate_object_metadata(page) for page in pages_value]
    pages.sort(key=lambda page: page["ordinal"])
    expected_total = _require_nonnegative_integer(
        manifest.get("tick_record_count"), "tick_record_count"
    )
    for ordinal, page in enumerate(pages):
        if page.get("kind") != "TickPage" or page["ordinal"] != ordinal:
            raise PartitionVerificationError(
                "source tick page ordinals are not contiguous"
            )
    if sum(page["record_count"] for page in pages) != expected_total:
        raise PartitionVerificationError(
            "manifest total tick count differs from its pages"
        )
    return PublishedSourcePartition(
        root=partition_root,
        revision_identity=revision,
        page_count=len(pages),
        tick_record_count=expected_total,
        revision_root=revision_root,
        pages=tuple(pages),
    )


def iter_published_records(
    partition: PublishedSourcePartition,
) -> Iterator[dict[str, Any]]:
    """Verify and yield records, retaining at most one decompressed page."""
    for page in partition.pages:
        envelope = _read_object(
            partition.revision_root,
            page,
        )
        items = envelope.get("items") if isinstance(envelope, dict) else None
        if not isinstance(items, list) or len(items) != page["record_count"]:
            raise PartitionVerificationError("tick page record count mismatch")
        if not all(isinstance(item, dict) for item in items):
            raise PartitionVerificationError("tick page contains a non-object record")
        yield from items


def load_published_partition(partition_root: Path) -> VerifiedSourcePartition:
    """Verify one current published partition without interpreting wire records."""
    partition = open_published_partition(partition_root)
    return VerifiedSourcePartition(
        root=partition.root,
        revision_identity=partition.revision_identity,
        page_count=partition.page_count,
        tick_record_count=partition.tick_record_count,
        records=tuple(iter_published_records(partition)),
    )
