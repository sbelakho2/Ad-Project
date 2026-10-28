import unittest
import numpy as np
from benchmark import exact,adjusted,groupmom
from rerandomize import run

class IndependentMathTests(unittest.TestCase):
    def test_constant_population_counterexample(self):
        e=exact()[0]
        z=next(r for r in e["effects"] if r["effect"]==0.)
        self.assertAlmostEqual(z["current_final_keep"],.269341150588,places=7)
        self.assertLess(z["corrected_975_final_keep"],.03)
    def test_sum_of_squares_adjustment_matches_direct_values(self):
        x=np.array([[0.,1.,2.,3.,4.,5.]])
        y=np.array([[2.,4.,1.,5.,6.,2.]])
        mask=np.array([[True,False,True,False,True,False]])
        g=groupmom(x,y,mask);center=np.mean(x,axis=1)
        m,v=adjusted(g,.7,center,1.03)
        z=(1.03*y-.7*(x-center[:,None]))[mask]
        self.assertAlmostEqual(m[0],z.mean())
        self.assertAlmostEqual(v[0],z.var(ddof=1))
    def test_revenue_zero_roster_is_preserved_and_seed_replays(self):
        y=np.array([0.]*500+[1.]*500)
        a=run(y,.5,300,0.,792123);b=run(y,.5,300,0.,792123)
        self.assertEqual(a,b)
        self.assertEqual(a["eligible_users"],1000)
        self.assertEqual(a["zero_revenue_users"],500)
    def test_uniform_player_roster_has_no_effect(self):
        a=run(np.ones(2000),.25,2000,0.,792123)
        self.assertEqual(a["rates"]["roster_mean_975"]["k"],0)
if __name__=="__main__":unittest.main()
