Set-Service Audiosrv -StartupType Disabled
Stop-Service Audiosrv -Force
$out = 'C:\work\out'
$cmd = 'C:\work\cmd'
New-Item -ItemType Directory $out, C:\run -Force | Out-Null
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class Mouse {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] static extern bool ScreenToClient(IntPtr w, ref POINT p);
  [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr w, uint f);
  [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr w);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr w, uint m, IntPtr wp, IntPtr lp);
  public static void Click(int x, int y) {
    var p = new POINT { X = x, Y = y };
    var w = WindowFromPoint(p);
    ScreenToClient(w, ref p);
    var at = (IntPtr)((p.Y << 16) | (p.X & 0xFFFF));
    SetForegroundWindow(GetAncestor(w, 2));
    PostMessage(w, 0x0200, IntPtr.Zero, at); System.Threading.Thread.Sleep(60);
    PostMessage(w, 0x0201, (IntPtr)1, at); System.Threading.Thread.Sleep(40);
    PostMessage(w, 0x0202, IntPtr.Zero, at);
  }
}
"@
[Mouse]::SetProcessDPIAware() | Out-Null
function Shot($name) {
  $b = [Windows.Forms.SystemInformation]::VirtualScreen
  $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
  [Drawing.Graphics]::FromImage($bmp).CopyFromScreen($b.Location, [Drawing.Point]::Empty, $b.Size)
  $bmp.Save("$out\$name.png"); $bmp.Dispose()
}
function Keys($keys) { [Windows.Forms.SendKeys]::SendWait($keys) }
function Click($x, $y) { [Mouse]::Click($x, $y) }
function Deploy {
  Get-Process vestigo -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep 1
  Copy-Item C:\app\vestigo.exe C:\run\ -Force
  Copy-Item C:\work\rt\*.dll C:\run\ -Force
}
function App($arguments) { if ($arguments) { Start-Process C:\run\vestigo.exe -ArgumentList $arguments } else { Start-Process C:\run\vestigo.exe } }
function Rec($name, $seconds) {
  Start-Process C:\ffmpeg\ffmpeg.exe -WindowStyle Hidden -ArgumentList '-y','-f','gdigrab','-framerate','10','-t',"$seconds",'-i','desktop','-vf','scale=trunc(iw/2)*2:trunc(ih/2)*2','-c:v','libx264','-preset','ultrafast','-pix_fmt','yuv420p',"$out\$name.mp4"
}
Deploy
'ready' | Set-Content "$out\ready.txt"
while ($true) {
  foreach ($f in Get-ChildItem $cmd -Filter *.ps1 | Sort-Object Name) {
    $log = "$cmd\$($f.BaseName).out"
    try { . $f.FullName *>&1 | Out-String | Set-Content $log } catch { "ERROR: $_" | Set-Content $log }
    Remove-Item $f.FullName
    'done' | Set-Content "$cmd\$($f.BaseName).done"
  }
  Start-Sleep -Milliseconds 300
}
