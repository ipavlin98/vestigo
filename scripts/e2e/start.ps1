param([switch]$Stop, [int]$MemoryMB = 3072)

$ErrorActionPreference = 'Stop'
$processes = 'WindowsSandbox', 'WindowsSandboxClient', 'WindowsSandboxRemoteSession'
Get-Process $processes -ErrorAction SilentlyContinue | Stop-Process -Force
if ($Stop) { return }

$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$work = Join-Path $env:TEMP 'vestigo-e2e'
$gpui = Join-Path $repo 'vendor\gpui_windows'
$ffmpegLink = Get-Item (Get-Command ffmpeg).Source
$ffmpeg = Split-Path ($(if ($ffmpegLink.ResolvedTarget) { $ffmpegLink.ResolvedTarget } else { $ffmpegLink.FullName }))

Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory "$work\cmd", "$work\out", "$work\rt" | Out-Null
Copy-Item "$PSScriptRoot\agent.ps1" $work
foreach ($dll in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'd3dcompiler_47.dll') {
    Copy-Item "$env:SystemRoot\System32\$dll" "$work\rt"
}

$config = @"
<Configuration>
  <VGpu>Disable</VGpu>
  <MemoryInMB>$MemoryMB</MemoryInMB>
  <Networking>Disable</Networking>
  <MappedFolders>
    <MappedFolder><HostFolder>$repo\target\debug</HostFolder><SandboxFolder>C:\app</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>$gpui</HostFolder><SandboxFolder>$gpui</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>$ffmpeg</HostFolder><SandboxFolder>C:\ffmpeg</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>$work</HostFolder><SandboxFolder>C:\work</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>
  </MappedFolders>
  <LogonCommand><Command>powershell -ExecutionPolicy Bypass -File C:\work\agent.ps1</Command></LogonCommand>
</Configuration>
"@
Set-Content "$work\sandbox.wsb" $config

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Hidden {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo {
        public int cb; public string reserved, desktop, title;
        public int x, y, width, height, xChars, yChars, fill, flags;
        public short show, reserved2; public IntPtr reserved3, input, output, error;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInfo { public IntPtr process, thread; public int processId, threadId; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateDesktop(string name, IntPtr device, IntPtr mode, int flags, uint access, IntPtr attributes);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcess(string application, string commandLine, IntPtr processAttributes,
        IntPtr threadAttributes, bool inherit, int flags, IntPtr environment, string directory,
        ref StartupInfo startup, out ProcessInfo info);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    public static void Start(string desktop, string commandLine) {
        if (CreateDesktop(desktop, IntPtr.Zero, IntPtr.Zero, 0, 0x10000000, IntPtr.Zero) == IntPtr.Zero)
            throw new System.ComponentModel.Win32Exception();
        var startup = new StartupInfo { desktop = desktop };
        startup.cb = Marshal.SizeOf(startup);
        ProcessInfo info;
        if (!CreateProcess(null, commandLine, IntPtr.Zero, IntPtr.Zero, false, 0, IntPtr.Zero, null, ref startup, out info))
            throw new System.ComponentModel.Win32Exception();
        CloseHandle(info.thread);
        CloseHandle(info.process);
    }
}
"@

[Hidden]::Start('vestigo-e2e', "`"$env:SystemRoot\System32\WindowsSandbox.exe`" `"$work\sandbox.wsb`"")
$deadline = (Get-Date).AddMinutes(3)
while (-not (Test-Path "$work\out\ready.txt")) {
    if ((Get-Date) -gt $deadline) { throw 'sandbox did not become ready in 3 minutes' }
    Start-Sleep -Milliseconds 500
}
"ready: $work\out"
