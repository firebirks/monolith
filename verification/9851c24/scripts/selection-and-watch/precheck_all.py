"""Runs the runner's own precheck over every list against the repository
tree as it is, without running anything."""
import os
import sys

repo = r'<repo>'
sys.path.insert(0, os.path.join(repo, 'mutation'))
import faults  # noqa: E402
import run  # noqa: E402

lists = {
    'phase1': faults.PHASE1, 'phase2': faults.PHASE2, 'phase3': faults.PHASE3,
    'phase4': faults.PHASE4, 'phase5': faults.PHASE5, 'phase6': faults.PHASE6,
    'phase7': faults.PHASE7, 'phase7w': faults.PHASE7_WINDOWS,
    'phase8': faults.PHASE8, 'phase8w': faults.PHASE8_WINDOWS,
}
total = 0
for name, items in lists.items():
    problems = run.precheck(items, repo)
    total += len(problems)
    print(f'{name}: {len(items)} faults, {len(problems)} problems')
    for p in problems:
        print('   ', p)
print('total problems', total)
