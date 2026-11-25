import unittest
from backendbench import summarize


class BackendBenchTests(unittest.TestCase):
    def test_only_measured_values_contribute(self):
        base = dict(framework="MACH",model="fixture",task=1,elapsed_s=1.5,
                    functional_pass=True,security_pass=True)
        measured = dict(base,run=1,input_tokens=10,usage_source="test fixture")
        result, = summarize([measured,dict(base,run=2,security_pass=False)])
        self.assertEqual(result["input_tokens"], {"median":10,"n":1})
        self.assertEqual(result["security_pass_rate"], .5)
        self.assertNotIn("output_tokens",result)
        with self.assertRaises(ValueError):
            summarize([dict(measured,usage_source="")])
        with self.assertRaises(ValueError):
            summarize([measured,measured])
