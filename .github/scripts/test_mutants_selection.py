import unittest

from mutants_selection import selection_patterns, validate_selection


class SelectionTests(unittest.TestCase):
    patterns = [r"crates/example/src/lib.rs:.*\b(run|extract)\b"]

    def test_reads_literal_regexes_without_interpreting_backslashes(self):
        config = "examine_re = [\n    '" + self.patterns[0] + "',\n]\n"
        self.assertEqual(selection_patterns(config), self.patterns)

    def test_rejects_missing_empty_and_unsupported_pattern_lists(self):
        for config in ["", "examine_re = [\n]\n", 'examine_re = [\n"unsupported",\n]\n']:
            with self.subTest(config=config), self.assertRaises(ValueError):
                selection_patterns(config)

    def test_accepts_original_function_and_extracted_helper(self):
        validate_selection([
            {"name": "crates/example/src/lib.rs:10:5: replace run -> bool with true"},
            {"name": "crates/example/src/lib.rs:20:5: delete ! in extract"},
        ], self.patterns)

    def test_rejects_empty_selection_after_a_rename(self):
        with self.assertRaisesRegex(ValueError, "No mutants selected"):
            validate_selection([], self.patterns)

    def test_rejects_struct_field_mutants_that_bypass_tool_filters(self):
        for name in [
            "crates/example/src/other.rs:10:5: delete field x in run",
            "crates/example/src/lib.rs:10:5: delete field x in unrelated",
        ]:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "outside"):
                validate_selection([{"name": name}], self.patterns)


if __name__ == "__main__":
    unittest.main()
