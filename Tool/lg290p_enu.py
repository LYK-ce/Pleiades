#!/usr/bin/env python3
"""Survey-in UM960, relay RTCM to LG290P, and print the rover position in ENU."""

import argparse
import math
import os
import select
import subprocess
import time


WGS84_A = 6_378_137.0
WGS84_F = 1 / 298.257223563
WGS84_E2 = WGS84_F * (2 - WGS84_F)
DEFAULT_GEOID = -9.535
QUALITY = {
    0: "INVALID",
    1: "SINGLE",
    2: "DGPS",
    4: "RTK_FIXED",
    5: "RTK_FLOAT",
    6: "ESTIMATED",
    7: "BASE_ORIGIN",
}


def parse_gga(line):
    line = line.strip()
    if not line.startswith("$") or "*" not in line:
        return None
    payload, supplied = line[1:].split("*", 1)
    checksum = 0
    for char in payload:
        checksum ^= ord(char)
    try:
        if checksum != int(supplied[:2], 16):
            return None
    except ValueError:
        return None

    fields = payload.split(",")
    if fields[0] not in ("GNGGA", "GPGGA") or not fields[2] or not fields[4]:
        return None

    def degrees(value, hemisphere):
        raw = float(value)
        whole = int(raw // 100)
        result = whole + (raw - whole * 100) / 60
        return -result if hemisphere in ("S", "W") else result

    try:
        return {
            "utc": fields[1],
            "lat": degrees(fields[2], fields[3]),
            "lon": degrees(fields[4], fields[5]),
            "quality": int(fields[6] or 0),
            "satellites": int(fields[7] or 0),
            "hdop": float(fields[8]) if fields[8] else math.nan,
            "alt_msl": float(fields[9]) if fields[9] else math.nan,
            "geoid": float(fields[11]) if fields[11] else math.nan,
            "age": float(fields[13]) if fields[13] else math.nan,
        }
    except (ValueError, IndexError):
        return None


def ecef(lat_deg, lon_deg, height):
    lat, lon = map(math.radians, (lat_deg, lon_deg))
    sin_lat, cos_lat = math.sin(lat), math.cos(lat)
    radius = WGS84_A / math.sqrt(1 - WGS84_E2 * sin_lat * sin_lat)
    return (
        (radius + height) * cos_lat * math.cos(lon),
        (radius + height) * cos_lat * math.sin(lon),
        (radius * (1 - WGS84_E2) + height) * sin_lat,
    )


def enu(base_lat, base_lon, base_height, lat, lon, height):
    x0, y0, z0 = ecef(base_lat, base_lon, base_height)
    x, y, z = ecef(lat, lon, height)
    dx, dy, dz = x - x0, y - y0, z - z0
    lat0, lon0 = map(math.radians, (base_lat, base_lon))
    east = -math.sin(lon0) * dx + math.cos(lon0) * dy
    north = (
        -math.sin(lat0) * math.cos(lon0) * dx
        - math.sin(lat0) * math.sin(lon0) * dy
        + math.cos(lat0) * dz
    )
    up = (
        math.cos(lat0) * math.cos(lon0) * dx
        + math.cos(lat0) * math.sin(lon0) * dy
        + math.sin(lat0) * dz
    )
    return east, north, up


def self_test():
    sample = "$GNGGA,063450.200,4005.23407724,N,11613.24217273,E,5,32,0.75,41.438,M,-9.535,M,0.2,0000*70"
    fix = parse_gga(sample)
    assert fix and fix["quality"] == 5 and abs(fix["lat"] - 40.0872346207) < 1e-9
    assert max(map(abs, enu(40.0, 116.0, 10.0, 40.0, 116.0, 10.0))) < 1e-6
    _, north, _ = enu(0.0, 0.0, 0.0, 0.00001, 0.0, 0.0)
    assert 1.10 < north < 1.12
    print("self-test: OK")


def send_base_command(port, command):
    subprocess.run(
        ["sudo", "-n", "tee", port],
        input=(command + "\r\n").encode(),
        stdout=subprocess.DEVNULL,
        check=True,
    )


def base_lines(process, seconds):
    """Read complete ASCII lines from the base without blocking on binary data."""
    fd = process.stdout.fileno()
    buffer = b""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        ready, _, _ = select.select([fd], [], [], min(1.0, deadline - time.monotonic()))
        if not ready:
            continue
        chunk = os.read(fd, 4096)
        if not chunk:
            return
        buffer += chunk
        while b"\n" in buffer:
            line, buffer = buffer.split(b"\n", 1)
            yield line.decode("ascii", errors="ignore")


def auto_base(args):
    """Survey-in at the current site and return the new base GGA origin."""
    reader = subprocess.Popen(
        ["sudo", "-n", "stdbuf", "-o0", "cat", args.base_port],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    try:
        send_base_command(args.base_port, "UNLOG COM1")
        send_base_command(args.base_port, f"MODE BASE TIME {args.survey_seconds}")
        send_base_command(args.base_port, "GPGGA COM1 1")
        print(f"survey-in: waiting {args.survey_seconds}s for the new base origin...", flush=True)

        survey_end = time.monotonic() + args.survey_seconds
        deadline = survey_end + 30
        candidates = []
        while time.monotonic() < deadline:
            for line in base_lines(reader, min(2.0, deadline - time.monotonic())):
                fix = parse_gga(line)
                if not fix:
                    continue
                if fix["quality"] != 7:
                    candidates.clear()
                    continue
                candidates.append(fix)
                if len(candidates) >= 2 and time.monotonic() >= survey_end:
                    previous, current = candidates[-2:]
                    east, north, up = enu(
                        previous["lat"], previous["lon"], previous["alt_msl"],
                        current["lat"], current["lon"], current["alt_msl"],
                    )
                    if math.hypot(east, north) < 0.5 and abs(up) < 0.5:
                        print(
                            f"survey-in: origin {current['lat']:.11f}, "
                            f"{current['lon']:.11f}, {current['alt_msl']:.4f} m MSL",
                            flush=True,
                        )
                        return current
            time.sleep(0.1)
        raise RuntimeError("Survey-in timed out; keep the base antenna still with clear sky")
    finally:
        stop([reader])

def stop(processes):
    for process in reversed(processes):
        if process.poll() is None:
            process.terminate()
    for process in reversed(processes):
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()


def run(args):
    target = f"{args.user}@{args.host}"
    ssh = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", target]
    subprocess.run(
        ["sudo", "-n", "stty", "-F", args.base_port, str(args.baud), "raw", "-echo"],
        check=True,
    )
    subprocess.run(
        ssh + ["sudo", "-n", "stty", "-F", args.rover_port, str(args.baud), "raw", "-echo"],
        check=True,
    )

    base_fix = auto_base(args)
    send_base_command(args.base_port, "UNLOG COM1 GPGGA")
    args.base_lat = base_fix["lat"]
    args.base_lon = base_fix["lon"]
    args.base_alt_msl = base_fix["alt_msl"]
    args.base_geoid = base_fix["geoid"] if math.isfinite(base_fix["geoid"]) else DEFAULT_GEOID
    for command in (
        "RTCM1006 COM1 10",
        "RTCM1033 COM1 10",
        "RTCM1074 COM1 1",
        "RTCM1084 COM1 1",
        "RTCM1094 COM1 1",
        "RTCM1124 COM1 1",
    ):
        send_base_command(args.base_port, command)

    base = subprocess.Popen(
        ["sudo", "-n", "stdbuf", "-o0", "cat", args.base_port],
        stdout=subprocess.PIPE,
    )
    relay = subprocess.Popen(
        ssh + ["sudo", "-n", "tee", args.rover_port],
        stdin=base.stdout,
        stdout=subprocess.DEVNULL,
    )
    base.stdout.close()
    rover = subprocess.Popen(
        ssh + ["sudo", "-n", "stdbuf", "-o0", "cat", args.rover_port],
        stdout=subprocess.PIPE,
    )
    processes = [base, relay, rover]

    print("utc          state       cm   east(m)  north(m)     up(m)  sats  hdop  age(s)")
    last_print = 0.0
    try:
        for raw in rover.stdout:
            fix = parse_gga(raw.decode("ascii", errors="ignore"))
            now = time.monotonic()
            if not fix or now - last_print < args.interval:
                continue
            last_print = now
            geoid = fix["geoid"] if math.isfinite(fix["geoid"]) else args.base_geoid
            east, north, up = enu(
                args.base_lat,
                args.base_lon,
                args.base_alt_msl + geoid,
                fix["lat"],
                fix["lon"],
                fix["alt_msl"] + geoid,
            )
            state = QUALITY.get(fix["quality"], f"Q{fix['quality']}")
            print(
                f"{fix['utc']:<12} {state:<11} {'YES' if fix['quality'] == 4 else 'NO ':<3} "
                f"{east:9.3f} {north:9.3f} {up:9.3f} "
                f"{fix['satellites']:5d} {fix['hdop']:5.2f} {fix['age']:7.1f}",
                flush=True,
            )
    except KeyboardInterrupt:
        pass
    finally:
        stop(processes)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="10.12.35.83")
    parser.add_argument("--user", default="jetson")
    parser.add_argument("--base-port", default="/dev/ttyUSB0")
    parser.add_argument("--rover-port", default="/dev/ttyUSB2")
    parser.add_argument("--baud", type=int, default=460800)
    parser.add_argument("--interval", type=float, default=0.5)
    parser.add_argument("--survey-seconds", type=int, default=120)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        run(args)


if __name__ == "__main__":
    main()
