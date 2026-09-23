"""Write the Scoop and winget manifests for one release.

usage: package-manifests.py VERSION DIR
DIR holds the release's screenpeek-<arch>-windows.zip.sha256 files; an
architecture without one is left out.

>>> manifests = render("0.0.2-alpha", {"x86_64": "AB" * 32})
>>> scoop = json.loads(manifests["bucket/screenpeek.json"])
>>> scoop["version"], list(scoop["architecture"])
('0.0.2-alpha', ['64bit'])
>>> scoop["architecture"]["64bit"]["url"].endswith("/v0.0.2-alpha/screenpeek-x86_64-windows.zip")
True
>>> "InstallerSha256: " + "AB" * 32 in manifests[f"{WINGET}.installer.yaml"]
True
"""

import json
import sys
from pathlib import Path

REPO = "I-No-oNe/screenpeek"
ID = "I-No-oNe.screenpeek"
WINGET = f"packaging/winget/{ID}"
DESCRIPTION = "Read desktop apps as text and click controls by name"
# Release archive name, Scoop architecture, winget architecture.
ARCHES = [("x86_64", "64bit", "x64"), ("aarch64", "arm64", "arm64")]


def url(version, arch):
    return f"https://github.com/{REPO}/releases/download/v{version}/screenpeek-{arch}-windows.zip"


def render(version, hashes):
    """File path to contents, for the architectures in `hashes`."""
    present = [arch for arch in ARCHES if arch[0] in hashes]
    scoop = {
        "version": version,
        "description": DESCRIPTION,
        "homepage": f"https://github.com/{REPO}",
        "license": "MIT",
        "architecture": {
            scoop_arch: {"url": url(version, arch), "hash": hashes[arch].lower()}
            for arch, scoop_arch, _ in present
        },
        "bin": "screenpeek.exe",
        # Every release is an alpha for now, which GitHub's "latest" skips.
        "checkver": {
            "url": f"https://api.github.com/repos/{REPO}/releases",
            "jsonpath": "$[0].tag_name",
            "regex": "v([\\w.-]+)",
        },
        "autoupdate": {
            "architecture": {
                scoop_arch: {"url": url("$version", arch)} for arch, scoop_arch, _ in ARCHES
            },
            "hash": {"url": "$url.sha256"},
        },
    }
    header = f"PackageIdentifier: {ID}\nPackageVersion: {version}\n"
    installers = "".join(
        f"- Architecture: {winget_arch}\n"
        f"  InstallerUrl: {url(version, arch)}\n"
        f"  InstallerSha256: {hashes[arch].upper()}\n"
        for arch, _, winget_arch in present
    )
    return {
        "bucket/screenpeek.json": json.dumps(scoop, indent=4) + "\n",
        f"{WINGET}.yaml": header + "DefaultLocale: en-US\nManifestType: version\nManifestVersion: 1.6.0\n",
        f"{WINGET}.installer.yaml": header
        + "InstallerType: zip\nNestedInstallerType: portable\nNestedInstallerFiles:\n"
        + "- RelativeFilePath: screenpeek.exe\n  PortableCommandAlias: screenpeek\n"
        + "Installers:\n"
        + installers
        + "ManifestType: installer\nManifestVersion: 1.6.0\n",
        f"{WINGET}.locale.en-US.yaml": header
        + "PackageLocale: en-US\nPublisher: I-No-oNe\nPackageName: screenpeek\n"
        + f"PackageUrl: https://github.com/{REPO}\nLicense: MIT\n"
        + f"LicenseUrl: https://github.com/{REPO}/blob/main/LICENSE\n"
        + f"ShortDescription: {DESCRIPTION}\n"
        + "Tags:\n- ocr\n- automation\n- accessibility\n- ai-agents\n"
        + "ManifestType: defaultLocale\nManifestVersion: 1.6.0\n",
    }


def main():
    version, folder = sys.argv[1], Path(sys.argv[2])
    hashes = {}
    for arch, _, _ in ARCHES:
        checksum = folder / f"screenpeek-{arch}-windows.zip.sha256"
        if checksum.exists():
            hashes[arch] = checksum.read_text().split()[0]
    if not hashes:
        sys.exit(f"no Windows checksums in {folder}")
    for path, text in render(version, hashes).items():
        Path(path).parent.mkdir(parents=True, exist_ok=True)
        Path(path).write_text(text)
        print(path)


if __name__ == "__main__":
    main()
