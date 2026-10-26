#!/usr/bin/env python3
"""A/A calibration of the proposed speed+RSS gate: the same build twice (two copies), interleaved, N runs per side per
trial, median wall and max... peak RSS per side, ratio B/A. Repeated TRIALS times per corpus.

usage: aa.py BIN CORPORA_DIR OUT.json [--trials 20] [--runs 15]
"""
import argparse
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

SRC = {
    'mantine': 'mantine/packages/@mantine/core/src',
    'chakra': 'chakra-ui/packages/react/src',
    'shadcn': 'shadcn-ui/apps/v4',
    'antd': 'ant-design/components',
    'spectrum': 'react-spectrum/packages',
}


def measure(cmd):
    start = time.perf_counter()
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    _, status, usage = os.wait4(proc.pid, 0)
    wall = (time.perf_counter() - start) * 1000
    code = os.waitstatus_to_exitcode(status)
    if code not in (0, 1):
        sys.exit(f'{cmd} exited {code}')
    rss = usage.ru_maxrss / (1024 * 1024 if sys.platform == 'darwin' else 1024)
    return wall, rss


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('bin')
    ap.add_argument('corpora')
    ap.add_argument('out')
    ap.add_argument('--trials', type=int, default=20)
    ap.add_argument('--runs', type=int, default=15)
    ap.add_argument('--only')
    ap.add_argument('--srcs', help='JSON file mapping corpus name to its absolute source directory')
    ap.add_argument('--no-deps', action='store_true')
    args = ap.parse_args()

    tmp = tempfile.mkdtemp(prefix='aa-')
    a, b = os.path.join(tmp, 'a-caliper'), os.path.join(tmp, 'b-caliper')
    shutil.copy2(args.bin, a)
    shutil.copy2(args.bin, b)
    result = {'host': platform.node(), 'machine': platform.machine(), 'runs': args.runs, 'corpora': {}}
    for name, rel in SRC.items():
        if args.only and name != args.only:
            continue
        src = json.load(open(args.srcs))[name] if args.srcs else os.path.join(args.corpora, rel)
        cmd = lambda binary: [binary, 'extract', '--json', '-s', src, '--no-cache', *(['--no-load-dependencies'] if args.no_deps else [])]
        for binary in (a, b):  # warm the page cache and the binary
            measure(cmd(binary))
        trials = []
        for t in range(args.trials):
            wa, wb, ra, rb = [], [], [], []
            for i in range(args.runs):
                # alternate which side goes first so order bias hits both alike
                order = (a, b) if i % 2 == 0 else (b, a)
                for binary in order:
                    w, r = measure(cmd(binary))
                    (wa if binary == a else wb).append(w)
                    (ra if binary == a else rb).append(r)
            trials.append({
                'wall_a': statistics.median(wa), 'wall_b': statistics.median(wb),
                'rss_a': statistics.median(ra), 'rss_b': statistics.median(rb),
                'wall_ratio': statistics.median(wb) / statistics.median(wa),
                'rss_ratio': statistics.median(rb) / statistics.median(ra),
                # a cheaper alternative: min of runs
                'wall_min_ratio': min(wb) / min(wa),
                'wall_a_all': wa, 'wall_b_all': wb,
            })
            print(name, t, round(trials[-1]['wall_ratio'], 3), round(trials[-1]['rss_ratio'], 3), flush=True)
        result['corpora'][name] = trials
        with open(args.out, 'w') as f:
            json.dump(result, f)
    shutil.rmtree(tmp, ignore_errors=True)


main()
