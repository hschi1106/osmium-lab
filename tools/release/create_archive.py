#!/usr/bin/env python3
"""Create a deterministic gzip-compressed tar archive for a release."""

from __future__ import annotations

import argparse
import gzip
import os
from pathlib import Path
import stat
import tarfile


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--name", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source-date-epoch", type=int, default=0)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.source_date_epoch < 0:
        raise SystemExit("--source-date-epoch must be non-negative")
    root = args.root.resolve()
    output = args.output.resolve()
    if not root.is_dir():
        raise SystemExit(f"archive root is not a directory: {root}")
    if output.exists():
        raise SystemExit(f"archive already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)

    files = sorted(path for path in root.rglob("*") if path.is_file())
    with output.open("wb") as raw:
        with gzip.GzipFile(
            filename="", fileobj=raw, mode="wb", mtime=args.source_date_epoch
        ) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for path in files:
                    relative = path.relative_to(root).as_posix()
                    archive_name = f"{args.name}/{relative}"
                    info = archive.gettarinfo(str(path), arcname=archive_name)
                    info.uid = 0
                    info.gid = 0
                    info.uname = ""
                    info.gname = ""
                    info.mtime = args.source_date_epoch
                    info.pax_headers = {}
                    mode = path.stat().st_mode
                    info.mode = 0o755 if mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH) else 0o644
                    with path.open("rb") as payload:
                        archive.addfile(info, payload)


if __name__ == "__main__":
    main()
