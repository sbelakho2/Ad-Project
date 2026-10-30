"""The Python half of the A3 composition twin agrees with the golden vector."""
import unittest

import a3_fee_twin


class A3FeeTwinTest(unittest.TestCase):
    def test_bound_fee_matches_the_shared_fixture(self):
        self.assertEqual(a3_fee_twin.main(), 0)


if __name__ == "__main__":
    unittest.main()
