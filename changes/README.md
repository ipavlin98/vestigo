# Unreleased changes

A user-visible change adds one file here instead of editing `CHANGELOG.md` or the version in `Cargo.toml`. Every
branch writes its own file, so any number of branches merge without conflicts.

- Name: the branch name without its prefix, for example `folder-marks.md` for `feat/folder-marks`.
- First line: `minor` for a new feature, setting or notable workflow; `patch` for a fix or a refinement of existing
  behavior.
- Then the changelog bullets, written for users.

```markdown
minor
- Folder marks: `Alt+M` and a letter remembers the current folder, `Alt+'` and the letter jumps back to it.
```

Internal changes (refactoring, tests, CI, docs) need no file.

`scripts\release.ps1` folds every file into a new `CHANGELOG.md` section, raises the version (minor if any file says
`minor`, otherwise patch) and deletes the files; see *Releasing* in [CONTRIBUTING.md](../CONTRIBUTING.md).
