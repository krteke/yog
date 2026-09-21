```sh
cargo run -p yog-runtime \
  --features dev-tools \
  --example emulate_chart_preview -- \
  --png /tmp/chart.png \
  --svg /tmp/chart.svg \
  --candidates 4 \
  --range 16,42 \
  --parameter cq \
  --width 1280 \
  --height 720
```
