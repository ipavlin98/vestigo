param([switch]$Local)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
Set-Location (Split-Path $PSScriptRoot -Parent)

if (git status --porcelain) { throw 'The working tree has changes. Commit or stash them first.' }
git switch main
git pull --ff-only

$files = @(Get-ChildItem changes -Filter *.md | Where-Object Name -ne 'README.md')
if (!$files) { throw 'Nothing to release: changes\ has no files.' }

$minor = $false
$bullets = foreach ($file in $files) {
    $lines = @(Get-Content $file.FullName -Encoding utf8)
    $level = "$($lines[0])".Trim()
    if ($level -notin 'minor', 'patch') { throw "$($file.Name): the first line must be 'minor' or 'patch'." }
    if ($level -eq 'minor') { $minor = $true }
    ($lines | Select-Object -Skip 1) -join "`n" | ForEach-Object Trim
}

$manifest = [IO.File]::ReadAllText("$PWD\Cargo.toml")
$found = [regex]::Match($manifest, '(?m)^version = "(\d+)\.(\d+)\.(\d+)"')
if (!$found.Success) { throw "Couldn't read the version from Cargo.toml." }
$major, $middle, $patch = $found.Groups[1..3].Value | ForEach-Object { [int]$_ }
$old = "$major.$middle.$patch"
$new = if ($minor) { "$major.$($middle + 1).0" } else { "$major.$middle.$($patch + 1)" }

$utf8 = [Text.UTF8Encoding]::new($false)
$version = [regex]'(?m)^version = "[^"]+"'
[IO.File]::WriteAllText("$PWD\Cargo.toml", $version.Replace($manifest, "version = `"$new`"", 1), $utf8)
$lock = [IO.File]::ReadAllText("$PWD\Cargo.lock")
$locked = [regex]'(?m)(^name = "vestigo"\r?\nversion = ")[^"]+'
[IO.File]::WriteAllText("$PWD\Cargo.lock", $locked.Replace($lock, "`${1}$new", 1), $utf8)

$section = "## [$new] — $(Get-Date -Format 'yyyy-MM-dd')`n`n$($bullets -join "`n")`n`n"
$changelog = [IO.File]::ReadAllText("$PWD\CHANGELOG.md")
$first = [regex]::Match($changelog, '(?m)^## \[')
$at = if ($first.Success) { $first.Index } else { $changelog.Length }
[IO.File]::WriteAllText("$PWD\CHANGELOG.md", $changelog.Insert($at, $section), $utf8)
$files | Remove-Item

Write-Host "Vestigo $old -> $new"
if ($Local) {
    Write-Host 'Left in the working tree: review the changes, then commit them on a release branch.'
    return
}

git switch -c "chore/release-$new"
git commit -am "Release $new"
git push -u origin HEAD
gh pr create --title "Release $new" --body "Raises the version to $new and moves the notes from ``changes/`` to the changelog. Merging it publishes the release." --label skip-changelog
gh pr merge --auto --squash
git switch main
