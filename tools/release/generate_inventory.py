#!/usr/bin/env python3
"""Generate deterministic CycloneDX and dependency-license inventory files."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import uuid
from urllib.parse import quote


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--sbom", required=True, type=Path)
    parser.add_argument("--licenses", required=True, type=Path)
    return parser.parse_args()


def cargo_metadata(root: Path) -> dict:
    completed = subprocess.run(
        [
            "cargo",
            "metadata",
            "--manifest-path",
            "crates/osmium-cli/Cargo.toml",
            "--locked",
            "--format-version",
            "1",
        ],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(completed.stdout)


def package_ref(package: dict) -> str:
    name = quote(package["name"], safe="-._")
    return f"pkg:cargo/{name}@{package['version']}"


def source_label(package: dict) -> str:
    source = package.get("source")
    if source is None:
        return "workspace"
    if source.startswith("registry+"):
        return "crates.io"
    if source.startswith("git+"):
        return "git"
    return "external"


def dependency_closure(metadata: dict) -> tuple[dict, set[str], str]:
    packages = {package["id"]: package for package in metadata["packages"]}
    root_package = next(package for package in metadata["packages"] if package["name"] == "osmium-cli")
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    reachable: set[str] = set()
    pending = [root_package["id"]]
    while pending:
        package_id = pending.pop()
        if package_id in reachable:
            continue
        reachable.add(package_id)
        pending.extend(dependency["pkg"] for dependency in nodes[package_id]["deps"])
    return packages, reachable, root_package["id"]


def licenses_for(package: dict) -> tuple[list[dict], str]:
    expression = package.get("license")
    if expression:
        return [{"license": {"expression": expression}}], expression
    return [{"license": {"name": "NOASSERTION"}}], "NOASSERTION"


def component(package: dict, root_package_id: str) -> tuple[dict, str]:
    licenses, expression = licenses_for(package)
    ref = package_ref(package)
    result = {
        "bom-ref": ref,
        "type": "application" if package["id"] == root_package_id else "library",
        "name": "osmium" if package["id"] == root_package_id else package["name"],
        "version": package["version"],
        "purl": ref,
        "scope": "required",
        "licenses": licenses,
        "properties": [
            {"name": "osmium:source", "value": source_label(package)},
            {"name": "osmium:license-expression", "value": expression},
        ],
    }
    if package.get("repository"):
        result["externalReferences"] = [{"type": "vcs", "url": package["repository"]}]
    return result, ref


def write_outputs(args: argparse.Namespace, metadata: dict) -> None:
    packages, reachable, root_package_id = dependency_closure(metadata)
    selected = sorted(
        (packages[package_id] for package_id in reachable),
        key=lambda package: (package["name"], package["version"], package["id"]),
    )
    components = []
    refs = {}
    for package in selected:
        item, ref = component(package, root_package_id)
        components.append(item)
        refs[package["id"]] = ref

    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    dependencies = []
    for package in selected:
        dependencies.append(
            {
                "ref": refs[package["id"]],
                "dependsOn": sorted(
                    refs[dependency["pkg"]]
                    for dependency in nodes[package["id"]]["deps"]
                    if dependency["pkg"] in refs
                ),
            }
        )

    root_ref = refs[root_package_id]
    serial = f"urn:uuid:{uuid.uuid5(uuid.NAMESPACE_URL, f'osmium-sbom-{args.version}') }"
    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": serial,
        "version": 1,
        "metadata": {
            "component": next(item for item in components if item["bom-ref"] == root_ref),
            "properties": [
                {"name": "osmium:dependency-source", "value": "cargo metadata --locked"},
                {"name": "osmium:package-version", "value": args.version},
            ],
        },
        "components": components,
        "dependencies": dependencies,
    }

    args.sbom.parent.mkdir(parents=True, exist_ok=True)
    args.sbom.write_text(json.dumps(sbom, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    lines = [
        "osmium third-party dependency license inventory",
        "inventory_version: 1",
        f"product_version: {args.version}",
        "source: cargo metadata --locked, transitive closure of osmium-cli",
        "license_text_policy: declared SPDX/license expressions are recorded; NOASSERTION requires review",
        "",
    ]
    for package in selected:
        _, expression = licenses_for(package)
        lines.append(
            f"{package['name']} {package['version']} | license={expression} | source={source_label(package)}"
        )
    args.licenses.parent.mkdir(parents=True, exist_ok=True)
    args.licenses.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> None:
    args = parse_args()
    write_outputs(args, cargo_metadata(args.root.resolve()))


if __name__ == "__main__":
    main()
