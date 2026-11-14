"""The report must reject a false-green native process collector."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
import summarize


class SummaryGates(unittest.TestCase):
    def result(self, *, minimum=59, drift=0, reconfigurations=0, missing=False):
        with tempfile.TemporaryDirectory(prefix="sticktalk-metric-gates-") as tmp:
            root = Path(tmp)
            case = root / "case"
            case.mkdir()
            def write(path, value):
                path.write_text(json.dumps(value), encoding="utf-8")
            write(root / "matrix-results.json", [{"Case": "case", "ExitCode": 0}])
            write(case / "run.json", dict(source="synthetic", quality="1440", fps=60, seconds=600))
            for role in ("sharer", "viewer_b", "viewer_c"):
                if missing and role == "viewer_c":
                    continue
                write(case / f"{role}.metrics.json", dict(
                    frames=36000, seconds=600, fps_by_second=[minimum, 60], drift_ms=drift,
                    encoder_reconfigurations=reconfigurations if role == "sharer" else None,
                    decode_errors=0, cpu_machine_pct=0, rss_bytes=0, private_bytes=0, payload_bytes=0,
                ))
            previous = summarize.ROOT
            summarize.ROOT = root
            try:
                with contextlib.redirect_stdout(io.StringIO()):
                    summarize.summarize()
            finally:
                summarize.ROOT = previous
            return json.loads((root / "acceptance-summary.json").read_text(encoding="utf-8"))[0]

    def test_zero_collector_exit_cannot_hide_eleven_fps(self):
        self.assertFalse(self.result(minimum=11)["passed"])

    def test_minimum_boundary_can_pass(self):
        self.assertTrue(self.result(minimum=45, drift=19.99)["passed"])

    def test_drift_boundary_fails(self):
        self.assertFalse(self.result(drift=20)["passed"])

    def test_reconfiguration_fails(self):
        self.assertFalse(self.result(reconfigurations=1)["passed"])

    def test_missing_viewer_fails(self):
        self.assertFalse(self.result(missing=True)["passed"])


if __name__ == "__main__":
    unittest.main()
