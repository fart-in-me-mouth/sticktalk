import unittest
from qualify import metric_pass

class Gates(unittest.TestCase):
    def metric(self, **kw):
        m=dict(fps_by_second=[60]*60, frames=3600, seconds=60.001, drift_ms=0.1, encoder_reconfigurations=0)
        m.update(kw)
        return m
    def passes(self, m, quality='1440', role='viewer_b'):
        return metric_pass(m, dict(quality=quality, seconds=60), role)
    def test_first_bin_is_never_discarded(self):
        self.assertFalse(self.passes(self.metric(fps_by_second=[11]+[60]*59,frames=3551)))
    def test_drift_is_strict_including_1080_baseline(self):
        self.assertFalse(self.passes(self.metric(drift_ms=20)))
        self.assertFalse(self.passes(self.metric(drift_ms=5),quality='1080'))
        self.assertTrue(self.passes(self.metric(drift_ms=4.999),quality='1080'))
    def test_1080_uses_stricter_minimum(self):
        m=self.metric(fps_by_second=[54]+[60]*59,frames=3594)
        self.assertTrue(self.passes(m))
        self.assertFalse(self.passes(m,quality='1080'))
    def test_missing_bins_or_inconsistent_count_fail(self):
        self.assertFalse(self.passes(self.metric(fps_by_second=[60]*59)))
        self.assertFalse(self.passes(self.metric(frames=3601)))
        self.assertFalse(self.passes(self.metric(seconds=59.999)))
    def test_encoder_thrashing_and_valid_case(self):
        self.assertFalse(self.passes(self.metric(encoder_reconfigurations=1),role='sharer'))
        self.assertTrue(self.passes(self.metric()))

if __name__=='__main__': unittest.main()
