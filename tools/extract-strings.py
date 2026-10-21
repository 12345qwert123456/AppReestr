#!/usr/bin/env python3
"""Extract translatable strings (tr, trf, trn) from the Rust sources into po/appreestr.pot.

xgettext's C lexer trips over Rust lifetimes, so this does the small job directly.
Usage: tools/extract-strings.py  (then: msgmerge -U po/ru.po po/appreestr.pot)
"""
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
STR = r'"((?:[^"\\]|\\.)*)"'
SINGLE = re.compile(r'\btrf?\(\s*' + STR, re.S)
PLURAL = re.compile(r'\btrn\(\s*' + STR + r'\s*,\s*' + STR, re.S)


def po_escape(s: str) -> str:
    return s.replace("\n", "\\n")


def main() -> None:
    entries: dict[tuple, list[str]] = {}
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        text = path.read_text()
        rel = path.relative_to(ROOT)
        for m in SINGLE.finditer(text):
            line = text.count("\n", 0, m.start()) + 1
            entries.setdefault((m.group(1),), []).append(f"{rel}:{line}")
        for m in PLURAL.finditer(text):
            line = text.count("\n", 0, m.start()) + 1
            entries.setdefault((m.group(1), m.group(2)), []).append(f"{rel}:{line}")
    out = [
        'msgid ""',
        'msgstr ""',
        '"Project-Id-Version: appreestr\\n"',
        '"Content-Type: text/plain; charset=UTF-8\\n"',
        '"Content-Transfer-Encoding: 8bit\\n"',
        "",
    ]
    for key, refs in entries.items():
        out.append("#: " + " ".join(refs))
        out.append(f'msgid "{po_escape(key[0])}"')
        if len(key) == 2:
            out.append(f'msgid_plural "{po_escape(key[1])}"')
            out.append('msgstr[0] ""')
            out.append('msgstr[1] ""')
        else:
            out.append('msgstr ""')
        out.append("")
    (ROOT / "po" / "appreestr.pot").write_text("\n".join(out))
    print(f"{len(entries)} messages")


if __name__ == "__main__":
    main()
