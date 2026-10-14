# Apex Lift Engine re-run kit

Export the same report from your MAX or LevelPlay dashboard, fill
`apex-lift-rerun-template.csv` (columns `day,arm,revenue,users`), and feed it
to the engine's `rerun` function. It returns the same lift and lower bound to
the rounding, with every daily log ratio in the output. When your export has
no `users` column, pass the test arm's allocated share instead; MAX reports
revenue per test group without user counts.
