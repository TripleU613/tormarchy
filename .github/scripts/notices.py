#!/usr/bin/env python3
"""Write THIRD-PARTY-NOTICES.txt: the original notices of everything linked
into the two shipped binaries.

    python3 .github/scripts/notices.py > THIRD-PARTY-NOTICES.txt
    python3 .github/scripts/notices.py --crates   # just the linked crate list

What is linked is what cargo resolves for the release target with build-time
edges (build scripts, proc macros) left out, per binary. Each crate's own
license files are copied verbatim from the exact source cargo built. Then the
Rust standard library's notices, as the toolchain that builds the release
ships them, and the two C pieces Rust's musl target links statically: musl
1.2.5 and LLVM's libunwind (texts in licenses/).

The release job regenerates this before building, so the file in the archive
and the copy the helper installs match the toolchain that built them.
"""
import html
import json
import os
import re
import subprocess
import sys

TARGET = "x86_64-unknown-linux-musl"
BINARIES = ("tormarchy", "tormarchyd")
REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|COPYRIGHT|UNLICENSE|NOTICE)", re.I)


def run(*args):
    return subprocess.run(args, cwd=REPO, check=True, capture_output=True, text=True).stdout


def linked():
    """{(name, version): set(binaries)} for crates outside this workspace."""
    found = {}
    for b in BINARIES:
        out = run("cargo", "tree", "--locked", "-p", b, "--target", TARGET,
                  "-e", "normal,no-proc-macro", "--prefix", "none", "--format", "{p}")
        for line in out.splitlines():
            parts = line.split()
            if len(parts) < 2 or "(/" in line or parts[0].startswith("tormarchy"):
                continue
            found.setdefault((parts[0], parts[1].lstrip("v")), set()).add(b)
    return found


def main():
    crates = linked()
    if "--crates" in sys.argv:
        for name, version in sorted(crates):
            print(f"{name} {version}")
        return
    meta = json.loads(run("cargo", "metadata", "--locked", "--format-version", "1",
                          "--filter-platform", TARGET))
    pkgs = {(p["name"], p["version"]): p for p in meta["packages"]}
    sysroot = run("rustc", "--print", "sysroot").strip()
    rustc = run("rustc", "--version").strip()

    out = []
    w = out.append
    w("Third-party notices for tormarchy and tormarchyd\n")
    w("=" * 48 + "\n\n")
    w(f"Built for {TARGET} with {rustc}.\n")
    w("Tormarchy's own code is under the MIT license in LICENSE.\n\n")
    w("Linked into the shipped binaries:\n\n")
    for key in sorted(crates):
        p = pkgs[key]
        w(f"  {key[0]} {key[1]:<10} {p.get('license') or '?':<28} {', '.join(sorted(crates[key]))}\n")
    w("  Rust standard library     Apache-2.0 OR MIT (and its dependencies, below)  both\n")
    w("  musl 1.2.5                MIT                                               both\n")
    w("  LLVM libunwind            Apache-2.0 WITH LLVM-exception                    both\n")

    for key in sorted(crates):
        p = pkgs[key]
        root = os.path.dirname(p["manifest_path"])
        files = sorted(f for f in os.listdir(root) if LICENSE_FILE.match(f))
        if not files:
            sys.exit(f"notices: {key[0]} {key[1]} ships no license file")
        for f in files:
            w("\n" + "-" * 78 + "\n")
            w(f"{key[0]} {key[1]}: {f}\n")
            w("-" * 78 + "\n\n")
            w(open(os.path.join(root, f), encoding="utf-8", errors="replace").read().rstrip() + "\n")

    std = open(os.path.join(sysroot, "share/doc/rust/COPYRIGHT-library.html"), encoding="utf-8").read()
    std = html.unescape(re.sub(r"<[^>]+>", "", re.sub(r"(?is)<(style|script)\b.*?</\1>", "", std)))
    std = re.sub(r"\n[ \t]*\n(\s*\n)+", "\n\n", std).strip()
    w("\n" + "-" * 78 + f"\nThe Rust standard library ({rustc}): COPYRIGHT-library\n" + "-" * 78 + "\n\n")
    w(std + "\n")

    for title, path in (("musl 1.2.5: COPYRIGHT", "licenses/musl-1.2.5-COPYRIGHT"),
                        ("LLVM libunwind: LICENSE.TXT", "licenses/llvm-LICENSE.TXT")):
        w("\n" + "-" * 78 + f"\n{title}\n" + "-" * 78 + "\n\n")
        w(open(os.path.join(REPO, path), encoding="utf-8").read().rstrip() + "\n")

    sys.stdout.write("".join(out))


if __name__ == "__main__":
    main()
