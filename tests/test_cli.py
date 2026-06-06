import json

from click.testing import CliRunner

from hieronymus.cli import main


def test_init_series_outputs_json_and_creates_database(tmp_path):
    data_root = tmp_path / "hieronymus"
    runner = CliRunner()

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "init-series",
            "only-sense-online",
            "--title",
            "Only Sense Online",
        ],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload == {
        "slug": "only-sense-online",
        "database_path": str(data_root / "series" / "only-sense-online.sqlite"),
    }
    assert (data_root / "series" / "only-sense-online.sqlite").exists()
