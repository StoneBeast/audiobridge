#!/usr/bin/env python3
"""AudioBridge 模拟器/真机 UI 驱动小工具（基于 uiautomator dump）。

用法（serial 可用环境变量 ANDROID_SERIAL 或 --serial 指定）：
  python scripts/emu-drive.py --serial 127.0.0.1:16384 dump
  python scripts/emu-drive.py tap 开始发送          # 按 text/content-desc 精确匹配
  python scripts/emu-drive.py tapc 立即            # 子串匹配
  python scripts/emu-drive.py sethost 127.0.0.1    # 清空并填写 IP 输入框
  python scripts/emu-drive.py shot out.png         # 截图
  python scripts/emu-drive.py waitfor 已连接 15    # 等待文本出现（秒）
"""
import argparse
import re
import subprocess
import sys
import time

UI_XML = "/sdcard/audiobridge_ui.xml"


def adb(serial, *args, timeout=30):
    cmd = ["adb"]
    if serial:
        cmd += ["-s", serial]
    cmd += list(args)
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)


def dump(serial):
    adb(serial, "shell", "uiautomator", "dump", UI_XML)
    time.sleep(0.3)
    r = adb(serial, "shell", "cat", UI_XML)
    return r.stdout or ""


def nodes(xml):
    for m in re.finditer(r"<node[^>]*>", xml):
        tag = m.group(0)

        def attr(name, tag=tag):
            mm = re.search(name + r'="([^"]*)"', tag)
            return mm.group(1) if mm else ""

        yield {
            "text": attr("text"),
            "desc": attr("content-desc"),
            "bounds": attr("bounds"),
            "class": attr("class"),
        }


def center(bounds):
    mm = re.match(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", bounds)
    if not mm:
        return None
    l, t, r, b = map(int, mm.groups())
    return (l + r) // 2, (t + b) // 2


def find(serial, needle, exact=False):
    xml = dump(serial)
    for n in nodes(xml):
        hay = [n["text"], n["desc"]]
        for h in hay:
            if not h:
                continue
            if (exact and needle == h) or (not exact and needle in h):
                return n
    return None


def tap(serial, needle, exact=False, tries=6, delay=1.0):
    for _ in range(tries):
        n = find(serial, needle, exact)
        if n:
            xy = center(n["bounds"])
            if xy:
                adb(serial, "shell", "input", "tap", str(xy[0]), str(xy[1]))
                print(f"tapped '{needle}' @ {xy}")
                time.sleep(delay)
                return True
        time.sleep(delay)
    print(f"NOT FOUND: {needle}")
    return False


def waitfor(serial, needle, seconds=15):
    deadline = time.time() + seconds
    while time.time() < deadline:
        n = find(serial, needle)
        if n:
            print(f"found '{needle}'")
            return True
        time.sleep(1)
    print(f"TIMEOUT waiting for: {needle}")
    return False


def sethost(serial, text):
    # 点击获得焦点 -> 光标移到末尾 -> 连续退格清空 -> 输入新值
    if not tap(serial, "IP 地址", tries=4, delay=0.5):
        return False
    adb(serial, "shell", "input", "keyevent", "123")
    for _ in range(20):
        adb(serial, "shell", "input", "keyevent", "67")
    adb(serial, "shell", "input", "text", text)
    print(f"set host to {text}")
    return True


def shot(serial, path):
    adb(serial, "shell", "screencap", "-p", "/sdcard/audiobridge_shot.png")
    subprocess.run(["adb", "-s", serial, "pull", "/sdcard/audiobridge_shot.png", path],
                   capture_output=True, text=True, timeout=30)
    print(f"saved {path}")


def list_texts(serial):
    xml = dump(serial)
    for n in nodes(xml):
        if n["text"] or n["desc"]:
            print(f"text={n['text']!r} desc={n['desc']!r} bounds={n['bounds']}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--serial", default=None)
    ap.add_argument("cmd")
    ap.add_argument("args", nargs="*")
    a = ap.parse_args()
    serial = a.serial
    if a.cmd == "dump":
        list_texts(serial)
    elif a.cmd == "tap":
        sys.exit(0 if tap(serial, a.args[0], exact=True) else 1)
    elif a.cmd == "tapc":
        sys.exit(0 if tap(serial, a.args[0], exact=False) else 1)
    elif a.cmd == "waitfor":
        sys.exit(0 if waitfor(serial, a.args[0], int(a.args[1]) if len(a.args) > 1 else 15) else 1)
    elif a.cmd == "sethost":
        sys.exit(0 if sethost(serial, a.args[0]) else 1)
    elif a.cmd == "shot":
        shot(serial, a.args[0])
    else:
        print(__doc__)


if __name__ == "__main__":
    main()
