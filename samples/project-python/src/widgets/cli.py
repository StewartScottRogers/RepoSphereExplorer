"""The ``widgets`` command line entry point."""

from __future__ import annotations

import argparse
import sys


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="widgets", description="List the floor's tooling widgets."
    )
    parser.add_argument("--count", action="store_true", help="print only the count")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    widgets = ["indexer", "reporter", "notifier"]
    if args.count:
        print(len(widgets))
    else:
        for widget in widgets:
            print(widget)
    return 0


if __name__ == "__main__":
    sys.exit(main())
