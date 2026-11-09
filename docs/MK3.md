# Maschine Mikro MK3 — modo nativo CAJA

Activa **MK3 nativo** en la barra superior. Desactiva el script de Bitwig
para el mismo puerto si lo usas a la vez (evita dobles LEDs).

## Modos

| Botón hardware | Modo CAJA | Pads |
|----------------|-----------|------|
| PAD MODE       | Perform   | Disparan el kit (audio) |
| PATTERN        | Pattern   | Eligen patrón A1–A16 |
| STEP           | Step/Rack | Pasos 1–16 del compás |

## Capas

| Gesto | Acción |
|-------|--------|
| SHIFT + pad (Pattern) | Patrones B1–B16 |
| LEFT / RIGHT (Pattern) | Banco A / B |
| LEFT / RIGHT (Step) | Compás anterior / siguiente |
| SELECT + pad (Step) | Elige fila (pieza) |
| Pad (Step) | Toggle golpe en el paso |
| SHIFT + pad (Step) | Toggle ghost |
| Encoder (CC 1, relativo) | BPM ± (botón TEMPO = foco BPM) |
| Botón VOLUME | Encoder controla Master |
| Botón TEMPO | Encoder controla BPM |
| SHIFT + Encoder | Master ± (siempre) |

## LEDs

- Pattern: cian = slot con contenido, blanco = activo
- Step: color por velocity del paso, blanco = playhead
- Botones de modo: brillo alto = modo activo

## Checklist hardware (Ubuntu / JACK)

1. [ ] Mikro conectado por USB, visible en `amidi -l` o `aconnect -i`
2. [ ] En standalone: elige el puerto MIDI del Mikro como entrada **y** salida
3. [ ] **MK3 nativo si** en la GUI
4. [ ] Modo **Siempre** + BPM libre si no hay transporte JACK
5. [ ] PATTERN → pads cambian de patrón; STEP → editan pasos
6. [ ] Encoder mueve el BPM en el LCD
7. [ ] LEDs de pads reaccionan (requiere MIDI OUT al Mikro)

## GUI

- **Ayuda** en la barra superior abre el mapa resumido
- Perform: clic der. = Mute, clic medio = Solo
- Pads: mesa de mezcla (faders + pan continuo)


## Por qué no reaccionaba antes

1. El **encoder** del Mikro es **CC 1** (relativo), no CC 23. CC 23 es el botón VOLUME.
2. Hay que seleccionar en el standalone el puerto MIDI del Mikro como **entrada y salida**.
3. **MK3 nativo** debe estar en **si** (y el script de Bitwig no debe pelear por el mismo puerto).

## Prueba rápida

1. `aconnect -i` / `-o` → anota el cliente del Mikro.
2. Arranca CAJA standalone, elige ese dispositivo MIDI in/out.
3. MK3 nativo **si**.
4. Pulsa **PATTERN** → el LED del botón debe cambiar; pads eligen patrón.
5. Gira el encoder → BPM en el LCD.
6. Pulsa **STEP** → pads editan pasos.
