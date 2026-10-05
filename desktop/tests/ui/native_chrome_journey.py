"""Exercise custom chrome in the real, already launched Versora WebView2 window.

Run only after rebuilding, using launch_native.ps1's fresh isolated launch proof:
  python native_chrome_journey.py --launch-proof <native-process-proof.json> \
      --evidence <new directory under desktop/tests/ui>

This harness never launches, terminates or destroys a process, emulates a browser
viewport, sends keyboard input, or substitutes a backend. Confirmation closes the
application through its own CloseRequested handler. Failures before confirmation
leave the application open; failures after confirmation preserve its exit evidence.
"""
import argparse
import asyncio
import ctypes
from ctypes import wintypes as W
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time
from urllib.parse import urlparse

from playwright.async_api import async_playwright
from native_journey import comparable_path


async def navigate(page, where):
    """Open a page through the corner coin; the current page's own control is hidden."""
    target = page.get_by_test_id(f"nav-{where}")
    if await target.is_visible():
        await target.click()
    await page.wait_for_function("p => document.querySelector(`[data-testid=nav-${p}]`).getAttribute('aria-current') === 'page'", arg=where)


TEST_ROOT = Path(__file__).resolve().parent
CLOSE_TEXT = "\u4ef2\u6709\u672a\u5132\u5b58\u5605\u7de8\u8f2f\u3002\u96e2\u958b\u6703\u653e\u68c4\u5462\u6b21\u7de8\u8f2f\u3002"
RESIZE = {
    "north": (0, -24, "North"), "east": (24, 0, "East"),
    "south": (0, 24, "South"), "west": (-24, 0, "West"),
    "north-west": (-24, -24, "NorthWest"),
    "north-east": (24, -24, "NorthEast"),
    "south-west": (-24, 24, "SouthWest"),
    "south-east": (24, 24, "SouthEast"),
}
LOCALES = ("en", "zh-Hant", "zh-Hans", "ja", "ko", "fr", "de", "es", "pt", "vi", "th", "id")
R40_REFERENCES = {
    "Litora-motion-scrollbar-reference.txt": "b6cb25d3a0569463969efdb2a9b16622db7164952e1428eeb0268673c522de05",
    "Litora-motion-scrollbar-source-map.json": "ed41a168c38ac1f3a147f544b272ef9f5f2dcab210e5de64fcafbdd97bb55a1c",
    "Litora-motion-scrollbar-verbatim.txt": "722adefdbfc7089930baa656044ac75dcb1ee0558924bb62a7fef204bf5831e1",
}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def owned_test_path(value):
    path = Path(value).resolve()
    require(path.is_relative_to(TEST_ROOT) and path != TEST_ROOT,
            "Proof and evidence must stay below desktop/tests/ui.")
    return path


def sha256(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def creation_seconds(value):
    if isinstance(value, str) and value.startswith("/Date("):
        return int(re.match(r"/Date\((-?\d+)", value).group(1)) / 1000
    require(isinstance(value, str), "Launch proof requires a creation timestamp.")
    # CIM/.NET can preserve seven fractional digits; Windows process FILETIME is
    # pinned exactly after matching this independent, millisecond-safe receipt.
    value = re.sub(r"(\.\d{6})\d+(?=[+-]|Z|$)", r"\1", value)
    stamp = datetime.fromisoformat(value.replace("Z", "+00:00"))
    require(stamp.tzinfo is not None, "Creation timestamp must include a timezone.")
    return stamp.timestamp()


class RECT(ctypes.Structure):
    _fields_ = [(name, W.LONG) for name in ("left", "top", "right", "bottom")]


class POINT(ctypes.Structure):
    _fields_ = [("x", W.LONG), ("y", W.LONG)]


class WINDOWPLACEMENT(ctypes.Structure):
    _fields_ = [("length", W.UINT), ("flags", W.UINT), ("showCmd", W.UINT),
                ("minPosition", POINT), ("maxPosition", POINT), ("normal", RECT)]


class MONITORINFO(ctypes.Structure):
    _fields_ = [("size", W.DWORD), ("monitor", RECT), ("work", RECT), ("flags", W.DWORD)]


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", W.LONG), ("dy", W.LONG), ("mouseData", W.DWORD),
                ("flags", W.DWORD), ("time", W.DWORD), ("extra", ctypes.c_size_t)]


class INPUTUNION(ctypes.Union):
    _fields_ = [("mouse", MOUSEINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", W.DWORD), ("data", INPUTUNION)]


class OwnedWindow:
    """Every native target is pinned to the launch receipt and live process handle."""

    def __init__(self, proof):
        require(os.name == "nt", "Actual Windows HWND/WebView2 is required.")
        self.user = ctypes.WinDLL("user32", use_last_error=True)
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.pid = int(proof["processId"])
        self.hwnd = int(proof["windowHandle"])
        self.executable = Path(proof["executable"]).resolve()
        self.digest = proof["executableSHA256"].lower()
        require(re.fullmatch(r"[0-9a-f]{64}", self.digest), "Invalid expected EXE SHA256.")
        desktop = TEST_ROOT.parent.parent
        installed = Path(os.environ["LOCALAPPDATA"]) / "Programs/Versora/versora.exe"
        require(self.executable.is_relative_to(desktop) or
                comparable_path(self.executable) == comparable_path(installed),
                "Executable is outside the checkout or approved exact install.")
        require(self.executable.name.casefold() == "versora.exe" and
                proof["windowTitle"] == "Versora", "Unexpected native identity.")
        self._declare()
        self.pointer_cleanup = []
        # This only changes the harness coordinate context, never app settings.
        self.user.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4))
        self.process = self.kernel.OpenProcess(0x1000 | 0x00100000, False, self.pid)
        require(self.process, "Could not open the owned process for read/wait access.")
        self.created = self._created()
        records = [p for p in proof["processes"] if int(p["ProcessId"]) == self.pid]
        require(len(records) == 1, "Launch proof requires one exact root process.")
        expected = creation_seconds(records[0]["CreationDate"])
        actual = self.created / 10_000_000 - 11_644_473_600
        require(abs(actual - expected) <= .002, "PID creation time differs from launch proof.")
        self.verify_binary()
        self.guard()

    def _declare(self):
        self.kernel.OpenProcess.argtypes = [W.DWORD, W.BOOL, W.DWORD]
        self.kernel.OpenProcess.restype = W.HANDLE
        self.kernel.CloseHandle.argtypes = [W.HANDLE]
        self.kernel.GetProcessTimes.argtypes = [W.HANDLE] + [ctypes.POINTER(W.FILETIME)] * 4
        self.kernel.GetExitCodeProcess.argtypes = [W.HANDLE, ctypes.POINTER(W.DWORD)]
        self.kernel.QueryFullProcessImageNameW.argtypes = [W.HANDLE, W.DWORD, W.LPWSTR, ctypes.POINTER(W.DWORD)]
        self.kernel.WaitForSingleObject.argtypes = [W.HANDLE, W.DWORD]
        self.user.IsWindow.argtypes = [W.HWND]
        self.user.IsWindowVisible.argtypes = [W.HWND]
        self.user.IsWindowEnabled.argtypes = [W.HWND]
        self.user.IsIconic.argtypes = [W.HWND]
        self.user.IsZoomed.argtypes = [W.HWND]
        self.user.GetWindowThreadProcessId.argtypes = [W.HWND, ctypes.POINTER(W.DWORD)]
        self.user.GetWindowTextW.argtypes = [W.HWND, W.LPWSTR, ctypes.c_int]
        self.user.GetClassNameW.argtypes = [W.HWND, W.LPWSTR, ctypes.c_int]
        self.user.GetWindowRect.argtypes = [W.HWND, ctypes.POINTER(RECT)]
        self.user.GetClientRect.argtypes = [W.HWND, ctypes.POINTER(RECT)]
        self.user.GetWindowPlacement.argtypes = [W.HWND, ctypes.POINTER(WINDOWPLACEMENT)]
        self.user.ClientToScreen.argtypes = [W.HWND, ctypes.POINTER(POINT)]
        self.user.GetDpiForWindow.argtypes = [W.HWND]
        self.user.GetDpiForWindow.restype = W.UINT
        self.user.GetWindowLongW.argtypes = [W.HWND, ctypes.c_int]
        self.user.GetWindowLongW.restype = W.LONG
        self.user.GetForegroundWindow.restype = W.HWND
        self.user.SetForegroundWindow.argtypes = [W.HWND]
        self.user.ShowWindow.argtypes = [W.HWND, ctypes.c_int]
        self.user.GetAncestor.argtypes = [W.HWND, W.UINT]
        self.user.GetAncestor.restype = W.HWND
        self.user.WindowFromPoint.argtypes = [POINT]
        self.user.WindowFromPoint.restype = W.HWND
        self.user.SetWindowPos.argtypes = [W.HWND, W.HWND, ctypes.c_int, ctypes.c_int,
                                          ctypes.c_int, ctypes.c_int, W.UINT]
        self.user.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
        self.user.SendInput.argtypes = [W.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
        self.user.SendInput.restype = W.UINT
        self.user.SetThreadDpiAwarenessContext.argtypes = [ctypes.c_void_p]
        self.user.SetThreadDpiAwarenessContext.restype = ctypes.c_void_p
        self.user.GetDlgCtrlID.argtypes = [W.HWND]
        self.user.SendMessageTimeoutW.argtypes = [W.HWND, W.UINT, W.WPARAM, W.LPARAM,
                                                W.UINT, W.UINT, ctypes.POINTER(ctypes.c_size_t)]
        self.enum_type = ctypes.WINFUNCTYPE(W.BOOL, W.HWND, W.LPARAM)
        self.user.EnumWindows.argtypes = [self.enum_type, W.LPARAM]
        self.user.EnumChildWindows.argtypes = [W.HWND, self.enum_type, W.LPARAM]
        self.user.SystemParametersInfoW.argtypes = [W.UINT, W.UINT, ctypes.c_void_p, W.UINT]
        self.user.MonitorFromWindow.argtypes = [W.HWND, W.DWORD]
        self.user.MonitorFromWindow.restype = W.HANDLE
        self.user.GetMonitorInfoW.argtypes = [W.HANDLE, ctypes.POINTER(MONITORINFO)]

    def _created(self):
        values = [W.FILETIME() for _ in range(4)]
        require(self.kernel.GetProcessTimes(self.process, *(ctypes.byref(v) for v in values)),
                "Owned process creation read failed.")
        return (values[0].dwHighDateTime << 32) | values[0].dwLowDateTime

    def exit_code(self):
        code = W.DWORD()
        require(self.kernel.GetExitCodeProcess(self.process, ctypes.byref(code)), "Process status read failed.")
        return code.value

    def verify_binary(self):
        require(sha256(self.executable) == self.digest, "Executable SHA256 changed.")

    def guard(self):
        require(self.exit_code() == 259 and self._created() == self.created,
                "Owned process exited or its pinned incarnation changed.")
        size = W.DWORD(32768)
        name = ctypes.create_unicode_buffer(size.value)
        require(self.kernel.QueryFullProcessImageNameW(self.process, 0, name, ctypes.byref(size)),
                "Owned EXE path read failed.")
        require(comparable_path(name.value) == comparable_path(self.executable), "Live EXE path mismatch.")
        require(self.user.IsWindow(self.hwnd) and self.owner(self.hwnd) == self.pid and
                self.text(self.hwnd) == "Versora", "Owned HWND/title mismatch.")

    def owner(self, hwnd):
        value = W.DWORD()
        self.user.GetWindowThreadProcessId(hwnd, ctypes.byref(value))
        return value.value

    def text(self, hwnd):
        value = ctypes.create_unicode_buffer(8192)
        if self.owner(hwnd) == self.pid and self.classname(hwnd) in ("Static", "Button"):
            # GetWindowText cannot reliably read another process's child controls.
            result = ctypes.c_size_t()
            require(self.user.SendMessageTimeoutW(hwnd, 0x000D, len(value),
                                                  ctypes.addressof(value), 0x0002, 1000,
                                                  ctypes.byref(result)), "Owned dialog text read failed.")
        else:
            self.user.GetWindowTextW(hwnd, value, len(value))
        return value.value

    def classname(self, hwnd):
        value = ctypes.create_unicode_buffer(256)
        self.user.GetClassNameW(hwnd, value, len(value))
        return value.value

    def rect(self, hwnd=None, client=False):
        value = RECT()
        method = self.user.GetClientRect if client else self.user.GetWindowRect
        require(method(hwnd or self.hwnd, ctypes.byref(value)), "Native rectangle read failed.")
        return {"x": value.left, "y": value.top, "width": value.right - value.left,
                "height": value.bottom - value.top}

    def snapshot(self):
        self.guard()
        placement = WINDOWPLACEMENT()
        placement.length = ctypes.sizeof(placement)
        require(self.user.GetWindowPlacement(self.hwnd, ctypes.byref(placement)), "Placement read failed.")
        client = self.rect(client=True)
        origin = POINT(0, 0)
        require(self.user.ClientToScreen(self.hwnd, ctypes.byref(origin)), "Native client origin read failed.")
        monitor = MONITORINFO()
        monitor.size = ctypes.sizeof(monitor)
        require(self.user.GetMonitorInfoW(self.user.MonitorFromWindow(self.hwnd, 2), ctypes.byref(monitor)),
                "Owned window monitor work area read failed.")
        return {"pid": self.pid, "hwnd": self.hwnd, "creationFileTime": self.created,
                "window": self.rect(), "client": self.rect(client=True),
                "clientScreen": {"x": origin.x, "y": origin.y, "width": client["width"], "height": client["height"]},
                "monitorWork": {"x": monitor.work.left, "y": monitor.work.top,
                                "width": monitor.work.right-monitor.work.left,
                                "height": monitor.work.bottom-monitor.work.top},
                "dpi": self.user.GetDpiForWindow(self.hwnd),
                "style": self.user.GetWindowLongW(self.hwnd, -16) & 0xffffffff,
                "minimized": bool(self.user.IsIconic(self.hwnd)),
                "maximized": bool(self.user.IsZoomed(self.hwnd)), "showCmd": placement.showCmd}

    def foreground(self):
        self.guard()
        require(not self.user.IsIconic(self.hwnd), "Restore the owned minimized HWND first.")
        if self.user.GetForegroundWindow() != self.hwnd:
            self.user.SetForegroundWindow(self.hwnd)
        require(self.user.GetForegroundWindow() == self.hwnd, "Owned HWND did not gain foreground.")

    def restore(self):
        self.guard()
        self.verify_binary()
        self.user.ShowWindow(self.hwnd, 9)

    def place(self, logical_width=900, logical_height=640):
        self.guard()
        self.verify_binary()
        require(not self.user.IsIconic(self.hwnd) and not self.user.IsZoomed(self.hwnd),
                "Native geometry setup requires a restored window.")
        self.user.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4))
        scale = self.user.GetDpiForWindow(self.hwnd) / 96
        outer, client = self.rect(), self.rect(client=True)
        width = round(logical_width * scale) + outer["width"] - client["width"]
        height = round(logical_height * scale) + outer["height"] - client["height"]
        work = RECT()
        require(self.user.SystemParametersInfoW(0x0030, 0, ctypes.byref(work), 0), "Desktop work area unavailable.")
        require(width + 96 <= work.right - work.left and height + 96 <= work.bottom - work.top,
                "Actual desktop lacks room for this native pointer journey.")
        x = work.left + (work.right - work.left - width) // 2
        y = work.top + (work.bottom - work.top - height) // 2
        require(self.user.SetWindowPos(self.hwnd, None, x, y, width, height, 0x0014),
                "Owned native client resize failed.")
        self.foreground()

    def screen_point(self, css_x, css_y, viewport):
        self.guard()
        self.user.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4))
        client = self.rect(client=True)
        point = POINT(round(css_x * client["width"] / viewport["width"]),
                      round(css_y * client["height"] / viewport["height"]))
        require(self.user.ClientToScreen(self.hwnd, ctypes.byref(point)), "Owned coordinate conversion failed.")
        return point.x, point.y

    def pointer(self, start, delta=(0, 0), double=False):
        """Only a verified owned point begins a bounded native mouse transaction."""
        self.user.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4))
        self.guard()
        self.verify_binary()
        self.foreground()
        target = self.user.WindowFromPoint(POINT(*start))
        require(self.user.GetAncestor(target, 2) == self.hwnd,
                "Pointer start is outside the owned native window.")
        require(self.user.SetCursorPos(*start), "Owned pointer positioning failed.")
        pressed = False

        def event(flags):
            self.guard()
            require(self.user.GetForegroundWindow() == self.hwnd,
                    "Foreground ownership changed during native pointer input.")
            packet = INPUT(0, INPUTUNION(mouse=MOUSEINPUT(0, 0, 0, flags, 0, 0)))
            require(self.user.SendInput(1, ctypes.byref(packet), ctypes.sizeof(INPUT)) == 1,
                    "Owned native mouse event was rejected.")

        try:
            for click in range(2 if double else 1):
                event(0x0002)
                pressed = True
                if delta != (0, 0):
                    for step in range(1, 17):
                        self.guard()
                        require(self.user.GetForegroundWindow() == self.hwnd,
                                "Foreground ownership changed during owned drag.")
                        require(self.user.SetCursorPos(start[0] + round(delta[0] * step / 16),
                                                       start[1] + round(delta[1] * step / 16)),
                                "Owned drag pointer motion failed.")
                        time.sleep(.025)
                else:
                    time.sleep(.04)
                event(0x0004)
                pressed = False
                if double and click == 0:
                    time.sleep(.06)
        finally:
            # Pair only our successful outstanding LEFTDOWN, even if the app or
            # foreground changed. Reapplying target guards would strand the global
            # button state. This fallback sends no new press, move or keyboard input.
            if pressed:
                packet = INPUT(0, INPUTUNION(mouse=MOUSEINPUT(0, 0, 0, 0x0004, 0, 0)))
                sent = self.user.SendInput(1, ctypes.byref(packet), ctypes.sizeof(INPUT))
                self.pointer_cleanup.append({"matchingOutstandingRelease": True,
                                             "sent": sent == 1,
                                             "foreground": self.user.GetForegroundWindow()})
                require(sent == 1, "Matching outstanding native LEFTUP cleanup was rejected.")

    def enumerate(self, parent=None):
        values = []

        @self.enum_type
        def collect(hwnd, unused):
            values.append(int(hwnd))
            return True

        if parent:
            self.user.EnumChildWindows(parent, collect, 0)
        else:
            self.user.EnumWindows(collect, 0)
        return values

    def close_dialog(self):
        self.guard()
        dialogs = [h for h in self.enumerate() if self.owner(h) == self.pid and
                   self.classname(h) == "#32770" and self.user.IsWindowVisible(h) and
                   self.text(h) == "Versora"]
        require(len(dialogs) <= 1, "Ambiguous owned close dialogs.")
        if not dialogs:
            return None
        dialog = dialogs[0]
        children = self.enumerate(dialog)
        text = [self.text(h) for h in children if self.classname(h) == "Static"]
        require(CLOSE_TEXT in text, "Owned dialog is not the unsaved-close confirmation.")
        buttons = {}
        for ident in (1, 2):
            matches = [h for h in children if self.owner(h) == self.pid and
                       self.classname(h) == "Button" and self.user.GetDlgCtrlID(h) == ident and
                       self.user.IsWindowVisible(h) and self.user.IsWindowEnabled(h)]
            require(len(matches) == 1, "Owned close dialog lacks unique OK/Cancel controls.")
            buttons[str(ident)] = {"hwnd": matches[0], "caption": self.text(matches[0])}
        return {"hwnd": dialog, "pid": self.pid, "title": self.text(dialog),
                "description": CLOSE_TEXT, "buttons": buttons, "rect": self.rect(dialog)}

    def answer_dialog(self, expected, ident):
        self.guard()
        self.verify_binary()
        current = self.close_dialog()
        require(current == expected, "Owned close dialog changed before confirmation.")
        target = current["buttons"][str(ident)]["hwnd"]
        output = ctypes.c_size_t()
        require(self.user.SendMessageTimeoutW(target, 0x00F5, 0, 0, 0x0002, 2000,
                                              ctypes.byref(output)), "Owned dialog button invocation failed.")

    def release_handle(self):
        if getattr(self, "process", None):
            self.kernel.CloseHandle(self.process)
            self.process = None


def capture_full_window(native, destination):
    """Reuse the existing owned PrintWindow helper inside this stronger live pin."""
    native.guard()
    native.verify_binary()
    command = ["powershell.exe", "-NoProfile", "-NonInteractive", "-File",
               str(TEST_ROOT / "capture_native_window.ps1"), "-ProcessId", str(native.pid),
               "-OutputPath", str(destination), "-ExpectedExecutable", str(native.executable),
               "-ExpectedExecutableSHA256", native.digest, "-Mode", "window"]
    output = subprocess.run(command, capture_output=True, timeout=30)
    require(output.returncode == 0, "Actual owned full-HWND PrintWindow capture failed.")
    native.guard()
    proof = json.loads(Path(str(destination) + ".json").read_text(encoding="utf-8-sig"))
    require(int(proof["pid"]) == native.pid and int(proof["windowHandle"]) == native.hwnd and
            comparable_path(proof["executable"]) == comparable_path(native.executable) and
            proof["title"] == "Versora" and proof["captureMode"] == "window" and
            sha256(destination) == proof["sha256"].lower(), "Full-HWND capture ownership receipt differs.")
    return proof


def process_snapshot(port=None):
    # Harmless reads relate the localhost CDP listener to this exact process tree.
    command = ("$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); "
               "$p=@(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,CreationDate); ")
    command += (f"$l=@(Get-NetTCPConnection -LocalPort {int(port)} -State Listen | "
                "Select-Object LocalAddress,LocalPort,OwningProcess); " if port is not None else "$l=@(); ")
    command += "@{processes=$p;listeners=$l}|ConvertTo-Json -Depth 5 -Compress"
    output = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", command],
                            capture_output=True, text=True, encoding="utf-8", timeout=20)
    require(output.returncode == 0, "Native process/CDP listener ownership read failed.")
    return json.loads(output.stdout.lstrip("\ufeff"))


def owned_descendants(snapshot, root_pid, root_created):
    records = {int(p["ProcessId"]): p for p in snapshot["processes"]}
    require(root_pid in records and
            abs(creation_seconds(records[root_pid]["CreationDate"]) - root_created) <= .002,
            "Current process tree root differs from the launch receipt.")
    owned = {root_pid}
    for unused in range(len(records)):
        added = {pid for pid, p in records.items() if int(p["ParentProcessId"]) in owned and
                 creation_seconds(p["CreationDate"]) >=
                 creation_seconds(records[int(p["ParentProcessId"])]["CreationDate"])} - owned
        if not added:
            break
        owned.update(added)
    return [records[pid] for pid in sorted(owned)]


async def wait_native(predicate, message, seconds=8):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        await asyncio.sleep(.05)
    raise RuntimeError(message)


async def settled(page):
    await page.evaluate("() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))")


async def viewport(page):
    return await page.evaluate("() => ({width:innerWidth,height:innerHeight,devicePixelRatio})")


async def point_for(page, native, locator):
    await verify_interactive_hits(page)
    require(await locator.count() == 1, "Native pointer target must be unique.")
    bounds = await locator.bounding_box()
    require(bounds and bounds["width"] and bounds["height"], "Native pointer target is not visible.")
    require(await locator.evaluate("""node => {
      const r=node.getBoundingClientRect();
      const hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);
      return hit===node||node.contains(hit);
    }"""), "Native pointer target is covered by another DOM control; do not click it.")
    return native.screen_point(bounds["x"] + bounds["width"] / 2,
                               bounds["y"] + bounds["height"] / 2, await viewport(page))


async def blank_header_point(page, native):
    await verify_interactive_hits(page)
    point = await page.evaluate("""() => {
      const header=document.querySelector('[data-testid=topbar]');
      if(!header)return null;
      const r=header.getBoundingClientRect(), y=r.top+r.height/2;
      for(let x=r.left+30;x<r.right-150;x+=8){
        const node=document.elementFromPoint(x,y);
        if(node&&header.contains(node)&&!node.closest('button,a,input,select,textarea,[role=button],[data-tauri-drag-region="false"],[data-resize-direction]')){
          const mark=node.closest('[data-tauri-drag-region]');
          if(mark&&mark.getAttribute('data-tauri-drag-region')==='deep')return {x,y,tag:node.tagName};
        }
      }
      return null;
    }""")
    require(point, "The native header has no clear deep drag region.")
    return native.screen_point(point["x"], point["y"], await viewport(page))


async def chrome_geometry(page):
    return await page.evaluate("""() => {
      const rect=n=>{const r=n.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};};
      const headers=[...document.querySelectorAll('[data-testid=topbar]')];
      return {viewport:{width:innerWidth,height:innerHeight},headers:headers.map(rect),
        headerTags:headers.map(n=>n.tagName),drag:headers.map(n=>n.getAttribute('data-tauri-drag-region')),
        overflow:document.documentElement.scrollWidth>innerWidth,
        controls:['minimize','maximize','close'].map(name=>{
          const nodes=[...document.querySelectorAll(`[data-testid=window-${name}]`)];
          return {name,count:nodes.length,...(nodes.length===1?{...rect(nodes[0]),tag:nodes[0].tagName,
            type:nodes[0].type,label:nodes[0].getAttribute('aria-label'),
            drag:nodes[0].closest('[data-tauri-drag-region]')?.getAttribute('data-tauri-drag-region')}: {})};
        })};
    }""")


async def verify_interactive_hits(page):
    # Only the corner coin for the page you are not on is shown, so sample the visible one.
    samples = await page.evaluate("""() => ['brand','theme-switcher','window-minimize','window-maximize','window-close',
      ...[...document.querySelectorAll('.page-dock .nav-button')].filter(n=>n.getClientRects().length).map(n=>n.dataset.testid)].map(id=>{
        const nodes=[...document.querySelectorAll(`[data-testid=${id}]`)];
        if(nodes.length!==1)return {id,count:nodes.length,points:[]};
        const node=nodes[0],r=node.getBoundingClientRect();
        const points=[{x:r.x+r.width/2,y:r.y+r.height/2},{x:r.x+r.width/2,y:r.y+1}].map(p=>{
          const hit=document.elementFromPoint(p.x,p.y);
          return {...p,owned:hit===node||node.contains(hit),hit:hit?.dataset.testid||hit?.tagName||null};
        });
        return {id,count:1,points};
      })""")
    require(all(s["count"] == 1 and len(s["points"]) == 2 and all(p["owned"] for p in s["points"])
                for s in samples), "Resize grip covers an interactive center or top-inside 1px; do not send input.")
    return samples


def validate_chrome(record):
    require(len(record["headers"]) == 1 and record["headerTags"] == ["HEADER"] and
            record["drag"] == ["deep"], "Expected one native deep-drag header.")
    header = record["headers"][0]
    require(abs(header["height"] - 48) <= .5 and abs(header["y"]) <= .5 and
            abs(header["width"] - record["viewport"]["width"]) <= .5 and
            not record["overflow"], "48px titlebar geometry or layout overflow differs.")
    for i, control in enumerate(record["controls"]):
        require(control["count"] == 1 and control["tag"] == "BUTTON" and
                control["type"] == "button" and bool(control["label"]) and control["drag"] == "false" and
                abs(control["width"] - 46) <= .5 and abs(control["height"] - 48) <= .5 and
                abs(control["y"]) <= .5 and
                abs(control["x"] - (record["viewport"]["width"] - (3-i)*46)) <= .5,
                f"Native {control['name']} control differs from the 46x48 right-edge contract.")


def validate_maximized_work_area(snapshot):
    require(snapshot["maximized"], "Native IsZoomed must prove actual maximize.")
    client, work = snapshot["clientScreen"], snapshot["monitorWork"]
    require(all(abs(client[key]-work[key]) <= 2 for key in ("x", "y", "width", "height")),
            f"Maximized client differs from the native monitor work area: {client} versus {work}")


async def maximize_accessibility(page, maximized, normal_icon):
    label = "Restore" if maximized else "Maximize"
    button = page.get_by_test_id("window-maximize")
    await page.wait_for_function("""label => {
      const b=document.querySelector('[data-testid=window-maximize]');
      return b?.getAttribute('aria-label')===label && b.title===label;
    }""", arg=label)
    require(await button.locator("svg").count() == 1, "Maximize/Restore must have one visible SVG glyph.")
    icon = await button.locator("svg").inner_html()
    require(bool(icon) and (icon != normal_icon if maximized else icon == normal_icon),
            "Actual native maximized state did not change/restore the caption glyph.")
    return {"ariaLabel": await button.get_attribute("aria-label"), "title": await button.get_attribute("title"),
            "svg": icon}


async def shell_geometry(page):
    return await page.evaluate("""() => {
      const rect=n=>{const r=n.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};};
      const shell=document.querySelector('[data-testid=app-shell]');
      const inner=document.querySelector('[data-testid=content-shell]');
      const footer=document.querySelector('[data-testid=app-footer]');
      const style=getComputedStyle(shell), sb=getComputedStyle(shell,'::-webkit-scrollbar');
      const thumb=getComputedStyle(shell,'::-webkit-scrollbar-thumb');
      return {viewport:{width:innerWidth,height:innerHeight},shell:rect(shell),content:rect(inner),footer:rect(footer),
        dock:rect(document.querySelector('[data-testid=page-dock]')),
        footerOutsideScroller:!shell.contains(footer),gutter:style.scrollbarGutter,
        shellClientWidth:shell.clientWidth,scrollTop:shell.scrollTop,
        contentMaxWidth:getComputedStyle(inner).maxWidth,
        scrollbar:{width:sb.width,height:sb.height,thumbBorder:thumb.borderTopWidth,
          thumbRadius:thumb.borderTopLeftRadius,thumbClip:thumb.backgroundClip,minHeight:thumb.minHeight}};
    }""")


def validate_shell(record, reference=None):
    size, shell, content, footer = (record[k] for k in ("viewport", "shell", "content", "footer"))
    require(record["footerOutsideScroller"] and abs(footer["height"]-36) <= .5 and
            abs(footer["x"]) <= .5 and abs(footer["width"]-size["width"]) <= .5 and
            abs(footer["y"]+footer["height"]-size["height"]) <= .5,
            "Footer must reserve 36px at the client bottom outside the scroller.")
    require(record["gutter"] == "stable both-edges" and abs(shell["x"]) <= .5 and
            abs(shell["width"]-size["width"]) <= .5 and abs(shell["y"]-48) <= .5 and
            abs(shell["y"]+shell["height"]-footer["y"]) <= .5 and
            record["contentMaxWidth"] == "912px" and
            abs(content["x"]+content["width"]/2-size["width"]/2) <= 1 and
            abs(content["width"]-min(912, size["width"]-24)) <= 1,
            "Full-window scroller/gutters or original centered 912px content width differs.")
    dock = record["dock"]
    require(abs(dock["x"]+dock["width"]-(size["width"]-28)) <= 1 and
            abs(dock["y"]+dock["height"]-(footer["y"]-20)) <= 1,
            "Corner navigation coin must sit 28px from the right, 20px above the footer.")
    bar = record["scrollbar"]
    require(bar == {"width": "12px", "height": "12px", "thumbBorder": "4px",
                    "thumbRadius": "999px", "thumbClip": "padding-box", "minHeight": "36px"},
            "Actual computed capsule scrollbar differs: " + repr(bar))
    if reference is not None:
        require(footer == reference["footer"] and shell == reference["shell"] and
                all(abs(content[k]-reference["content"][k]) <= .5 for k in ("x", "width")),
                "Footer or content-shell x geometry shifted across scrolling/theme/routes/locales.")


async def provider_proofs(page):
    return await page.evaluate("""async () => {
      const state=await window.__TAURI__.core.invoke('get_state');
      return state.providers.map(p=>({id:p.id,kind:p.kind,keyPresent:p.keyPresent,
        probePerformed:p.probePerformed,nativeDetected:p.nativeDetected,signedIn:p.signedIn,
        transportVerified:p.transportVerified,availableForAttempt:p.availableForAttempt}));
    }""")


async def wait_provider_probes(page, native, seconds=45):
    deadline = time.monotonic()+seconds
    while time.monotonic() < deadline:
        native.guard()
        states = await provider_proofs(page)
        cli = [p for p in states if p["id"].endswith("_cli")]
        ready = await page.evaluate("""ids => document.querySelector('[data-testid=main-content]').getAttribute('aria-busy')==='false' &&
          ids.every(id=>document.querySelector(`[data-testid=provider-status-${id}]`)?.dataset.probePerformed==='true')""",
                                    [p["id"] for p in cli])
        if cli and ready and all(p["probePerformed"] is True for p in cli):
            return states
        await asyncio.sleep(.2)
    raise RuntimeError("Real native initial CLI Rechecks did not settle within 45 seconds.")


async def provider_layout(page):
    return await page.evaluate("""() => [...document.querySelectorAll('.provider-row')].map(row=>{
      const id=row.dataset.testid.slice('provider-'.length), actions=row.querySelector('.provider-actions');
      const rect=n=>{const r=n.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};};
      const status=row.querySelector('.provider-status'), dot=row.querySelector('.status-dot');
      const buttons=[...actions.children].filter(n=>n.tagName==='BUTTON');
      const issues=[];
      for(const button of buttons){
        if(getComputedStyle(button).visibility==='hidden')continue;
        const bounds=button.getBoundingClientRect(), walker=document.createTreeWalker(button,NodeFilter.SHOW_TEXT);
        for(let node=walker.nextNode();node;node=walker.nextNode()){
          if(!node.textContent.trim()||node.parentElement.closest('svg'))continue;
          const range=document.createRange();range.selectNodeContents(node);
          for(const r of range.getClientRects())if(r.width&&r.height&&(r.left<bounds.left-1||r.right>bounds.right+1||r.top<bounds.top-1||r.bottom>bounds.bottom+1))issues.push(button.dataset.testid);
        }
      }
      return {id,ready:dot.classList.contains('ready'),color:getComputedStyle(dot).backgroundColor,
        readyColor:getComputedStyle(document.documentElement).getPropertyValue('--ok').trim(),
        forcedColors:matchMedia('(forced-colors:active)').matches,
        data:{probe:status.dataset.probePerformed,signedIn:status.dataset.signedIn,transport:status.dataset.transportVerified},
        actionDisplay:getComputedStyle(actions).display,actions:rect(actions),issues,
        buttons:buttons.map(n=>({id:n.dataset.testid,...rect(n),visibility:getComputedStyle(n).visibility,
          ariaHidden:n.getAttribute('aria-hidden'),tabIndex:n.tabIndex,disabled:n.disabled}))};
    })""")


def validate_provider_layout(rows, proofs, reference=None):
    by_id = {p["id"]: p for p in proofs}
    require(len(rows) == len(proofs) == len(by_id) and {r["id"] for r in rows} == set(by_id),
            "Actual provider DOM/state identifiers differ.")
    columns = None
    for row in rows:
        proof = by_id[row["id"]]
        require(all(isinstance(proof[k], bool) for k in ("probePerformed", "transportVerified", "availableForAttempt")) and
                all(proof[k] is None or isinstance(proof[k], bool) for k in ("nativeDetected", "signedIn")),
                "Native provider evidence fields have invalid types.")
        require(row["ready"] == (proof["transportVerified"] is True) and row["data"] == {
            "probe": str(proof["probePerformed"] is True).lower(),
            "signedIn": str(proof["signedIn"] is True).lower(),
            "transport": str(proof["transportVerified"] is True).lower()},
            "Provider connected dot/status must match actual native transport proof.")
        if not row["forcedColors"] and re.fullmatch(r"#[0-9A-Fa-f]{6}", row["readyColor"]):
            ready_color = "rgb(" + ", ".join(str(int(row["readyColor"][i:i+2], 16)) for i in (1, 3, 5)) + ")"
            require(row["color"] != ready_color or proof["transportVerified"] is True,
                    "Actual green status-dot pixels are claimed without native transport verification.")
        if row["id"] == "demo":
            require(not row["buttons"], "Demo provider unexpectedly has live action controls.")
            continue
        require(row["actionDisplay"] == "grid" and len(row["buttons"]) == 4 and not row["issues"],
                "Four provider action slots or localized text bounds differ.")
        buttons = row["buttons"]
        expected = [name+row["id"] for name in ("probe-provider-", "test-provider-", "edit-provider-", "remove-provider-")]
        require([b["id"] for b in buttons] == expected and
                all(abs(b["width"]-buttons[0]["width"]) <= 1 and
                    abs(b["y"]-buttons[0]["y"]) <= .5 and abs(b["height"]-buttons[0]["height"]) <= .5
                    for b in buttons), "Provider action slot order/width/alignment differs.")
        require(buttons[1]["disabled"] == (proof["availableForAttempt"] is not True),
                "Paid Test eligibility must match native availability, without executing Test.")
        removal = buttons[3]
        require((removal["visibility"] == "hidden" and removal["ariaHidden"] == "true" and
                 removal["tabIndex"] == -1 and removal["disabled"]) if not proof["keyPresent"] else
                removal["visibility"] == "visible", "Remove action did not retain its reserved slot.")
        geometry = [(b["x"], b["width"]) for b in buttons]
        if columns is None:
            columns = geometry
        require(all(abs(a-b) <= 1 for actual, expected_column in zip(geometry, columns)
                    for a, b in zip(actual, expected_column)), "Provider rows do not share four aligned action columns.")
        if reference is not None:
            baseline = next(r for r in reference if r["id"] == row["id"])
            require(all(abs(b[k]-a[k]) <= 1 for b, a in zip(buttons, baseline["buttons"])
                        for k in ("x", "width")), "Status/Recheck/locale changed reserved provider action columns.")


async def inspect_motion(page, native):
    await verify_interactive_hits(page)
    await page.get_by_test_id("settings-pane-appearance").click()
    original_reduce = await page.get_by_test_id("reduce-motion").is_checked()
    await page.get_by_test_id("reduce-motion").uncheck()
    await page.wait_for_function("() => document.documentElement.dataset.reducedMotion==='false'")
    await wait_idle(page)
    os_reduce = await page.evaluate("() => matchMedia('(prefers-reduced-motion:reduce)').matches")
    measure = """() => {
      const root=getComputedStyle(document.documentElement), button=document.querySelector('[data-testid=settings-pane-appearance]') || document.querySelector('[data-testid=nav-settings]');
      const style=getComputedStyle(button), content=document.querySelector('[data-testid=main-content]');
      const card=content.querySelector('.card'), c=getComputedStyle(content), k=card&&getComputedStyle(card);
      return {tokens:{theme:root.getPropertyValue('--theme-duration').trim(),release:root.getPropertyValue('--control-release').trim(),
          press:root.getPropertyValue('--control-press').trim()},button:{properties:style.transitionProperty,durations:style.transitionDuration,transform:style.transform},
        route:{name:c.animationName,duration:c.animationDuration,timing:c.animationTimingFunction},
        card:k?{name:k.animationName,duration:k.animationDuration}:null,themeActive:document.documentElement.dataset.themeTransition||null};
    }"""
    await page.wait_for_function("() => !document.documentElement.dataset.themeTransition")
    normal = await page.evaluate(measure)
    require(normal["tokens"] == {"theme": "280ms", "release": "180ms", "press": "80ms"},
            "Actual motion timing tokens differ.")
    if not os_reduce:
        durations = dict(zip(normal["button"]["properties"].split(", "), normal["button"]["durations"].split(", ")))
        require(durations.get("transform") == "0.18s" and durations.get("filter") == "0.18s",
                "Normal control release must use actual computed 180ms transform/filter transitions.")
    button = page.get_by_test_id("settings-pane-appearance")
    await button.hover()
    native.guard()
    await page.mouse.down()
    try:
        press = await page.evaluate(measure)
        require(all(float(value.removesuffix("s")) == (0 if os_reduce else .08)
                    for value in press["button"]["durations"].split(", ")),
                "Actual active control press must be 80ms, or disabled by OS reduction.")
    finally:
        await page.mouse.up()
    native.guard()
    await navigate(page, "translate")
    route = await page.evaluate(measure)
    if not os_reduce:
        require(route["route"]["name"] == "route-enter" and route["route"]["duration"] == "0.22s" and
                route["card"] == {"name": "route-fade", "duration": "0.2s"},
                "Actual navigation must trigger the 220ms route and 200ms card entry.")
    else:
        require(route["route"]["name"] == "none", "OS reduced motion must suppress route animation.")
    await navigate(page, "settings")
    await page.get_by_test_id("settings-pane-appearance").click()
    theme = await page.get_by_test_id("appearance-theme").get_attribute("data-value")
    native.guard()
    await page.get_by_test_id("appearance-theme-standard").click()
    await page.get_by_test_id(f'appearance-tone-{"light" if theme == "dark" else "dark"}').click()
    await page.wait_for_function("theme => document.documentElement.dataset.theme!==theme", arg=theme)
    theme_motion = await page.evaluate("""() => {
      const s=getComputedStyle(document.body);
      return {active:document.documentElement.dataset.themeTransition||null,
        properties:s.transitionProperty,durations:s.transitionDuration};
    }""")
    if not os_reduce:
        require(theme_motion["active"] == "running" and set(theme_motion["properties"].split(", ")) ==
                {"background-color", "color", "border-color", "outline-color", "fill", "stroke"} and
                all(v.strip() == "0.28s" for v in theme_motion["durations"].split(",")),
                "Actual theme change must run the color-only 280ms timeline.")
    else:
        require(theme_motion["active"] is None, "OS reduction must skip the theme timeline.")
    await page.wait_for_function("() => !document.documentElement.dataset.themeTransition")
    await wait_idle(page)
    await page.get_by_test_id("reduce-motion").check()
    await page.wait_for_function("() => document.documentElement.dataset.reducedMotion==='true'")
    await wait_idle(page)
    native.guard()
    await navigate(page, "translate")
    reduced = await page.evaluate(measure)
    require(reduced["route"]["name"] == ("none" if os_reduce else "route-fade") and
            (os_reduce or (reduced["route"]["duration"] == "0.16s" and reduced["route"]["timing"] == "linear")) and
            reduced["card"]["name"] == "none" and
            all(float(v.removesuffix("s")) == 0 for v in reduced["button"]["durations"].split(", ")),
            "App reduced motion must retain only its 160ms opacity fade; OS reduction disables it.")
    await navigate(page, "settings")
    await page.get_by_test_id("settings-pane-appearance").click()
    await page.get_by_test_id("settings-pane-appearance").hover()
    native.guard()
    await page.mouse.down()
    try:
        reduced_press = await page.evaluate(measure)
        require(reduced_press["button"]["transform"] == "none" and
                all(float(v.removesuffix("s")) == 0 for v in reduced_press["button"]["durations"].split(", ")),
                "Reduced motion must remove active control transform and transitions.")
    finally:
        await page.mouse.up()
    await navigate(page, "settings")
    await page.get_by_test_id("settings-pane-appearance").click()
    await page.get_by_test_id("appearance-theme-standard").click()
    await page.get_by_test_id(f"appearance-tone-{theme}").click()
    await page.wait_for_function("theme => document.documentElement.dataset.theme===theme", arg=theme)
    await wait_idle(page)
    reduced_theme = await page.evaluate("""() => ({active:document.documentElement.dataset.themeTransition||null,
      duration:getComputedStyle(document.body).transitionDuration})""")
    require(reduced_theme["active"] is None and
            all(float(v.strip().removesuffix("s")) == 0 for v in reduced_theme["duration"].split(",")),
            "App reduced motion must settle theme colors without a transition.")
    await page.get_by_test_id("reduce-motion").set_checked(original_reduce)
    await page.wait_for_function("reduced => document.documentElement.dataset.reducedMotion===String(reduced)", arg=original_reduce)
    await wait_idle(page)
    await page.wait_for_function("() => !document.documentElement.dataset.themeTransition")
    return {"osReducedMotion": os_reduce, "originalAppReduction": original_reduce,
            "normal": normal, "press": press, "route": route, "theme": theme_motion, "reduced": reduced,
            "reducedPress": reduced_press, "reducedTheme": reduced_theme}


async def wait_idle(page):
    # Theme destination can be painted before finishOperation restores controls.
    await page.wait_for_function("""() => document.querySelector('[data-testid=main-content]').getAttribute('aria-busy')==='false' &&
      !document.querySelector('[data-testid=theme-choice-standard]').disabled""")


async def inspect_retained_theme_and_rail(page, native):
    await page.get_by_test_id("settings-pane-purposes").click()
    await wait_idle(page)
    draft = page.get_by_test_id("purpose-instructions")
    original = await draft.input_value()
    value = original + "\nNative r40 unsaved theme/race draft."
    await draft.fill(value)
    await draft.focus()
    await draft.evaluate("n=>n.setSelectionRange(2,8)")
    handle = await draft.element_handle()
    pane = await page.get_by_test_id("settings-content-purposes").element_handle()
    layout = await page.locator(".settings-layout").element_handle()
    rail = await page.locator(".settings-rail").element_handle()
    buttons = await page.locator(".settings-rail .rail-button").element_handles()
    theme = await page.evaluate("() => document.documentElement.dataset.theme")
    cycles = []
    for unused in range(4):
        target = "light" if await page.evaluate("() => document.documentElement.dataset.theme") == "dark" else "dark"
        native.guard()
        # DOM click invokes the real delegated product action and Rust save. It
        # preserves the existing input focus so DOM replacement is observable.
        await page.evaluate("() => document.querySelector('[data-testid=theme-choice-standard]').click()")
        await page.wait_for_function("theme=>document.documentElement.dataset.theme===theme", arg=target)
        await wait_idle(page)
        retained = await handle.evaluate("""n=>({same:n.isConnected&&n===document.querySelector('[data-testid=purpose-instructions]'),
          focused:document.activeElement===n,disabled:n.disabled,start:n.selectionStart,end:n.selectionEnd,value:n.value})""")
        require(retained == {"same": True, "focused": True, "disabled": False, "start": 2, "end": 8, "value": value},
                "Repeated real theme change replaced/lost focused draft DOM, caret or enabled state.")
        require(await pane.evaluate("n=>n.isConnected&&n===document.querySelector('[data-testid=settings-content-purposes]')") and
                await layout.evaluate("n=>n.isConnected&&n===document.querySelector('.settings-layout')"),
                "Theme change must preserve the same content/settings pane.")
        cycles.append({"theme": target, "focusedDraftNodeRetained": True, "selection": [2, 8]})
    native.guard()
    target = "light" if theme == "dark" else "dark"
    await page.evaluate("""() => {
      document.querySelector('[data-testid=theme-choice-standard]').click();
      document.querySelector('[data-testid=settings-pane-glossary]').click();
    }""")
    await page.wait_for_function("theme=>document.documentElement.dataset.theme===theme", arg=target)
    await wait_idle(page)
    require(await page.get_by_test_id("new-project").is_enabled() and
            await page.get_by_test_id("glossary-project").is_enabled() and
            await page.get_by_test_id("save-glossary").is_enabled(),
            "Real theme→immediate-pane race left the desired pane disabled.")
    require(await rail.evaluate("n=>n.isConnected&&n===document.querySelector('.settings-rail')") and
            all([await b.evaluate("n=>n.isConnected&&n===document.querySelector(`[data-testid=${n.dataset.testid}]`)") for b in buttons]),
            "Settings pane change replaced retained rail/button DOM.")
    markers = await page.evaluate("""() => [...document.querySelectorAll('.rail-button')].map(n=>{
      const s=getComputedStyle(n,'::before');return {id:n.dataset.testid,active:n.classList.contains('active'),
        properties:s.transitionProperty,durations:s.transitionDuration,timing:s.transitionTimingFunction,opacity:s.opacity,transform:s.transform};
    })""")
    reduced = await page.evaluate("() => document.documentElement.dataset.reducedMotion==='true'||matchMedia('(prefers-reduced-motion:reduce)').matches")
    for marker in markers:
        if reduced:
            require(all(float(v.strip().removesuffix("s")) == 0 for v in marker["durations"].split(",")),
                    "Reduced settings marker must have no transition.")
        else:
            durations = dict(zip(marker["properties"].split(", "), marker["durations"].split(", ")))
            require(durations.get("opacity") == "0.18s" and durations.get("transform") == "0.18s",
                    "Retained settings index marker must animate opacity/scale over 180ms.")
    await page.get_by_test_id("settings-pane-purposes").click()
    require(await page.get_by_test_id("purpose-instructions").input_value() == value,
            "Real theme/pane race lost the prior isolated purpose draft.")
    await page.get_by_test_id("purpose-instructions").fill(original)
    await page.get_by_test_id("settings-pane-appearance").click()
    await page.get_by_test_id("appearance-theme-standard").click()
    await page.get_by_test_id(f"appearance-tone-{theme}").click()
    await wait_idle(page)
    await page.wait_for_function("() => !document.documentElement.dataset.themeTransition")
    return {"cycles": cycles, "sameTaskThemePaneRace": "PASS", "railNodesRetained": True, "markers": markers,
            "draftRestoredWithoutSaving": True}


async def inspect_html_remove_dialog(page, native):
    await page.get_by_test_id("settings-pane-appearance").click()
    original_reduce = await page.get_by_test_id("reduce-motion").is_checked()
    await page.get_by_test_id("reduce-motion").uncheck()
    await wait_idle(page)
    await page.get_by_test_id("settings-pane-keys").click()
    proofs = await provider_proofs(page)
    require(next(p for p in proofs if p["id"] == "openai")["keyPresent"] is False,
            "Only a fresh isolated no-key provider may receive the synthetic dialog fixture.")
    native.guard()
    await page.get_by_test_id("edit-provider-openai").click()
    key = page.get_by_test_id("provider-key-openai")
    secret_node = await key.element_handle()
    await key.fill("synthetic-native-dialog-key-not-a-live-credential")
    await page.get_by_test_id("save-provider-openai").click()
    await wait_idle(page)
    require(await secret_node.evaluate("n=>n.value===''") and
            next(p for p in await provider_proofs(page) if p["id"] == "openai")["keyPresent"] is True,
            "Actual isolated UI Save did not clear its key input and save the fixture.")
    os_reduce = await page.evaluate("() => matchMedia('(prefers-reduced-motion:reduce)').matches")
    read_dialog = """() => {
      const n=document.querySelector('[data-testid=confirm-dialog]'),s=getComputedStyle(n),b=getComputedStyle(n,'::backdrop');
      return {open:n.open,modal:n.matches(':modal'),sheet:{name:s.animationName,duration:s.animationDuration,timing:s.animationTimingFunction},
        backdrop:{name:b.animationName,duration:b.animationDuration,timing:b.animationTimingFunction}};
    }"""
    records = []
    for app_reduce in (False, True):
        if app_reduce:
            await page.get_by_test_id("settings-pane-appearance").click()
            await page.get_by_test_id("reduce-motion").check()
            await wait_idle(page)
            await page.get_by_test_id("settings-pane-keys").click()
        native.guard()
        await page.get_by_test_id("remove-provider-openai").click()
        await page.wait_for_function("() => document.querySelector('[data-testid=confirm-dialog]').open")
        dialog = await page.evaluate(read_dialog)
        require(dialog["open"] and dialog["modal"], "Remove must open the actual product HTML modal.")
        if os_reduce or app_reduce:
            require(dialog["sheet"]["name"] == dialog["backdrop"]["name"] == "none",
                    "Existing app/OS reduction must suppress dialog displacement and animations.")
        else:
            require(dialog["sheet"]["name"] == "dialog-sheet-enter" and dialog["sheet"]["duration"] == "0.38s" and
                    dialog["sheet"]["timing"].replace(" ", "") == "cubic-bezier(0.34,1.4,0.5,1)" and
                    dialog["backdrop"]["name"] == "dialog-backdrop-enter" and dialog["backdrop"]["duration"] == "0.2s",
                    "Actual normal Remove dialog must use the r40 200ms backdrop / 380ms spring sheet.")
        await page.get_by_test_id("confirm-cancel").click()
        await page.wait_for_function("() => !document.querySelector('[data-testid=confirm-dialog]').open")
        await wait_idle(page)
        require(next(p for p in await provider_proofs(page) if p["id"] == "openai")["keyPresent"] is True,
                "Actual Keep/Cancel removed the saved isolated fixture.")
        records.append({"appReduced": app_reduce, "osReduced": os_reduce, "dialog": dialog, "CancelKeptKey": True})
    native.guard()
    await page.get_by_test_id("remove-provider-openai").click()
    await page.get_by_test_id("confirm-accept").click()
    await wait_idle(page)
    require(next(p for p in await provider_proofs(page) if p["id"] == "openai")["keyPresent"] is False,
            "Actual confirmed Remove did not clean up the isolated fixture.")
    await page.get_by_test_id("settings-pane-appearance").click()
    await page.get_by_test_id("reduce-motion").set_checked(original_reduce)
    await wait_idle(page)
    return {"fixture": "actual isolated UI Save/Remove", "paidTestClicks": 0, "dialogs": records, "fixtureRemoved": True,
            "OSReducedBranch": "PASS" if os_reduce else "NOT_RUN: actual OS setting was not reduced"}


async def run(args):
    evidence = owned_test_path(args.evidence)
    require(not evidence.exists(), "Use a fresh evidence directory; preserve previous receipts.")
    proof_path = owned_test_path(args.launch_proof)
    proof = json.loads(proof_path.read_text(encoding="utf-8-sig"))
    evidence.mkdir(parents=True)
    receipt = {"status": "RUNNING", "native": False, "browserViewportEmulation": False,
               "keyboardInput": False, "processTermination": False, "realTranslation": "NOT_RUN",
               "checks": [], "errors": [], "phase": "ownership", "launchProofSHA256": sha256(proof_path),
               "r40ReadOnlyReferenceSHA256": R40_REFERENCES,
               "nativeZoom200And300": "NOT_RUN: native zoom permission is absent and zoom hotkeys are disabled; no emulation",
               "scrollbarNativePointerScreenshots": "NOT_RUN: owned native hover/active/drag supplemental evidence pending"}
    native = browser = page = None
    try:
        native = OwnedWindow(proof)
        receipt["initialNative"] = native.snapshot()
        endpoint = args.cdp or f"http://127.0.0.1:{int(proof['debugPort'])}"
        parsed = urlparse(endpoint)
        require(parsed.scheme == "http" and parsed.hostname == "127.0.0.1" and
                parsed.port == int(proof["debugPort"]) and not parsed.username and
                not parsed.password and parsed.path in ("", "/") and not parsed.query and not parsed.fragment,
                "CDP must use the exact proof's loopback HTTP debug port.")
        tree = process_snapshot(parsed.port)
        descendants = owned_descendants(tree, native.pid,
                                        native.created / 10_000_000 - 11_644_473_600)
        owned_ids = {int(item["ProcessId"]) for item in descendants}
        require(tree["listeners"] and all(int(p["OwningProcess"]) in owned_ids and
                p["LocalAddress"] in ("127.0.0.1", "::1") for p in tree["listeners"]),
                "CDP listener is outside the owned native process tree or loopback.")
        receipt["processOwnership"] = {"descendants": descendants, "listeners": tree["listeners"]}
        async with async_playwright() as playwright:
            browser = await playwright.chromium.connect_over_cdp(endpoint)
            candidates = []
            for context in browser.contexts:
                for candidate in context.pages:
                    if await candidate.title() == "Versora" and await candidate.evaluate(
                            "Boolean(window.__TAURI__?.core?.invoke && window.__TAURI__?.window?.getCurrentWindow)"):
                        candidates.append(candidate)
            require(len(candidates) == 1, "Exactly one actual Versora Tauri page is required.")
            page = candidates[0]
            page.set_default_timeout(8000)
            page.on("pageerror", lambda error: receipt["errors"].append(str(error)))
            state = await page.evaluate("window.__TAURI__.core.invoke('get_state')")
            require(state.get("testMode") and comparable_path(state["dataDir"]) ==
                    comparable_path(proof["dataDir"]), "Expected isolated native Demo profile required.")
            require(Path(proof["dataDir"]).resolve().is_relative_to(proof_path.parent),
                    "Native data profile must belong to the launch evidence.")
            require(not any(item.get("keyPresent") or
                            (item.get("configured") and item["id"] != "demo" and
                             item.get("kind") != "cli" and not item["id"].endswith("_cli"))
                            for item in state["providers"]),
                    "Owned isolated chrome profile must not contain saved API credentials.")
            require(not any(item.get("transportVerified") for item in state["providers"]
                            if item["id"] != "demo"),
                    "Initial non-Demo providers must not contain paid transport-test proofs.")
            receipt.update(native=True, url=page.url, dataDir=state["dataDir"])
            decorated = await page.evaluate("window.__TAURI__.window.getCurrentWindow().isDecorated()")
            require(decorated is False, "Native Tauri isDecorated() must prove decorated=false.")
            # Tao can retain WS_CAPTION while WM_NCCALCSIZE removes the visible
            # caption. Preserve style bits as evidence, rather than rejecting them.
            receipt["nativeDecorated"] = decorated
            receipt["initialInteractiveHits"] = await verify_interactive_hits(page)
            await navigate(page, "settings")
            await page.get_by_test_id("settings-pane-appearance").click()
            await page.get_by_test_id("interface-language").select_option("en")
            await page.wait_for_function("() => document.documentElement.lang === 'en'")
            await wait_idle(page)
            receipt["phase"] = "minimize/maximize/restore"
            if native.snapshot()["minimized"] or native.snapshot()["maximized"]:
                native.restore()
                await wait_native(lambda: not native.snapshot()["minimized"] and
                                  not native.snapshot()["maximized"], "Initial restore failed.")
            native.place()
            await settled(page)
            normal = native.snapshot()
            validate_chrome(await chrome_geometry(page))
            await verify_interactive_hits(page)
            normal_icon = await page.get_by_test_id("window-maximize").locator("svg").inner_html()
            native.guard()
            await page.get_by_test_id("window-minimize").click(no_wait_after=True)
            minimized = await wait_native(lambda: native.snapshot() if native.snapshot()["minimized"] else None,
                                          "Actual native minimize did not occur.")
            native.restore()
            await wait_native(lambda: not native.snapshot()["minimized"], "Owned minimized HWND restore failed.")
            await settled(page)
            restored = native.snapshot()
            require(restored["window"] == normal["window"], "Minimize/restore changed normal placement.")
            native.guard()
            await page.get_by_test_id("window-maximize").click()
            maximized = await wait_native(lambda: native.snapshot() if native.snapshot()["maximized"] else None,
                                          "Actual native maximize did not occur.")
            validate_maximized_work_area(maximized)
            restore_accessibility = await maximize_accessibility(page, True, normal_icon)
            await verify_interactive_hits(page)
            native.guard()
            await page.get_by_test_id("window-maximize").click()
            await wait_native(lambda: not native.snapshot()["maximized"], "Actual maximize control restore failed.")
            await settled(page)
            require(native.snapshot()["window"] == normal["window"], "Maximize/restore changed normal placement.")
            maximize_restored = await maximize_accessibility(page, False, normal_icon)
            receipt["checks"].append({"name": "native minimize/maximize/restore", "normal": normal,
                                      "minimized": minimized, "restored": restored, "maximized": maximized,
                                      "restoreAccessible": restore_accessibility, "maximizeRestored": maximize_restored})
            receipt["phase"] = "drag and double click"
            before = native.snapshot()
            await asyncio.to_thread(native.pointer, await blank_header_point(page, native), (48, 32))
            await settled(page)
            after = native.snapshot()
            require(abs(after["window"]["x"] - before["window"]["x"]) >= 20 and
                    abs(after["window"]["y"] - before["window"]["y"]) >= 12 and
                    after["window"]["width"] == before["window"]["width"] and
                    after["window"]["height"] == before["window"]["height"],
                    "Actual deep header drag did not move the native HWND.")
            receipt["checks"].append({"name": "actual native header drag", "before": before, "after": after})
            await asyncio.to_thread(native.pointer, await blank_header_point(page, native), double=True)
            await wait_native(lambda: native.snapshot()["maximized"], "Header double click did not maximize.")
            validate_maximized_work_area(native.snapshot())
            await maximize_accessibility(page, True, normal_icon)
            await settled(page)
            await asyncio.to_thread(native.pointer, await blank_header_point(page, native), double=True)
            await wait_native(lambda: not native.snapshot()["maximized"], "Header double click did not restore.")
            await maximize_accessibility(page, False, normal_icon)
            receipt["checks"].append({"name": "actual native header double-click maximize/restore"})
            receipt["phase"] = "eight resize handles"
            for name, (dx, dy, direction) in RESIZE.items():
                native.place()
                await settled(page)
                handle = page.get_by_test_id("window-resize-" + name)
                require(await handle.get_attribute("data-resize-direction") == direction,
                        f"Resize {name} direction contract differs.")
                before = native.snapshot()
                scale = before["dpi"] / 96
                await asyncio.to_thread(native.pointer, await point_for(page, native, handle),
                                        (round(dx*scale), round(dy*scale)))
                await settled(page)
                after = native.snapshot()
                require((not dx or after["client"]["width"] >= before["client"]["width"] + 8) and
                        (not dy or after["client"]["height"] >= before["client"]["height"] + 8) and
                        (dx or after["client"]["width"] == before["client"]["width"]) and
                        (dy or after["client"]["height"] == before["client"]["height"]),
                        f"Actual native {name} resize did not change its intended dimensions.")
                receipt["checks"].append({"name": "native resize " + name, "before": before, "after": after})
            receipt["phase"] = "interactive nondrag"
            native.place()
            await settled(page)
            before = native.snapshot()["window"]
            brand = page.get_by_test_id("brand")
            require(await brand.get_attribute("data-tauri-drag-region") == "false",
                    "Interactive brand must explicitly block deep dragging.")
            # The nested text exercises inherited explicit-false handling.
            await asyncio.to_thread(native.pointer, await point_for(page, native, brand.locator("b")), (12, 0))
            await settled(page)
            require(native.snapshot()["window"] == before, "Nested interactive brand text dragged the window.")
            navigation = page.locator(".page-dock .nav-button:visible")
            await asyncio.to_thread(native.pointer, await point_for(page, native, navigation), (12, 0))
            await settled(page)
            require(native.snapshot()["window"] == before, "Interactive navigation unexpectedly dragged the window.")
            native.guard()
            await navigation.click()
            # The coin may have opened Translate; make sure Settings is open.
            await navigate(page, "settings")
            await page.get_by_test_id("settings-pane-appearance").click()
            require(native.snapshot()["window"] == before, "Interactive navigation moved native placement.")
            receipt["checks"].append({"name": "interactive brand/nested text/nav do not drag", "window": before})
            receipt["phase"] = "800px light/dark"
            native.place(800, 640)
            await page.wait_for_function("() => innerWidth === 800")
            await page.get_by_test_id("interface-language").select_option("en")
            await page.wait_for_function("() => document.documentElement.lang === 'en'")
            shell_reference = await shell_geometry(page)
            validate_shell(shell_reference)
            for theme in ("light", "dark"):
                native.guard()
                await page.get_by_test_id("appearance-theme-standard").click()
                await page.get_by_test_id(f"appearance-tone-{theme}").click()
                await page.wait_for_function("theme => document.documentElement.dataset.theme === theme", arg=theme)
                await wait_idle(page)
                await settled(page)
                record = await chrome_geometry(page)
                validate_chrome(record)
                shell_record = await shell_geometry(page)
                validate_shell(shell_record, shell_reference)
                require(record["viewport"]["width"] == 800, "Actual native 800px minimum differs.")
                await page.screenshot(path=str(evidence / f"native-chrome-{theme}-800-client.png"))
                full_capture = await asyncio.to_thread(capture_full_window, native,
                                                      evidence / f"native-chrome-{theme}-800-full.png")
                receipt["checks"].append({"name": theme + " actual 800px chrome", "geometry": record,
                                          "shellFooterScrollbar": shell_record,
                                          "native": native.snapshot(), "fullWindowCapture": full_capture})
            receipt["phase"] = "real native provider Recheck and four action slots"
            await page.get_by_test_id("settings-pane-keys").click()
            # The product automatically probes initially unprobed CLIs. Observe
            # those real results before performing exactly one explicit Recheck.
            before_proofs = await wait_provider_probes(page, native)
            before_rows = await provider_layout(page)
            validate_provider_layout(before_rows, before_proofs)
            first_cli = next((p for p in before_proofs if p["id"].endswith("_cli")), None)
            require(first_cli is not None and before_rows[0]["id"] == first_cli["id"],
                    "First Keys entry must be the actual first native CLI provider.")
            require(all(p["transportVerified"] is False for p in before_proofs),
                    "Fresh profile must have no paid transport Test proofs.")
            native.guard()
            await page.get_by_test_id("probe-provider-"+first_cli["id"]).click()
            await page.wait_for_function("() => document.querySelector('[data-testid=main-content]').getAttribute('aria-busy')==='false'", timeout=45000)
            after_proofs = await provider_proofs(page)
            after_rows = await provider_layout(page)
            validate_provider_layout(after_rows, after_proofs, before_rows)
            require(all(p["transportVerified"] is False for p in after_proofs),
                    "Native availability Recheck must not manufacture a transport Test proof.")
            receipt["checks"].append({"name": "one actual first-CLI Recheck; no paid Test; stable four slots",
                                      "provider": first_cli["id"], "explicitRecheckClicks": 1, "paidTestClicks": 0,
                                      "beforeNative": before_proofs, "afterNative": after_proofs,
                                      "beforeRows": before_rows, "afterRows": after_rows})
            receipt["phase"] = "12-locale provider action/footer widths"
            locale_layouts = []
            for locale in LOCALES:
                native.guard()
                await page.get_by_test_id("settings-pane-appearance").click()
                await page.get_by_test_id("interface-language").select_option(locale)
                await page.wait_for_function("locale => document.documentElement.lang===locale", arg=locale)
                await wait_idle(page)
                await page.evaluate("async () => await document.fonts.ready")
                appearance_shell = await shell_geometry(page)
                validate_shell(appearance_shell, shell_reference)
                await page.get_by_test_id("settings-pane-keys").click()
                await page.wait_for_function("() => document.querySelector('[data-testid=main-content]').getAttribute('aria-busy')==='false'")
                proofs = await provider_proofs(page)
                rows = await provider_layout(page)
                validate_provider_layout(rows, proofs, after_rows)
                provider_shell = await shell_geometry(page)
                validate_shell(provider_shell, shell_reference)
                validate_chrome(await chrome_geometry(page))
                await verify_interactive_hits(page)
                locale_layouts.append({"locale": locale, "native": proofs, "rows": rows,
                                       "appearanceShell": appearance_shell, "providerShell": provider_shell})
                print(json.dumps({"nativeChromeLocale": locale, "actualWidth": 800,
                                  "providerRows": len(rows), "paidTestClicks": 0}), flush=True)
            receipt["checks"].append({"name": "all 12 native locale action slots/footer/gutters", "layouts": locale_layouts})
            await page.get_by_test_id("settings-pane-appearance").click()
            await page.get_by_test_id("interface-language").select_option("en")
            await page.wait_for_function("() => document.documentElement.lang==='en'")
            await wait_idle(page)
            await page.get_by_test_id("settings-pane-order").click()
            order_proofs = await provider_proofs(page)
            order_dots = await page.evaluate("""() => [...document.querySelectorAll('.order-row')].map(row=>({
              id:row.dataset.testid.slice('order-row-'.length),ready:row.querySelector('.status-dot').classList.contains('ready')}))""")
            require({r["id"] for r in order_dots} == {p["id"] for p in order_proofs} and
                    all(row["ready"] == (next(p for p in order_proofs if p["id"]==row["id"])["transportVerified"] is True)
                        for row in order_dots), "Order connected dots must match native transport verification.")
            validate_shell(await shell_geometry(page), shell_reference)
            await navigate(page, "translate")
            validate_shell(await shell_geometry(page), shell_reference)
            await navigate(page, "settings")
            await page.get_by_test_id("settings-pane-keys").click()
            receipt["checks"].append({"name": "Order status proof and footer stable across real routes", "orderDots": order_dots})
            receipt["phase"] = "fixed caption while content scrolls"
            await page.get_by_test_id("settings-pane-keys").click()
            await settled(page)
            scroll_measure = """() => {
              const shell=document.querySelector('[data-testid=app-shell]');
              const content=document.querySelector('[data-testid=main-content]');
              const a=shell.getBoundingClientRect(), b=content.getBoundingClientRect();
              return {scrollTop:shell.scrollTop,scrollHeight:shell.scrollHeight,clientHeight:shell.clientHeight,
                shell:{x:a.x,width:a.width},content:{x:b.x,width:b.width}};
            }"""
            scroll_before = await page.evaluate(scroll_measure)
            require(scroll_before["scrollHeight"] > scroll_before["clientHeight"],
                    "The actual provider pane must have real scrollable content for this check.")
            chrome_before = await chrome_geometry(page)
            footer_before = await shell_geometry(page)
            validate_shell(footer_before, shell_reference)
            native.guard()
            await page.get_by_test_id("app-shell").hover(position={"x": 400, "y": 200})
            await page.mouse.wheel(0, 400)
            await page.wait_for_function("() => document.querySelector('[data-testid=app-shell]').scrollTop > 0")
            await settled(page)
            scroll_after = await page.evaluate(scroll_measure)
            chrome_after = await chrome_geometry(page)
            footer_after = await shell_geometry(page)
            validate_shell(footer_after, shell_reference)
            validate_chrome(chrome_after)
            require(chrome_after == chrome_before and
                    scroll_after["shell"] == scroll_before["shell"] and
                    scroll_after["content"] == scroll_before["content"],
                    "Scrolling content moved the fixed caption or shifted content/header horizontally.")
            receipt["checks"].append({"name": "scroll keeps fixed 48px caption and stable x geometry",
                                      "before": scroll_before, "after": scroll_after, "chrome": chrome_after,
                                      "footerBefore": footer_before, "footerAfter": footer_after})
            receipt["phase"] = "actual normal and reduced motion"
            motion_record = await inspect_motion(page, native)
            validate_shell(await shell_geometry(page), shell_reference)
            receipt["checks"].append({"name": "computed actual scrollbar and motion parity", "motion": motion_record})
            receipt["phase"] = "retained theme draft DOM and real pane race"
            retained_record = await inspect_retained_theme_and_rail(page, native)
            receipt["checks"].append({"name": "four real theme toggles retain focused draft DOM; real pane race; retained 180ms rail",
                                      "retained": retained_record})
            receipt["phase"] = "actual isolated UI Remove dialog normal/reduced parity"
            dialog_record = await inspect_html_remove_dialog(page, native)
            receipt["checks"].append({"name": "actual UI Save/Remove/Cancel; 200ms backdrop and 380ms sheet; no paid Test",
                                      "dialog": dialog_record})
            validate_shell(await shell_geometry(page), shell_reference)
            receipt["nativeZoom100"] = {"status": "PASS", "native": native.snapshot(), "shell": await shell_geometry(page),
                                        "browserViewportEmulation": False, "cssZoomOrPageScaleEmulation": False}
            receipt["phase"] = "close cancel unsaved"
            await page.get_by_test_id("settings-pane-glossary").click()
            draft = "native-chrome-unsaved-project"
            await page.get_by_test_id("new-project").fill(draft)
            # The product owns set_unsaved; never manufacture that native flag.
            native.guard()
            await page.get_by_test_id("window-close").click(no_wait_after=True)
            dialog = await wait_native(native.close_dialog, "Actual unsaved CloseRequested dialog did not appear.")
            receipt["closeCancelDialog"] = dialog
            native.answer_dialog(dialog, 2)
            await wait_native(lambda: native.close_dialog() is None, "Native Cancel did not dismiss the dialog.")
            await settled(page)
            require(await page.get_by_test_id("new-project").input_value() == draft and native.exit_code() == 259,
                    "Cancel lost the unsaved draft or closed the owned app.")
            receipt["checks"].append({"name": "native close Cancel retains unsaved draft", "native": native.snapshot()})
            require(not receipt["errors"], "Native page errors occurred before close: " + repr(receipt["errors"]))
            receipt["phase"] = "close confirm and cleanup"
            final_tree = process_snapshot(parsed.port)
            closing_children = owned_descendants(final_tree, native.pid,
                                                native.created / 10_000_000 - 11_644_473_600)
            native.guard()
            await page.get_by_test_id("window-close").click(no_wait_after=True)
            dialog = await wait_native(native.close_dialog, "Second actual unsaved-close dialog did not appear.")
            receipt["closeConfirmDialog"] = dialog
            native.answer_dialog(dialog, 1)
            await wait_native(lambda: native.exit_code() != 259, "Confirmed native close did not exit normally.", 20)
            require(native.exit_code() == 0 and not native.user.IsWindow(native.hwnd),
                    "Confirmed native close left its HWND or returned a nonzero exit code.")
            # Read-only descendant cleanup check pins each incarnation; reused PIDs
            # are unrelated and are never targeted or treated as surviving children.
            deadline = time.monotonic() + 15
            remaining = []
            while True:
                live = process_snapshot()["processes"]
                remaining = [child for child in closing_children if any(
                    int(p["ProcessId"]) == int(child["ProcessId"]) and
                    abs(creation_seconds(p["CreationDate"]) - creation_seconds(child["CreationDate"])) <= .002
                    for p in live)]
                if not remaining or time.monotonic() >= deadline:
                    break
                await asyncio.sleep(.25)
            receipt["cleanup"] = {"exitCode": native.exit_code(), "hwndGone": True,
                                  "observedDescendants": closing_children, "remainingSameIncarnation": remaining,
                                  "forcedTermination": False}
            require(not remaining, "Owned native/WebView descendants survived confirmed close.")
            receipt["checks"].append({"name": "native confirm clean exit without forced termination"})
            receipt.update(status="PASS", phase="complete")
    except Exception as error:
        receipt.update(status="FAILED", error=f"{type(error).__name__}: {error}")
        if page is not None and not page.is_closed() and native is not None and native.exit_code() == 259:
            try:
                receipt["failureNative"] = native.snapshot()
                if not receipt["failureNative"]["minimized"]:
                    await page.screenshot(path=str(evidence / "native-chrome-failure-client.png"), timeout=3000)
            except Exception as screenshot_error:
                receipt["failureCaptureError"] = str(screenshot_error)
        raise
    finally:
        if native is not None:
            receipt["outstandingPointerCleanup"] = native.pointer_cleanup
        receipt["completedUTC"] = datetime.now(timezone.utc).isoformat()
        (evidence / "native-chrome-journey.json").write_text(json.dumps(receipt, indent=2), encoding="utf-8")
        if browser is not None:
            try:
                await browser.close()  # Disconnect CDP; never close a native page/context.
            except Exception:
                pass  # The confirmed native close already removed the transport.
        if native is not None:
            native.release_handle()
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--launch-proof", required=True)
    parser.add_argument("--evidence", required=True)
    parser.add_argument("--cdp", help="Must match the launch proof's loopback debug port.")
    arguments = parser.parse_args()
    try:
        outcome = asyncio.run(run(arguments))
        print(json.dumps({"status": outcome["status"], "native": outcome["native"],
                          "checks": len(outcome["checks"]), "realTranslation": outcome["realTranslation"],
                          "evidence": str(Path(arguments.evidence).resolve()/"native-chrome-journey.json")}, indent=2))
    except Exception as error:
        print(json.dumps({"status": "FAILED", "error": f"{type(error).__name__}: {error}"}))
        raise SystemExit(1)
