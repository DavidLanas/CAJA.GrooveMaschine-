# Roadmap final — GUI MPC + Maschine MK3

Estado base (v0.2): multi-patrón, selector, pantalla Step/Rack, runtime MK3
(modos + LEDs), standalone, **BPM libre corregido**.

## Fase A — Look MPC (en curso)

- [x] Tokens de tema: chassis, face, rubber, lcd, leds
- [x] Fondo/bisel de chasis en `paint`
- [x] Pestañas renombradas: Perform / Library / Pads / Step
- [x] LCD strip (frase + status + BPM grande estilo display)
- [x] Pads como goma cuadrada con LED de esquina (Perform 4x4)
- [x] Step grid con celdas tipo pad (goma + velocity + playhead)

## Fase B — Layout por zonas

- [ ] `Zones`: top transport | LCD | pad/step matrix | knobs
- [x] Perform 4×4 + mute/solo (vol/pan en mixer Pads)
- [ ] Pantalla **Step**: fila seleccionada a tamaño pad + 16 pasos
- [ ] Pantalla **Sound**: parámetros del pad (gain, pan, seed, slot)

## Fase C — Maschine nativo completo

- [x] CCs de modo Pattern / Step / Pad Mode
- [x] LEDs de pads y botones de modo
- [x] Encoder → BPM; SHIFT+Encoder → Master
- [x] SHIFT capas (banco B, ghost, bancos LEFT/RIGHT)
- [x] Banco A/B vía LEFT/RIGHT y SHIFT+pad
- [ ] Preferencia “solo LEDs” vs “control total” (Bitwig script)

## Fase D — Pulido

- [x] Checklist en docs/MK3.md
- [ ] Bundle CLAP release
- [x] Overlay Ayuda en GUI

## Cómo probar el BPM (standalone)

1. **Modo** = `Siempre` (no “Con el transporte”)
2. Cambia **BPM libre** en la GUI
3. El patrón debe acelerar/ralentizar al momento

Si **Modo** = transporte, el tempo lo marca JACK/el host.
