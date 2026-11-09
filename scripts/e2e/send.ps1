param([Parameter(ValueFromPipeline)][string]$Line, [int]$TimeoutSeconds = 100)

begin { $lines = @() }
process { $lines += $Line }
end {
    $cmd = Join-Path $env:TEMP 'vestigo-e2e\cmd'
    $name = [guid]::NewGuid().ToString('N')
    Set-Content "$cmd\$name.tmp" ($lines -join "`n")
    Rename-Item "$cmd\$name.tmp" "$name.ps1"
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while (-not (Test-Path "$cmd\$name.done")) {
        if ((Get-Date) -gt $deadline) { throw 'sandbox agent did not answer: run scripts\e2e\start.ps1' }
        Start-Sleep -Milliseconds 200
    }
    Get-Content "$cmd\$name.out" -ErrorAction SilentlyContinue
    Remove-Item "$cmd\$name.done", "$cmd\$name.out" -ErrorAction SilentlyContinue
}
