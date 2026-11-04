# Release evidence report

Run:

```sh
BENCHMARK_EVENTS=1000000 OCEL_BENCHMARK_EVENTS=100000 packaging/release-report.sh
```

The script runs the complete workspace test suite plus case-centric and both
object-centric release-mode benchmark shapes on the actual product analysis
paths. It writes a machine-identified Markdown
report plus the raw benchmark JSON and test output under `reports/`.

The report is deliberately local and generated: benchmark results must describe
the exact machine and commit that produced them. Do not publish a copied or
hand-edited number without retaining its matching raw artifacts.
