#!/usr/bin/env python3
"""Guard the ISA-twin invariants that no test can observe.

A `#[target_feature]` twin is byte-identical to its baseline sibling by
construction -- that is the whole point -- so the correctness suite is blind
to every way a twin can be wrong. Three ways it has actually been wrong here:

  1. ORPHANED ATTRIBUTES. Deleting a function's `fn` line while leaving its
     attribute block re-parents those attributes onto the next item. This put
     `#[target_feature(enable = "avx2,...")]` on a twin dispatched by
     `has_bmi2()` alone -- an illegal-instruction bug on Skylake Pentium and
     Celeron parts, which ship BMI2 with AVX2 fused off. It emits no warning.
     It also stole `#[inline(always)]` from `encode::lz_insert_rowknown`.

  2. GUARD/FEATURE MISMATCH. A twin must not enable a feature its runtime
     dispatch does not test. Same failure as (1), reachable by editing either
     side.

  3. THUNK TWINS. A twin whose body just calls an `#[inline(never)]` sibling
     compiles to a single `jmp`. It dispatches correctly, is byte-identical,
     and does nothing at all -- the shifts it exists to widen stay `%cl` in
     the baseline symbol. Four of these have shipped in this crate.

THE HIJACK IS COVERED BY TWO GATES, NOT ONE. A hijack lands in one of two
shapes, and this script only sees the first:

  (a) The stolen attribute is one the victim did NOT already carry -- the
      `decode_4x` case, where a twin silently gained `avx2`. Nothing in rustc
      objects. Check 1 below is the only thing that sees it.
  (b) The stolen attribute duplicates one the victim already has -- the
      `lz_insert_rowknown` case. Check 1 is BLIND to this: an attribute block
      that does reach a `fn` looks well-formed from the outside. rustc sees it
      as `unused_attributes`, which `lib.rs` now DENIES rather than warns.

Neither gate subsumes the other; both must stay wired.

Checks 1 and 2 are source-level and run here. Check 3 needs the emitted asm;
run `scripts/isaudit.sh` after `cargo rustc --release -- --emit asm` and look
for a twin whose `bmi2` column is 0 while its baseline sibling's `cl` is not.

NOTE ON COMMENTS. Every check below runs on comment-STRIPPED source. This
crate documents its own attributes in prose constantly -- `#[inline(always)]`,
`#[target_feature]` and `#[cold]` all appear inside `//` and `///` blocks --
and matching raw lines produces ~25 false positives to 2 real ones. A check
whose output nobody can read is a check nobody runs.

Exit status is 1 if any check fails, so this is CI-wireable as-is.
"""

import re
import sys
import glob
import os

# DELIBERATELY EMPTY. This started as `{"bmi2": {"lzcnt", "bmi1"}}` -- eight
# twins enable `bmi2,lzcnt` while `simd::has_bmi2()` tested only `bmi2`, and
# the exemption existed to keep this check quiet about it. That is backwards:
# an exemption is a place where the invariant is asserted by argument instead
# of checked. `has_bmi2()` now tests LZCNT too (one extra CPUID bit, once per
# process, rejecting no part that exists), so the guard covers exactly what
# the twins enable and nothing needs exempting.
#
# If a future twin needs a feature its guard does not test, widen the GUARD.
# Adding an entry here re-opens the hole this crate has already fallen into
# twice.
IMPLIED_BY = {}

DOC = re.compile(r"^\s*(///|//!)")
COMMENT = re.compile(r"^\s*//")
BLANK = re.compile(r"^\s*$")
ATTR = re.compile(r"^\s*#!?\[")
FN = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?(default\s+)?(const\s+)?(async\s+)?"
                r"(unsafe\s+)?(extern\s+\"[^\"]*\"\s+)?fn\s+(\w+)")
TF = re.compile(r'#\[target_feature\(enable\s*=\s*"([^"]+)"\)\]')
# Attributes that can only ever apply to a function. Attributes on statements,
# expressions and `use` items are legal Rust; none of these can be one.
FN_ONLY_ATTR = re.compile(r"^\s*#\[(target_feature|inline|cold|no_mangle|track_caller)\b")


def strip_comments(text):
    """Blank out comment content, preserving line count and indentation.

    Line comments are cut at `//`; block comments are blanked in full. String
    literals containing `//` would be mis-cut, but an attribute pattern inside
    a string literal is not a thing that occurs, and the failure direction is
    a false NEGATIVE, which is the safe one for a guard that must stay quiet
    to stay useful.
    """
    text = re.sub(r"/\*.*?\*/", lambda m: re.sub(r"[^\n]", " ", m.group(0)),
                  text, flags=re.S)
    out = []
    for line in text.splitlines():
        i = line.find("//")
        out.append(line[:i] if i >= 0 else line)
    return out


def check_orphans(files):
    """A function-only attribute must be followed by a `fn`, with no blank line."""
    bad = []
    for path in files:
        raw = open(path, encoding="utf8", errors="replace").read().splitlines()
        code = strip_comments("\n".join(raw))
        i = 0
        while i < len(code):
            if not FN_ONLY_ATTR.match(code[i]):
                i += 1
                continue
            start = i
            attrs = []
            # Consume the attribute block. Doc comments and plain comments may
            # interleave with attributes freely and do not break the block; a
            # blank line does. Attributes may span lines (`#[cfg_attr(\n ...)]`),
            # so track bracket depth rather than guessing from line endings --
            # guessing swallowed multi-line `fn` signatures, whose first line
            # also ends in `(`.
            while i < len(code):
                if FN.match(code[i]):
                    break                      # the `fn` this block belongs to
                if ATTR.match(code[i]):
                    attrs.append(code[i].strip())
                    depth = code[i].count("(") - code[i].count(")")
                    i += 1
                    while i < len(code) and depth > 0:
                        depth += code[i].count("(") - code[i].count(")")
                        i += 1
                    continue
                if BLANK.match(code[i]) and not BLANK.match(raw[i]):
                    i += 1                     # comment-only line, blanked by the stripper
                    continue
                break
            if i >= len(code):
                break
            if BLANK.match(code[i]) and BLANK.match(raw[i]):
                bad.append((path, start + 1,
                            "blank line between a function-only attribute and its `fn`", attrs))
            elif not FN.match(code[i]):
                bad.append((path, start + 1,
                            f"function-only attribute, but next item is: {code[i].strip()[:58]}",
                            attrs))
            i += 1
    return bad


def guard_features(src):
    """Map each `has_*()` predicate to the CPU features it actually tests.

    Read out of the detection source rather than hardcoded, so the guard
    tracks what the code does instead of what a table claims. `has_bmi2()`
    tests both `bmi2` and `lzcnt`; if someone drops the `lzcnt` test, every
    twin enabling `bmi2,lzcnt` starts failing this check immediately, which is
    the entire point.
    """
    provides = {}
    for text in src.values():
        for m in re.finditer(r"\bfn\s+has_(\w+)\s*\([^)]*\)[^{]*\{", text):
            name = m.group(1)
            depth, i = 1, m.end()
            while i < len(text) and depth:
                depth += (text[i] == "{") - (text[i] == "}")
                i += 1
            body = text[m.end():i]
            feats = set(re.findall(r'is_x86_feature_detected!\s*\(\s*"([^"]+)"', body))
            feats |= set(re.findall(r'is_aarch64_feature_detected!\s*\(\s*"([^"]+)"', body))
            if feats:
                provides.setdefault(name, set()).update(feats)
    return provides


def check_guards(files):
    """A twin must not enable a feature its dispatch guard does not test."""
    src = {p: "\n".join(strip_comments(open(p, encoding="utf8", errors="replace").read()))
           for p in files}
    provides = guard_features(src)
    bad = []
    for path, text in src.items():
        for m in TF.finditer(text):
            feats = {f.strip() for f in m.group(1).split(",") if f.strip()}
            fm = re.search(r"\bfn\s+(\w+)", text[m.end():m.end() + 900])
            if not fm:
                continue
            fn = fm.group(1)
            guards = set()
            for text2 in src.values():
                for cm in re.finditer(r"\b" + re.escape(fn) + r"\s*\(", text2):
                    pre = text2[max(0, cm.start() - 800):cm.start()]
                    guards |= {g.group(1) for g in re.finditer(r"has_(\w+)\(\)", pre)}
            if not guards:
                continue  # no runtime dispatch found; not this check's business
            # What the guards actually TEST, read from the detection source --
            # `has_bmi2()` covers {bmi2, lzcnt}, not just its own name.
            tested = set(guards)
            for g in guards:
                tested |= provides.get(g, set())
                tested |= IMPLIED_BY.get(g, set())
            missing = feats - tested
            if missing:
                bad.append((path, text[:m.start()].count("\n") + 1, fn,
                            sorted(feats), sorted(tested), sorted(missing)))
    return bad


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    files = sorted(glob.glob(os.path.join(root, "crates", "*", "src", "**", "*.rs"),
                             recursive=True))
    if not files:
        print("twinguard: no sources found", file=sys.stderr)
        return 1

    fail = False

    orphans = check_orphans(files)
    if orphans:
        fail = True
        print("ATTRIBUTE HIJACK: function-only attribute with no `fn` attached\n")
        for path, line, why, attrs in orphans:
            print(f"  {os.path.relpath(path, root)}:{line}: {why}")
            for a in attrs:
                print(f"      {a}")
        print()

    guards = check_guards(files)
    if guards:
        fail = True
        print("TWIN GUARD MISMATCH: twin enables a feature its dispatch does not test\n")
        for path, line, fn, feats, gu, missing in guards:
            print(f"  {os.path.relpath(path, root)}:{line}: {fn}")
            print(f"      enables:     {','.join(feats)}")
            print(f"      guard tests: {','.join(gu)}")
            print(f"      MISSING:     {','.join(missing)}  <-- illegal instruction on a part "
                  f"with the tested feature but not this one")
        print()

    if fail:
        return 1
    print(f"twinguard: {len(files)} files clean "
          f"(no orphaned attributes, no twin/guard feature mismatch)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
