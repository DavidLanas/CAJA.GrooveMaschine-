# CAJA — Checklist Fase 4 / 5

## Fase 4 — MIDI Controller (MK3)

- [x] Runtime de modos Perform / Pattern / Rack
- [x] CCs: PATTERN, STEP, PAD_MODE, SHIFT, SELECT, LEFT, RIGHT
- [x] Pads en Pattern → `select_pattern` + apply_snap
- [x] Pads en Rack → toggle paso / Select+pad → fila
- [x] Cola de comandos audio → GUI (sin bloquear process)
- [x] Preferencia **MK3 nativo** (off si usas el script de Bitwig)
- [x] Salida MIDI (`MIDI_OUTPUT`) para LEDs
- [x] LEDs de pads según modo (Pattern: activo/lleno; Rack: pasos + playhead)
- [x] LEDs de botones de modo (PAD_MODE / PATTERN / STEP / PLAY)
- [ ] Perfiles de otros dispositivos (templates genéricos)
- [ ] MIDI Learn de CCs arbitrarios hacia targets del plugin
- [ ] Feedback LED completo de botones de transporte (REC/STOP/…)

## Fase 5 — Integración y pulido

- [x] Multi-patrón (rack 32) en core + JSON v2
- [x] Selector de patrones en GUI + renombrar
- [x] Pantalla RACK (step sequencer por compás)
- [x] Mute / Solo / Gain / Pan por pad (ya en kit + Pads)
- [x] Indicador de modo MK3 en top bar
- [x] Binario standalone (`cargo run -p caja --features standalone`)
- [ ] Pruebas de humo en Bitwig (script ON → MK3 nativo OFF)
- [ ] Pruebas de humo standalone con Mikro vía ALSA/PipeWire
- [ ] Documentación de usuario (mapeo pads / modos)
- [ ] Release: `cargo xtask bundle caja --release`

## Cómo probar MK3

1. **Con script Bitwig**  
   - Top bar → **MK3 nativo no**  
   - El script maneja LEDs y modos; CAJA solo recibe notas/CCs sueltos.

2. **Modo nativo (standalone o sin script)**  
   - **MK3 nativo si**  
   - PATTERN → pads eligen patrón A1–A16 (Shift = banco B)  
   - STEP → pads editan 16 pasos del compás; LEFT/RIGHT cambia compás  
   - PAD_MODE → vuelve a Perform (pads = kit)

## Archivos clave

| Área | Ruta |
|------|------|
| Rack multi-patrón | `core/src/pattern_rack.rs`, `session.rs` |
| Runtime MK3 | `plugin/src/midi/mk3.rs` |
| GUI selector / RACK | `plugin/src/editor.rs` |
| Preferencia MK3 | `plugin/src/params.rs` (`mk3_enabled`) |
