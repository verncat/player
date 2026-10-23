#!/usr/bin/env python3
"""Read Player's existing assets.small_url through the official Discord Social SDK.

Download and extract the C++ Social SDK first, then run:
  python3 scripts/read-discord-link.py --sdk /path/to/discord_social_sdk

Requires a C++20 compiler, no Python packages. Currently macOS/Linux.
The application must enable Public Client and the OAuth2 redirect
http://127.0.0.1/callback. Sign-in uses the SDK's normal OAuth consent screen.
"""

import argparse
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile


def locate(root, name, override=None):
    if override:
        path = Path(override).expanduser().resolve()
        if not path.is_file():
            raise ValueError(f'File not found: {path}')
        return path
    matches = sorted(root.rglob(name))
    release = [path for path in matches if 'release' in (part.lower() for part in path.parts)]
    if release:
        matches = release
    aliases = ('arm64', 'aarch64') if platform.machine().lower() in ('arm64', 'aarch64') else ('x86_64', 'x64')
    native = [path for path in matches if any(part.lower() in aliases for part in path.parts)]
    if native:
        matches = native
    if not matches:
        raise ValueError(f'{name} not found under {root}. Extract the C++ SDK for this platform.')
    if len(matches) > 1:
        raise ValueError(f'Multiple {name} files found. Use --library or --include to select the SDK build.')
    return matches[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--sdk', default=os.environ.get('DISCORD_SOCIAL_SDK'), help='Extracted C++ Social SDK folder')
    parser.add_argument('--application-id', type=int, default=1562037965537157120)
    parser.add_argument('--timeout', type=int, default=120, help='Seconds for sign-in and reading presence')
    parser.add_argument('--library', help='Explicit path to the SDK dylib/so for this CPU architecture')
    parser.add_argument('--include', help='Explicit path to discordpp.h')
    args = parser.parse_args()
    if not args.sdk:
        parser.error('Download the C++ Social SDK from your app in Discord Developer Portal, then pass --sdk PATH.')
    if sys.platform not in ('darwin', 'linux'):
        parser.error('This launcher currently supports macOS/Linux.')
    if not 0 < args.application_id < 2**64 or not 0 < args.timeout <= 600:
        parser.error('Use a valid Application ID and a timeout between 1 and 600 seconds.')
    compiler = shutil.which('clang++') or shutil.which('c++')
    if not compiler:
        parser.error('A C++20 compiler is required. On macOS install Xcode Command Line Tools.')

    try:
        root = Path(args.sdk).expanduser().resolve()
        if not root.is_dir():
            raise ValueError(f'SDK folder not found: {root}')
        # The full SDK also contains mobile/framework copies of this header.
        desktop_header = root / 'include' / 'discordpp.h'
        header = locate(root, 'discordpp.h', args.include or
                        (desktop_header if desktop_header.is_file() else None))
        library = locate(root, 'libdiscord_partner_sdk.dylib' if sys.platform == 'darwin'
                         else 'libdiscord_partner_sdk.so', args.library)
        source = Path(__file__).with_name('read-discord-link.cpp')
        with tempfile.TemporaryDirectory(prefix='player-discord-read-') as directory:
            executable = Path(directory) / 'read-discord-link'
            command = [compiler, '-std=c++20', '-pthread', '-I', str(header.parent),
                       str(source), str(library), f'-Wl,-rpath,{library.parent}', '-o', str(executable)]
            print('Building the Social SDK reader...', file=sys.stderr)
            subprocess.run(command, check=True)
            return subprocess.run([str(executable), str(args.application_id), str(args.timeout)],
                                  timeout=args.timeout + 10).returncode
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f'Error: {error}', file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == '__main__':
    sys.exit(main())
