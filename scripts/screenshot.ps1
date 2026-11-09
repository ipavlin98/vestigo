param([Parameter(Mandatory = $true)][string]$OutDir)

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class VestigoWindows {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    delegate bool EnumProc(IntPtr hwnd, IntPtr state);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr state);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] static extern int GetWindowLong(IntPtr hwnd, int index);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT point);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
    [DllImport("user32.dll")] static extern int GetWindowRgnBox(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr hwnd, int attribute, out RECT rect, int size);

    public static List<IntPtr> Visible(uint pid) {
        var found = new List<IntPtr>();
        EnumWindows((hwnd, state) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            bool clickThrough = (GetWindowLong(hwnd, -20) & 0x20) != 0;
            if (owner == pid && IsWindowVisible(hwnd) && !clickThrough) {
                found.Add(hwnd);
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static RECT Shown(IntPtr hwnd) {
        RECT frame;
        DwmGetWindowAttribute(hwnd, 9, out frame, 16);
        RECT region;
        if (GetWindowRgnBox(hwnd, out region) > 1) {
            RECT outer;
            DwmGetWindowAttribute(hwnd, 9, out outer, 16);
            frame.Left = outer.Left + region.Left;
            frame.Top = outer.Top + region.Top;
            frame.Right = outer.Left + region.Right;
            frame.Bottom = outer.Top + region.Bottom;
        }
        return frame;
    }

    public static bool OnTop(IntPtr hwnd, RECT rect) {
        int inset = 24;
        int[] xs = { rect.Left + inset, (rect.Left + rect.Right) / 2, rect.Right - inset };
        int[] ys = { rect.Top + inset, (rect.Top + rect.Bottom) / 2, rect.Bottom - inset };
        foreach (int x in xs) {
            foreach (int y in ys) {
                POINT point = new POINT { X = x, Y = y };
                if (GetAncestor(WindowFromPoint(point), 2) != hwnd) {
                    return false;
                }
            }
        }
        return true;
    }
}
"@

[void][VestigoWindows]::SetProcessDPIAware()
New-Item -ItemType Directory -Force $OutDir | Out-Null
$process = Get-Process vestigo -ErrorAction Stop | Select-Object -First 1
$index = 0
foreach ($hwnd in [VestigoWindows]::Visible([uint32]$process.Id)) {
    $rect = [VestigoWindows]::Shown($hwnd)
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -lt 64 -or $height -lt 32) { continue }
    if (-not [VestigoWindows]::OnTop($hwnd, $rect)) {
        "skipped a covered Vestigo window ({0}x{1}): bring it to the front and retry" -f $width, $height
        continue
    }
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $path = Join-Path $OutDir ("window-{0}.png" -f $index)
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
    "{0} {1}x{2}" -f $path, $width, $height
    $index++
}
