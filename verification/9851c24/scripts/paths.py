base = '<work>\\verify8\\'


def edit(name, pairs):
    p = base + name
    s = open(p, encoding='utf-8').read()
    for old, new in pairs:
        assert s.count(old) == 1, (name, old)
        s = s.replace(old, new)
    open(p, 'w', encoding='utf-8', newline='\n').write(s)


# The worktree and its build under target\ of the repository, which the
# antivirus leaves alone (a fresh test binary elsewhere may be blocked).
edit('windows-stage.ps1', [
    ("$src = Join-Path $here 'win-src'\n$out = Join-Path $here 'out-windows'\n$repo = '<repo>'\n",
     "$out = Join-Path $here 'out-windows'\n$repo = '<repo>'\n$src = Join-Path $repo 'target\\final8-src'\n"),
    ("$env:CARGO_TARGET_DIR = Join-Path $here 'win-target'\n",
     "$env:CARGO_TARGET_DIR = Join-Path $repo 'target\\final8-win'\n"),
])
edit('master.ps1', [
    ("git -C $repo worktree remove --force (Join-Path $V 'win-src') *>> $log\nRemove-Item (Join-Path $V 'win-target') -Recurse -Force -ErrorAction SilentlyContinue\n",
     "git -C $repo worktree remove --force (Join-Path $repo 'target\\final8-src') *>> $log\nRemove-Item (Join-Path $repo 'target\\final8-win') -Recurse -Force -ErrorAction SilentlyContinue\n"),
])
print('ok')
