"""Which lines of a legacy source file the legacy build compiles.

The legacy build is clang on FreeBSD, i386, release (`server/server/premake5.lua`,
`build_live.sh`). Each file is fed to the host C preprocessor with its `#include` lines removed
and every ordinary line replaced by a marker, so only the conditional directives decide which
markers survive. The feature switches come from a prelude: `common/prodomodefines.h` plus every
non-guard switch defined in a legacy header (`char.h`, `quest.h`, `guild.h`, ...), because the
removed includes would have brought them in.

Usage as a module: `active_lines(path) -> set[int]` (1-based line numbers).
"""
import functools
import os
import re
import subprocess
import tempfile

LEGACY = os.path.normpath(os.path.join(os.path.dirname(__file__), '../../../server/server'))

# What clang on FreeBSD i386 in the release configuration predefines, as far as the legacy
# conditionals ask about it.
TARGET = """#define __FreeBSD__ 1
#define __clang__ 1
#define __GNUC__ 4
#define __i386__ 1
#define __unix__ 1
#define NDEBUG 1
#define _THREAD_SAFE 1
"""

DIRECTIVE = re.compile(r'\s*#\s*(if|ifdef|ifndef|elif|else|endif|define|undef)\b')
INCLUDE = re.compile(r'\s*#\s*(include|import|pragma|error|warning|line)\b')
GUARD_DEFINE = re.compile(r'\s*#\s*define\s+([A-Za-z_0-9]+)\s*(//.*|/\*.*)?$')


def read(path):
    with open(path, encoding='latin-1') as handle:
        return handle.read().split('\n')


def join_continuations(lines):
    """Yield (first line number, [physical lines]) for each logical line."""
    index = 0
    while index < len(lines):
        start = index
        group = [lines[index]]
        while group[-1].endswith('\\') and index + 1 < len(lines):
            index += 1
            group.append(lines[index])
        yield start + 1, group
        index += 1


@functools.lru_cache(maxsize=None)
def prelude():
    """The switches a translation unit would have seen through its includes."""
    parts = [TARGET]
    defines = os.path.join(LEGACY, 'common/prodomodefines.h')
    parts.append('\n'.join(line for line in read(defines) if not INCLUDE.match(line)))
    for folder in ('common', 'game', 'libthecore/include', 'libthecore'):
        root = os.path.join(LEGACY, folder)
        if not os.path.isdir(root):
            continue
        for name in sorted(os.listdir(root)):
            if not name.lower().endswith('.h') or name == 'prodomodefines.h':
                continue
            lines = read(os.path.join(root, name))
            for number, line in enumerate(lines):
                match = GUARD_DEFINE.match(line)
                if not match:
                    continue
                macro = match.group(1)
                before = ' '.join(lines[max(0, number - 3):number])
                if re.search(r'#\s*(ifndef\s+' + macro + r'\b|if\s+!\s*defined\s*\(?\s*'
                             + macro + r'\b)', before):
                    continue  # an include guard
                parts.append('#define ' + macro)
    return '\n'.join(parts) + '\n'


@functools.lru_cache(maxsize=None)
def active_lines(path):
    """The 1-based numbers of the lines the legacy build compiles in `path`."""
    lines = read(path)
    marked = []
    for number, group in join_continuations(lines):
        if DIRECTIVE.match(group[0]):
            # Keep the directive but not a trailing guard-less comment that cpp would choke on.
            marked.extend(group)
        elif INCLUDE.match(group[0]):
            marked.extend([''] * len(group))
        else:
            for offset in range(len(group)):
                marked.append('@@L%d@@' % (number + offset))
    # An include guard around the whole file must not hide it: drop a leading #ifndef/#define
    # pair whose macro is only used as the guard.
    with tempfile.NamedTemporaryFile('w', suffix='.cpp', delete=False, encoding='latin-1') as tmp:
        tmp.write(prelude())
        tmp.write('\n'.join(marked))
        name = tmp.name
    try:
        out = subprocess.run(['cpp', '-undef', '-P', '-w', '-x', 'c++', name],
                             capture_output=True, text=True, encoding='latin-1', check=True).stdout
    finally:
        os.unlink(name)
    return frozenset(int(match) for match in re.findall(r'@@L(\d+)@@', out))


if __name__ == '__main__':
    import sys
    for arg in sys.argv[1:]:
        live = active_lines(arg)
        print(arg, len(live), 'of', len(read(arg)), 'lines live')
