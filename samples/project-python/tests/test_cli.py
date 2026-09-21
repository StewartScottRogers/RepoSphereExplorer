from widgets.cli import build_parser, main


def test_build_parser_accepts_count_flag():
    parser = build_parser()
    args = parser.parse_args(["--count"])
    assert args.count is True


def test_main_lists_widgets(capsys):
    assert main([]) == 0
    out = capsys.readouterr().out
    assert "indexer" in out
