import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("benchmark_runtime", pathlib.Path(__file__).with_name("benchmark_runtime.py"))
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


class CalibrationMetricTests(unittest.TestCase):
    def test_character_error_is_local_for_a_small_chinese_edit(self):
        wer, cer = benchmark.error_rates("今天我们讨论计划", "今天我们讨论计画")
        self.assertEqual(wer, 1.0)
        self.assertAlmostEqual(cer, 1 / 8)

    def test_diacritic_marks_are_not_silently_erased_from_quality_scores(self):
        _, cer = benchmark.error_rates("किताब", "कताब")
        self.assertGreater(cer, 0)

    def test_unlabelled_reference_is_rejected(self):
        with self.assertRaises(ValueError):
            benchmark.error_rates("   ", "invented words")

    def test_actual_tail_latency_is_reported(self):
        median, p95 = benchmark.median_p95([10, 11, 100])
        self.assertEqual(median, 11)
        self.assertEqual(p95, 100)

    def test_empty_and_non_finite_timings_are_rejected(self):
        for values in ([], [10, float("nan")], [10, float("inf")], [-1, 10]):
            with self.subTest(values=values), self.assertRaises(ValueError):
                benchmark.median_p95(values)


class CalibrationWinnerTests(unittest.TestCase):
    @staticmethod
    def candidate(candidate_id="passing", **overrides):
        return {"id": candidate_id, "median_ms": 100, "p95_ms": 110,
                "wer": 0.0, "cer": 0.0, "character_language": False,
                "repeats": 3, "spill_detected": False, "labelled": True,
                **overrides}

    def test_faster_candidate_with_worse_accuracy_cannot_win(self):
        accurate = self.candidate("accurate")
        inaccurate = self.candidate("inaccurate", median_ms=50, p95_ms=55, wer=0.04)
        self.assertEqual(benchmark.select_candidate([inaccurate, accurate]), accurate)

    def test_fastest_similarly_accurate_candidate_wins(self):
        accurate = self.candidate("accurate")
        comparable = self.candidate("comparable", median_ms=70, p95_ms=80, wer=0.004)
        self.assertEqual(benchmark.select_candidate([accurate, comparable]), comparable)

    def test_unlabelled_under_sampled_unstable_and_spilling_rows_are_rejected(self):
        invalid = [
            self.candidate("unlabelled", labelled=False),
            self.candidate("under-sampled", repeats=2),
            self.candidate("unstable", p95_ms=500),
            self.candidate("spilling", spill_detected=True),
            self.candidate("invalid-tail", p95_ms=50),
            self.candidate("zero-time", median_ms=0, p95_ms=0),
            self.candidate("non-finite", wer=float("nan")),
            self.candidate("negative-error", wer=-0.1),
        ]
        for row in invalid:
            with self.subTest(candidate=row["id"]):
                self.assertIsNone(benchmark.select_candidate([row]))

    def test_character_languages_use_character_error_and_require_quality(self):
        acceptable = self.candidate("small-edit", character_language=True, wer=1.0, cer=0.01)
        unacceptable = self.candidate("large-edit", character_language=True, wer=0.0, cer=0.1)
        self.assertEqual(benchmark.select_candidate([unacceptable, acceptable]), acceptable)


if __name__ == "__main__":
    unittest.main()
