#!/usr/bin/env python3
"""The five public repos Caliper is measured on, fetched at pinned commits. Shared by weekly.py and bench.py.

`setup` fetches each repo, installs its dependencies with its own package manager and lockfile (lifecycle scripts off,
caches under the corpora directory's parent), and prunes tests, stories and demos from the source directory. With
--sparse it fetches only the source directory and installs nothing: enough for `just bench`, which turns dependency
loading off.

Usage: scripts/corpora.py setup [--corpora DIR] [--corpus NAME]... [--sparse] [--install-node-dir DIR]
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_DIR = os.path.join(ROOT, '.corpora', 'repos')

CORPORA = [
    {
        'name': 'mantine',
        'repo': 'mantinedev/mantine',
        'commit': 'e47e45af06427b3b2497100c2b849935c23af30d',
        'clone': 'mantine',
        'src': 'packages/@mantine/core/src',
        'manager': 'yarn',
    },
    {
        'name': 'chakra',
        'repo': 'chakra-ui/chakra-ui',
        'commit': 'fa9696f328ea5a0df8685c6d8460edbd779b63b9',
        'clone': 'chakra-ui',
        'src': 'packages/react/src',
        'manager': 'pnpm',
    },
    {
        'name': 'shadcn',
        'repo': 'shadcn-ui/ui',
        'commit': 'ba1503c64472b82256d9de95e2e62c5901a38e8b',
        'clone': 'shadcn-ui',
        'src': 'apps/v4',
        'manager': 'pnpm',
    },
    {
        'name': 'antd',
        'repo': 'ant-design/ant-design',
        'commit': 'ba7ccbaa7b4b58ac1ff8e6fe2e78dfa10bfb5538',
        'clone': 'ant-design',
        'src': 'components',
        # ant-design commits no lockfile and sets package-lock=false, so there is nothing to freeze against.
        'manager': 'npm',
    },
    {
        'name': 'spectrum',
        'repo': 'adobe/react-spectrum',
        'commit': '797762ed3526ea12f089e321bd3891066affad70',
        'clone': 'react-spectrum',
        'src': 'packages',
        'manager': 'yarn',
    },
]

COREPACK_VERSION = '0.34.0'
PRUNED_DIRS = {
    '__tests__', '__snapshots__', 'test', 'tests', 'stories', '__stories__', 'demo', '__demo__', 'docs', 'e2e',
    'dist', 'build', '.storybook', 'examples',
}
PRUNED_FILE = re.compile(r'\.(test|spec|stories|story)\.|\.d\.ts$')
MARKER = '.corpus-complete'


def selected(names):
    unknown = set(names) - {c['name'] for c in CORPORA}
    if unknown:
        sys.exit(f"unknown corpus: {', '.join(sorted(unknown))}")
    return [c for c in CORPORA if not names or c['name'] in names]


def run_once(args, stdout=subprocess.DEVNULL):
    """Wall seconds and peak RSS in MiB of one process."""
    start = time.perf_counter()
    proc = subprocess.Popen(args, stdout=stdout, stderr=subprocess.DEVNULL)
    _, status, usage = os.wait4(proc.pid, 0)
    elapsed = time.perf_counter() - start
    code = os.waitstatus_to_exitcode(status)
    if code != 0:
        raise RuntimeError(f"{' '.join(args)} exited with {code}")
    # ru_maxrss is bytes on macOS and kilobytes on Linux.
    return elapsed, usage.ru_maxrss / (2**20 if sys.platform == 'darwin' else 1024)


def package_manager_env(base):
    caches = os.path.join(base, 'caches')
    # corepack downloads CommonJS bundles (yarn 4's `yarn.js`). Its home lives under this repository, whose root
    # package.json says `"type": "module"`, so Node would load them as ES modules, where `require` does not exist.
    corepack_home = os.path.join(caches, 'corepack')
    os.makedirs(corepack_home, exist_ok=True)
    with open(os.path.join(corepack_home, 'package.json'), 'w') as manifest:
        manifest.write('{"type": "commonjs"}\n')
    env = dict(os.environ)
    env.update({
        'COREPACK_HOME': corepack_home,
        'COREPACK_ENABLE_DOWNLOAD_PROMPT': '0',
        'COREPACK_ENABLE_AUTO_PIN': '0',
        'YARN_CACHE_FOLDER': os.path.join(caches, 'yarn'),
        'YARN_GLOBAL_FOLDER': os.path.join(caches, 'yarn-global'),
        'YARN_ENABLE_GLOBAL_CACHE': '0',
        'YARN_ENABLE_SCRIPTS': '0',
        'YARN_NODE_LINKER': 'node-modules',
        'YARN_ENABLE_TELEMETRY': '0',
        'npm_config_cache': os.path.join(caches, 'npm'),
        'PATH': os.path.join(base, 'tools', 'bin') + os.pathsep + os.environ.get('PATH', ''),
    })
    return env


def ensure_package_managers(base, env, log):
    """pnpm and yarn shims from corepack, at the versions each repo's `packageManager` pins."""
    bin_dir = os.path.join(base, 'tools', 'bin')
    if os.path.exists(os.path.join(bin_dir, 'pnpm')):
        return
    os.makedirs(bin_dir, exist_ok=True)
    corepack = shutil.which('corepack')
    if corepack is None:
        # Node 25 and later no longer bundle corepack.
        tools = os.path.join(base, 'tools')
        subprocess.run(['npm', 'install', '--prefix', tools, '--no-audit', '--no-fund', f'corepack@{COREPACK_VERSION}'],
                       env=env, stdout=log, stderr=log, check=True)
        corepack = os.path.join(tools, 'node_modules', '.bin', 'corepack')
    subprocess.run([corepack, 'enable', '--install-directory', bin_dir, 'pnpm', 'yarn'], env=env, check=True)


def git(*args, cwd):
    subprocess.run(['git', *args], cwd=cwd, check=True, stdout=subprocess.DEVNULL)


def fetch(corpus, checkout, sparse):
    shutil.rmtree(checkout, ignore_errors=True)
    os.makedirs(checkout)
    git('init', '-q', cwd=checkout)
    git('remote', 'add', 'origin', f"https://github.com/{corpus['repo']}.git", cwd=checkout)
    flags = []
    if sparse:
        git('sparse-checkout', 'set', corpus['src'], cwd=checkout)
        flags = ['--filter=blob:none']
    # Network steps retry: one dropped connection shouldn't fail a scheduled run.
    for attempt in range(3):
        try:
            git('fetch', '-q', '--depth', '1', *flags, 'origin', corpus['commit'], cwd=checkout)
            git('checkout', '-q', 'FETCH_HEAD', cwd=checkout)
            return
        except subprocess.CalledProcessError:
            if attempt == 2:
                raise
            time.sleep(5)


def install(corpus, checkout, base, env, log):
    caches = os.path.join(base, 'caches')
    pnpm = ['--store-dir', os.path.join(caches, 'pnpm-store'), f"--config.cache-dir={os.path.join(caches, 'pnpm-cache')}"]
    commands = {
        'pnpm': [['pnpm', 'install', '--frozen-lockfile', '--ignore-scripts', *pnpm],
                 ['pnpm', 'install', '--no-frozen-lockfile', '--ignore-scripts', *pnpm]],
        'yarn': [['yarn', 'install', '--immutable', '--mode=skip-build'],
                 ['yarn', 'install', '--no-immutable', '--mode=skip-build']],
        'npm': [['npm', 'install', '--ignore-scripts', '--no-audit', '--no-fund']],
    }[corpus['manager']]
    # The lockfile first; a lockfile the pinned package manager rejects falls back to resolving again.
    for command in commands:
        log.write(f"\n$ {' '.join(command)}\n")
        log.flush()
        if subprocess.run(command, cwd=checkout, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log).returncode == 0:
            return
    raise RuntimeError(f"{corpus['name']}: dependency install failed, see {log.name}")


def prune(src):
    """What neither tool should document, inside the source directory only. node_modules is never entered."""
    for current, dirs, files in os.walk(src, topdown=True):
        keep = []
        for d in dirs:
            if d == 'node_modules':
                continue
            if d in PRUNED_DIRS:
                shutil.rmtree(os.path.join(current, d))
            else:
                keep.append(d)
        dirs[:] = keep
        for f in files:
            if PRUNED_FILE.search(f):
                os.remove(os.path.join(current, f))


def ready(corpus, corpora_dir, sparse):
    """The source directory when the corpus is fetched at its pinned commit (installed, unless `sparse`), else None."""
    checkout = os.path.join(corpora_dir, corpus['clone'])
    try:
        with open(os.path.join(checkout, MARKER)) as f:
            have = f.read()
    except OSError:
        return None
    wanted = corpus['commit'] + ('' if sparse else ':installed')
    if have == wanted or (sparse and have.startswith(corpus['commit'])):
        return os.path.join(checkout, corpus['src'])
    return None


def source_dir(corpus, corpora_dir, sparse=True):
    """`ready`, fetching the corpus first if it is not (installed corpora are never fetched here: use `setup`)."""
    found = ready(corpus, corpora_dir, sparse)
    if found:
        return found
    checkout = os.path.join(corpora_dir, corpus['clone'])
    print(f"{corpus['name']}: fetching {corpus['commit'][:12]}", file=sys.stderr, flush=True)
    fetch(corpus, checkout, sparse=True)
    with open(os.path.join(checkout, MARKER), 'w') as f:
        f.write(corpus['commit'])
    return os.path.join(checkout, corpus['src'])


def setup(args):
    corpora_dir = os.path.abspath(args.corpora)
    base = os.path.dirname(corpora_dir)
    os.makedirs(os.path.join(base, 'logs'), exist_ok=True)
    env = package_manager_env(base)
    if args.sparse:
        for corpus in selected(args.corpus):
            source_dir(corpus, corpora_dir)
        return
    with open(os.path.join(base, 'logs', 'corepack.log'), 'w') as log:
        ensure_package_managers(base, env, log)
    # The corpora's own package managers can need an older Node than this repository's (Yarn 4.18, which Mantine and
    # React Spectrum pin, crashes under Node 26). Only their installs run under it.
    if args.install_node_dir:
        env['PATH'] = os.path.abspath(args.install_node_dir) + os.pathsep + env['PATH']
    failed = []
    for corpus in selected(args.corpus):
        if ready(corpus, corpora_dir, sparse=False):
            print(f"{corpus['name']}: ready at {corpus['commit'][:12]}")
            continue
        checkout = os.path.join(corpora_dir, corpus['clone'])
        log_path = os.path.join(base, 'logs', f"install-{corpus['name']}.log")
        try:
            print(f"{corpus['name']}: fetching and installing with {corpus['manager']}", flush=True)
            fetch(corpus, checkout, sparse=False)
            with open(log_path, 'w') as log:
                install(corpus, checkout, base, env, log)
            prune(os.path.join(checkout, corpus['src']))
            with open(os.path.join(checkout, MARKER), 'w') as f:
                f.write(corpus['commit'] + ':installed')
        except (subprocess.CalledProcessError, RuntimeError, OSError) as error:
            # One corpus failing must not hide the others: say which one and why, then carry on with the rest.
            failed.append(corpus['name'])
            print(f"{corpus['name']}: setup FAILED ({error})", file=sys.stderr, flush=True)
    if failed:
        sys.exit(f"setup failed for: {', '.join(failed)}; see the install logs in {os.path.join(base, 'logs')}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('setup', help='fetch, install and prune the corpora')
    p.add_argument('--corpora', default=DEFAULT_DIR)
    p.add_argument('--corpus', action='append', default=[])
    p.add_argument('--sparse', action='store_true', help='fetch the source directory only; install nothing')
    p.add_argument('--install-node-dir', help="a directory with the `node` the corpora's installs should use")
    setup(parser.parse_args())


if __name__ == '__main__':
    main()
