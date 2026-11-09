param([string]$Target = 'x86_64-pc-windows-msvc', [string]$Out = 'dist\THIRD-PARTY-NOTICES.txt')
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$metadata = (cargo metadata --format-version 1 --filter-platform $Target | ConvertFrom-Json)
$active = @{}
cargo tree --target $Target --edges normal,build --prefix none --format '{p}' | ForEach-Object {
    if ($_ -match '^(\S+) v(\S+)') { $active["$($matches[1]) $($matches[2])"] = $true }
}
$notices = [System.Text.StringBuilder]::new()
[void]$notices.AppendLine('Vestigo third-party licenses')
foreach ($package in ($metadata.packages | Sort-Object name, version)) {
    if ($package.name -eq 'vestigo' -or !$active.ContainsKey("$($package.name) $($package.version)")) { continue }
    [void]$notices.AppendLine("`n================================================================")
    [void]$notices.AppendLine("$($package.name) $($package.version)")
    [void]$notices.AppendLine("License: $($package.license)")
    [void]$notices.AppendLine("Source: $($package.repository)")
    $packageDir = Split-Path $package.manifest_path -Parent
    $licenseFiles = @(Get-ChildItem -LiteralPath $packageDir -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' })
    if ($package.license_file) {
        $licensePath = Join-Path $packageDir $package.license_file
        if (Test-Path -LiteralPath $licensePath) { $licenseFiles += Get-Item -LiteralPath $licensePath }
    }
    foreach ($file in ($licenseFiles | Sort-Object FullName -Unique)) {
        [void]$notices.AppendLine("`n$($file.Name):")
        [void]$notices.AppendLine([System.IO.File]::ReadAllText($file.FullName))
    }
}
$path = Join-Path (Get-Location) $Out
New-Item -ItemType Directory -Force (Split-Path $path -Parent) | Out-Null
[System.IO.File]::WriteAllText($path, $notices.ToString())
