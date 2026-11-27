"""The `tridi` command a pip install puts on PATH: the bundled program itself."""

import os
import sys

from . import _binary


def main():
    exe = str(_binary())
    os.execv(exe, [exe, *sys.argv[1:]])


if __name__ == "__main__":
    main()
