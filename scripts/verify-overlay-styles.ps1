<#
.SYNOPSIS
  Verifies LIQUID's overlay window styles from outside the process.

.DESCRIPTION
  The app reporting its own window styles proves little. This enumerates top-level
  windows owned by liquid.exe and reads WS_EX_* directly via GetWindowLongPtrW, so
  the overlay's always-on-top / tool-window / no-activate / layered state is
  confirmed independently.

  Part of the Phase 1 exit criteria in IMPLEMENTATION_PLAN.md.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts/verify-overlay-styles.ps1
#>

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class WinProbe {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr GetWindowLongPtrW(IntPtr hWnd, int nIndex);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder text, int count);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    public static List<IntPtr> WindowsOfProcess(uint target) {
        var found = new List<IntPtr>();
        EnumWindows((h, l) => {
            uint pid;
            GetWindowThreadProcessId(h, out pid);
            if (pid == target) found.Add(h);
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static string TitleOf(IntPtr h) {
        var sb = new StringBuilder(512);
        GetWindowTextW(h, sb, sb.Capacity);
        return sb.ToString();
    }
}
'@

$GWL_EXSTYLE = -20
$STYLES = [ordered]@{
    'WS_EX_TOPMOST'     = 0x00000008
    'WS_EX_TRANSPARENT' = 0x00000020
    'WS_EX_TOOLWINDOW'  = 0x00000080
    'WS_EX_LAYERED'     = 0x00080000
    'WS_EX_NOACTIVATE'  = 0x08000000
}

$proc = Get-Process liquid -ErrorAction SilentlyContinue
if ($null -eq $proc) {
    Write-Output 'FAIL: liquid.exe is not running. Start it first.'
    exit 1
}

Write-Output "liquid.exe PID $($proc.Id), working set $([math]::Round($proc.WorkingSet64/1MB,1)) MB"
Write-Output ''

$handles = [WinProbe]::WindowsOfProcess([uint32]$proc.Id)
$overlays = 0
$failures = 0

foreach ($h in $handles) {
    $title = [WinProbe]::TitleOf($h)
    if ([string]::IsNullOrWhiteSpace($title)) { continue }

    $ex = [int64][WinProbe]::GetWindowLongPtrW($h, $GWL_EXSTYLE)
    $rect = New-Object WinProbe+RECT
    [void][WinProbe]::GetWindowRect($h, [ref]$rect)
    $visible = [WinProbe]::IsWindowVisible($h)

    $set = @()
    foreach ($name in $STYLES.Keys) {
        if (($ex -band $STYLES[$name]) -ne 0) { $set += $name }
    }

    Write-Output ("window: '{0}'" -f $title)
    Write-Output ("  hwnd      : 0x{0:X}" -f $h.ToInt64())
    Write-Output ("  rect      : {0},{1} -> {2},{3}" -f $rect.Left, $rect.Top, $rect.Right, $rect.Bottom)
    Write-Output ("  visible   : {0}" -f $visible)
    Write-Output ("  ex style  : 0x{0:X8}" -f $ex)
    Write-Output ("  flags     : {0}" -f ($(if ($set.Count) { $set -join ', ' } else { '(none)' })))

    if ($title -like '*Overlay*') {
        $overlays++
        foreach ($required in 'WS_EX_TOPMOST', 'WS_EX_TOOLWINDOW', 'WS_EX_NOACTIVATE') {
            if (($ex -band $STYLES[$required]) -eq 0) {
                Write-Output ("  FAIL      : missing {0}" -f $required)
                $failures++
            }
        }
        if ($failures -eq 0) { Write-Output '  OK        : overlay styles correct' }
    }
    Write-Output ''
}

Write-Output "overlay windows found: $overlays"
if ($overlays -eq 0) {
    Write-Output 'FAIL: no overlay windows found.'
    exit 1
}
if ($failures -gt 0) {
    Write-Output "FAIL: $failures required style(s) missing."
    exit 1
}
Write-Output 'PASS: every overlay window carries TOPMOST + TOOLWINDOW + NOACTIVATE.'
exit 0
