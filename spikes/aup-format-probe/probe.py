#!/usr/bin/env python3
"""Spike S5 — clean-room decoder for the Audacity project document blob.

The grammar below was derived by observation of file bytes alone: no Audacity
source was consulted, so nothing here inherits Audacity's GPL. It is validated
against a corpus by requiring every byte of every document to be consumed —
a wrong grammar desynchronises and fails loudly rather than quietly guessing.

    ./probe.py corpus/*.aup3          # structural report
    ./probe.py --xml one.aup3         # reconstructed XML
    ./probe.py --json corpus/*.aup3   # machine-readable, for CI

--- Wire format -------------------------------------------------------------

An .aup3/.aup4 is an SQLite database (application_id 0x41554459 "AUDY").
`project.dict` and `project.doc` hold the document; `autosave` holds the same
shape for an unsaved session and is empty in a cleanly closed project.

dict := 00 04                                  two-byte prologue, invariant
        ( 0F id:u16 nbytes:u16 utf32le[nbytes] )*

The dict maps small integer ids to element and attribute names, so the document
never repeats a name. Strings throughout are UTF-32LE and lengths are in BYTES,
not characters.

doc  := record*
   01 id:u16                                   start element
   02 id:u16                                   end element
   03 id:u16 nbytes:u32 utf32le[nbytes]        attribute, string
   04 id:u16 value:i32                         attribute, 32-bit signed
   05 id:u16 value:u8                          attribute, byte/bool
   06 id:u16 value:u32                         attribute, 32-bit
   07 id:u16 value:u64                         attribute, 64-bit
   08 id:u16 value:u32                         attribute, 32-bit
   0A id:u16 value:f64 digits:i32              attribute, double + precision
   0C nbytes:u32 utf32le[nbytes]               character data

0x06 and 0x08 carry identical 4-byte payloads and are used interchangeably for
the same attribute across files (`sampleformat` appears under both). They are
presumably distinct C++ overloads upstream; a reader can treat them alike.
`digits` on 0x0A is a formatting hint, 0xFFFFFFFF meaning "default".

Tags 0x00, 0x09, 0x0B, 0x0D, 0x0E do not occur in the corpus. A reader must
reject them rather than assume a width.
"""

import argparse
import json
import sqlite3
import struct
import sys
from collections import Counter

DICT_PROLOGUE = b"\x00\x04"
APPLICATION_ID = 0x41554459  # "AUDY"

START, END, CHARDATA, DICT_ENTRY = 0x01, 0x02, 0x0C, 0x0F
# tag -> (label, payload width in bytes after the u16 name id)
FIXED = {0x04: ("i32", 4), 0x05: ("u8", 1), 0x06: ("u32", 4),
         0x07: ("u64", 8), 0x08: ("u32", 4), 0x0A: ("f64", 12)}
ATTR_STR = 0x03

# Sample formats observed. The high half is bytes-per-sample, the low half a
# type code. Note the absence of any 32-bit integer format — see S5 findings.
SAMPLE_FORMATS = {0x00020001: "int16", 0x00040001: "int24", 0x0004000F: "float32"}


class FormatError(Exception):
    pass


def _utf32(raw):
    return raw.decode("utf-32-le")


def parse_dict(blob):
    if not blob.startswith(DICT_PROLOGUE):
        raise FormatError(f"dict prologue {blob[:2].hex()!r}")
    names, i = {}, len(DICT_PROLOGUE)
    while i < len(blob):
        if blob[i] != DICT_ENTRY:
            raise FormatError(f"dict tag {blob[i]:#04x} at {i}")
        ident, nbytes = struct.unpack_from("<HH", blob, i + 1)
        i += 5
        names[ident] = _utf32(blob[i : i + nbytes])
        i += nbytes
    return names


def parse_doc(blob, names):
    """Yield (kind, name, value). Consumes every byte or raises."""
    i, n = 0, len(blob)
    while i < n:
        tag, off = blob[i], i
        i += 1
        if tag == CHARDATA:
            (nbytes,) = struct.unpack_from("<I", blob, i)
            i += 4
            yield "text", None, _utf32(blob[i : i + nbytes])
            i += nbytes
            continue
        if tag == DICT_ENTRY:  # dict entries may also appear inline
            ident, nbytes = struct.unpack_from("<HH", blob, i)
            i += 4 + 2
            names[ident] = _utf32(blob[i : i + nbytes])
            i += nbytes
            continue
        (ident,) = struct.unpack_from("<H", blob, i)
        i += 2
        name = names.get(ident, f"<unknown id {ident}>")
        if tag == START:
            yield "start", name, None
        elif tag == END:
            yield "end", name, None
        elif tag == ATTR_STR:
            (nbytes,) = struct.unpack_from("<I", blob, i)
            i += 4
            yield "attr", name, _utf32(blob[i : i + nbytes])
            i += nbytes
        elif tag in FIXED:
            kind, width = FIXED[tag]
            if kind == "i32":
                (v,) = struct.unpack_from("<i", blob, i)
            elif kind == "u8":
                v = blob[i]
            elif kind == "u32":
                (v,) = struct.unpack_from("<I", blob, i)
            elif kind == "u64":
                (v,) = struct.unpack_from("<Q", blob, i)
            else:  # f64 + precision hint we deliberately discard
                (v,) = struct.unpack_from("<d", blob, i)
            i += width
            yield "attr", name, v
        else:
            raise FormatError(f"tag {tag:#04x} at {off} (name {name!r})")


def to_xml(events):
    """Rebuild the XML text. Order of attributes is preserved as written."""
    out, open_tag = [], None
    for kind, name, value in events:
        if kind == "text":
            if open_tag:
                out.append(">")
                open_tag = None
            out.append(value)
        elif kind == "start":
            if open_tag:
                out.append(">")
            out.append(f"<{name}")
            open_tag = name
        elif kind == "attr":
            text = value if isinstance(value, str) else repr(value)
            out.append(f' {name}="{text}"')
        elif kind == "end":
            if open_tag == name:
                out.append("/>")
                open_tag = None
            else:
                out.append(f"</{name}>")
    return "".join(out)


def inspect(path):
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    app_id = db.execute("pragma application_id").fetchone()[0]
    user_version = db.execute("pragma user_version").fetchone()[0]
    page_size = db.execute("pragma page_size").fetchone()[0]
    row = db.execute("select dict, doc from project").fetchone()
    if row is None or row[1] is None:
        raise FormatError("no project document")
    names = parse_dict(row[0]) if row[0] else {}
    events = list(parse_doc(row[1], names))

    tags = Counter(n for k, n, _ in events if k == "start")
    # Cross-check the document against the audio actually present.
    referenced = {v for k, n, v in events if k == "attr" and n == "blockid"}
    stored = {r[0] for r in db.execute("select blockid from sampleblocks")}
    fmts = {r[0] for r in db.execute("select distinct sampleformat from sampleblocks")}
    result = {
        "path": path,
        "application_id": hex(app_id),
        "magic": app_id.to_bytes(4, "big").decode("ascii", "replace"),
        "user_version": ".".join(str(b) for b in user_version.to_bytes(4, "big")),
        "page_size": page_size,
        "dict_entries": len(names),
        "doc_bytes": len(row[1]),
        "events": len(events),
        "elements": dict(tags),
        "sample_formats": sorted(SAMPLE_FORMATS.get(f, hex(f)) for f in fmts),
        "blocks_stored": len(stored),
        "blocks_referenced": len(referenced),
        "dangling_refs": sorted(referenced - stored),
        "orphan_blocks": len(stored - referenced),
        "rate": next((v for k, n, v in events if k == "attr" and n == "rate"), None),
        "xml": to_xml(events),
    }
    db.close()
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("paths", nargs="+")
    ap.add_argument("--xml", action="store_true", help="print reconstructed XML")
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    args = ap.parse_args()

    results, failures = [], 0
    for path in args.paths:
        try:
            r = inspect(path)
        except (FormatError, sqlite3.DatabaseError) as exc:
            failures += 1
            print(f"FAIL {path}: {exc}", file=sys.stderr)
            continue
        results.append(r)
        if args.xml:
            print(r["xml"])
            continue
        if args.json:
            continue
        name = path.rsplit("/", 1)[-1]
        print(f"{name[:44]:46} {r['magic']} v{r['user_version']:9} "
              f"page={r['page_size']:<6} {'+'.join(r['sample_formats']):8} "
              f"rate={r['rate']!s:8} blocks={r['blocks_stored']:<6} "
              f"refs={r['blocks_referenced']:<6} "
              f"dangling={len(r['dangling_refs'])} orphan={r['orphan_blocks']}")

    if args.json:
        json.dump(results, sys.stdout, indent=2)
        print()
    elif not args.xml:
        elements = Counter()
        for r in results:
            elements.update(r["elements"])
        print(f"\n{len(results)} parsed, {failures} failed")
        print(f"elements seen: {dict(elements)}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
