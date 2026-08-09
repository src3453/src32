import re
from collections import Counter

path = "test.sol.a"

mn_re = re.compile(r"^\s*([A-Z]{2,6})\b")
ignore_directives = {'.ORG', '.DB'}

counts = Counter()
lines = []
with open(path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

for ln in lines:
    s = ln.strip()
    if not s:
        continue
    if s.startswith(';'):
        continue
    if s.startswith('.'):
        # directive
        tok = s.split()[0]
        if tok in ignore_directives:
            continue
    if s.endswith(':'):
        continue
    m = mn_re.match(ln)
    if m:
        instr = m.group(1)
        counts[instr] += 1

total = sum(counts.values())
print(f"Total instructions: {total}")
print()
print(f"{'Mnemonic':<10} {'Count':>7} {'Percent':>9}")
print('-'*30)
for instr, cnt in counts.most_common():
    pct = cnt/total*100 if total>0 else 0
    print(f"{instr:<10} {cnt:7d} {pct:8.2f}%")
