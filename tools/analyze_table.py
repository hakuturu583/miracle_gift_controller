"""Statistical analysis of the reference command table (ohtake/miracle-light-controller src/mlc.ts).

usage: python3 tools/analyze_table.py path/to/mlc.ts
Prints the table ordered by the "bits" ID (bit i = high tone at slot i, slot 0 = LSB),
code-space statistics, and a permutation test of effect clustering per bit ordering.
"""
import re, sys, random
from collections import Counter
from itertools import combinations

src = open(sys.argv[1], encoding="utf-8").read()
rows = []
for m in re.finditer(r"\{ command: '(\d{7})', pattern: \[([^\]]*)\](, devices: \['strawberry'\])? \}", src):
    items = [x.strip().strip("'") for x in m.group(2).split(",") if x.strip()]
    bits = [1 if int(d) >= 3 else 0 for d in m.group(1)]
    rows.append(dict(code=m.group(1), bits=bits, bpm=int(items[0]) if items else None, colors=items[1:], straw=bool(m.group(3))))

def kind(r):
    if r["bpm"] is None: return "."
    if r["bpm"] == 10: return "S"          # steady
    if len(r["colors"]) == 2 and r["colors"][1] == "black": return "B"  # on/off blink
    return "P"                               # multi-colour pattern

def ident(r, le=True, hi1=True):
    b = [x if hi1 else 1 - x for x in r["bits"]]
    if not le: b = b[::-1]
    return sum(x << i for i, x in enumerate(b))

for r in sorted(rows, key=ident):
    i = ident(r)
    print(f"{i:3} {i:07b} {r['code']} {kind(r)} {r['bpm'] or '':>3} {'/'.join(r['colors']):45} {'strawberry' if r['straw'] else ''}")

known = [r for r in rows if r["bpm"] is not None]
print("\nknown", len(known), "unknown", len(rows) - len(known), "strawberry", sum(r["straw"] for r in rows))
print("min Hamming distance among known codes:",
      min(sum(a != b for a, b in zip(x["bits"], y["bits"])) for x, y in combinations(known, 2)))
print("parity of known codes:", Counter(sum(r["bits"]) % 2 for r in known))

def score(seq): return sum(1 for a, b in zip(seq, seq[1:]) if a != "." and a == b)
random.seed(0)
null = sorted(score(random.sample([kind(r) for r in rows], len(rows))) for _ in range(5000))
print(f"\nadjacent same-kind pairs; random: median {null[2500]}, 99th pct {null[4950]}")
for le in (True, False):
    for hi1 in (True, False):
        seq = [kind(r) for r in sorted(rows, key=lambda r: ident(r, le, hi1))]
        print(f"  {'slot0=LSB' if le else 'slot0=MSB'} {'high=1' if hi1 else 'low=1'}: {score(seq):3}  {''.join(seq)}")
