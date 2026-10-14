# Virtual-cycle timer expansion card

The card exposes a reload period, start/stop control, sticky IRQ status, and a read-only package label. Timer deadlines use CPT32 virtual cycles, not wall-clock time.

| Offset | Read | Write |
|---|---|---|
| `0x00` | Period low byte | Period low byte |
| `0x01` | Period high byte | Period high byte |
| `0x02` | `1` while enabled | Bit 0 starts (`1`) or stops (`0`) |
| `0x03` | Bit 0 is pending | Writing bit 0 clears pending and deasserts IRQ |
| `0x10+` | Bytes from asset ID 0; out of range reads zero | Ignored |

Writing period zero selects a one-cycle minimum. When started, each deadline sets the pending flag, asserts the card IRQ, and schedules the next deadline. The guest clears the level by writing `1` to status. The bundled `assets/label.txt` demonstrates lazy package asset reads.

Build and package from this directory:

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
python3 ../../sdk/wasm-expansion-card/package.py \
  --asset assets/label.txt=assets/label.txt \
  target/wasm32-unknown-unknown/release/cpt32_wasm_timer_card.wasm \
  target/timer.cpt32card
```

Configure slot 0 using a path relative to `config.yaml`:

```yaml
expansion_card:
  - slot: 0
    package: "plugins/timer.cpt32card"
```

See [`doc/WASM_expansion_card_development.md`](../../doc/WASM_expansion_card_development.md) for the full SDK and host contract.
