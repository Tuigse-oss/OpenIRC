#!/usr/bin/env bash
# Writes the third-party notices that ship with a release build:
#
#   scripts/third-party-notices.sh [output]    default THIRD-PARTY-NOTICES.txt
#
# Every crate compiled into the Windows program with its license text
# (cargo-about, configured in about.toml, laid out by scripts/notices.hbs),
# then the bundled DejaVu fonts' license and the Rust standard library's
# notices from the toolchain. Needs cargo-about (0.9) on the PATH.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-THIRD-PARTY-NOTICES.txt}
rule='============================================================================'

cargo about generate --config about.toml --locked --fail scripts/notices.hbs -o "$out"
{
    printf '\n%s\nDejaVu fonts (bundled in the program)\n%s\n\n' "$rule" "$rule"
    cat assets/DejaVu-LICENSE.txt
    printf '\n%s\nThe Rust Standard Library\n%s\n\n' "$rule" "$rule"
    "$(command -v python3 || command -v python)" - "$(rustc --print sysroot)" <<'PY'
import html, pathlib, re, sys
docs = pathlib.Path(sys.argv[1]) / "share" / "doc" / "rust"
text = html.unescape(re.sub(r"<[^>]+>", "", (docs / "COPYRIGHT-library.html").read_text()))
print("\n".join(line.rstrip() for line in text.splitlines() if line.strip()))
for name in ("MIT.txt", "Unicode-3.0.txt"):
    print(f"\n{'-' * 76}\n{name}\n{'-' * 76}\n")
    print((docs / "licenses" / name).read_text().strip())
PY
} >> "$out"
echo "wrote $out"
